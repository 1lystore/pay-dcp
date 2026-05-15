//! `pay account new` — generate a fresh keypair and store it.

use std::time::Duration;

use dialoguer::Select;
use owo_colors::OwoColorize;
use pay_core::accounts::{AccountsFile, MAINNET_NETWORK};
use pay_core::dcp_signer::DEFAULT_DCP_URL;
use pay_core::keystore::Keystore;
use serde::Deserialize;
use solana_pubkey::Pubkey;

/// Generate a new keypair and store it securely.
#[derive(clap::Args)]
pub struct NewCommand {
    /// Account name (required).
    pub name: String,

    /// Storage backend: "keychain" (macOS), "gnome-keyring" (Linux),
    /// "windows-hello" (Windows), or "dcp".
    #[arg(long)]
    pub backend: Option<String>,

    /// Legacy vault name.
    #[arg(long, hide = true)]
    pub vault: Option<String>,

    /// Replace existing account.
    #[arg(long)]
    pub force: bool,
}

impl NewCommand {
    pub fn run(self) -> pay_core::Result<()> {
        let (pubkey, backend_name) = create_account(
            &self.name,
            self.backend.as_deref(),
            self.vault.as_deref(),
            self.force,
        )?;
        eprintln!();

        let config = pay_core::Config::load().unwrap_or_default();
        let rpc_url = config
            .rpc_url
            .clone()
            .unwrap_or_else(pay_core::balance::mainnet_rpc_url);
        let completion = crate::tui::run_topup_flow(&pubkey, &rpc_url, &self.name)?;
        print_next_steps(
            &self.name,
            backend_name,
            completion.as_ref().map(|c| &c.received),
        );
        Ok(())
    }
}

/// Core account creation logic. Returns the base58 pubkey on success.
/// Shared by `pay account new` and `pay setup`.
/// Returns `(pubkey_b58, backend_display_name)`.
pub fn create_account(
    name: &str,
    backend: Option<&str>,
    vault: Option<&str>,
    force: bool,
) -> pay_core::Result<(String, &'static str)> {
    let backend_id = match backend {
        Some(b) => b.to_string(),
        None => pick_backend()?,
    };

    if backend_id == "dcp" {
        return create_dcp_account(name, force);
    }

    let (ks, keystore_kind, backend_display, op_info) = build_keystore(&backend_id, vault)?;

    if ks.exists(name) && !force {
        let pubkey = ks
            .pubkey(name)
            .map_err(|e| pay_core::Error::Config(format!("{e}")))?;
        let pubkey_b58 = bs58::encode(&pubkey).into_string();
        eprintln!();
        crate::components::print_notice(
            crate::components::NoticeLevel::Info,
            "Account already exists",
            &format!(
                "`{name}` is already stored in {backend_display}.\nUse --force to replace it."
            ),
        );

        // Ensure the account is registered in accounts.yml even if the
        // keypair already exists in the keystore (e.g. after a reset).
        save_account(
            name,
            keystore_kind,
            &pubkey_b58,
            op_info.as_ref().and_then(|i| i.vault.clone()),
            None,
            None,
            op_info.as_ref().and_then(|i| i.account.clone()),
        )?;

        return Ok((pubkey_b58, backend_display));
    }

    let (keypair_bytes, pubkey_b58) = generate_keypair();

    let sync = if backend_id == "1password" {
        pay_core::keystore::SyncMode::CloudSync
    } else {
        pay_core::keystore::SyncMode::ThisDeviceOnly
    };

    let intent = pay_core::keystore::AuthIntent::create_account(name);
    ks.import_with_intent(name, &keypair_bytes, sync, &intent)
        .map_err(|e| pay_core::Error::Config(format!("{e}")))?;

    save_account(
        name,
        keystore_kind,
        &pubkey_b58,
        op_info
            .as_ref()
            .and_then(|i| i.vault.clone())
            .or(vault.map(|v| v.to_string())),
        None,
        None,
        op_info.as_ref().and_then(|i| i.account.clone()),
    )?;

    Ok((pubkey_b58, backend_display))
}

/// Resolved 1Password account info for storing in accounts.yml.
pub struct OpAccountInfo {
    pub vault: Option<String>,
    pub account: Option<String>,
}

fn build_keystore(
    backend_id: &str,
    vault: Option<&str>,
) -> pay_core::Result<(
    Keystore,
    pay_core::accounts::Keystore,
    &'static str,
    Option<OpAccountInfo>,
)> {
    match backend_id {
        #[cfg(target_os = "macos")]
        "keychain" => Ok((
            Keystore::apple_keychain(),
            pay_core::accounts::Keystore::AppleKeychain,
            "Apple Keychain",
            None,
        )),
        #[cfg(not(target_os = "macos"))]
        "keychain" => Err(pay_core::Error::Config(
            "Keychain is only available on macOS".to_string(),
        )),

        #[cfg(target_os = "linux")]
        "gnome-keyring" => {
            if !Keystore::gnome_keyring_available() {
                return Err(pay_core::Error::Config(
                    "GNOME Keyring is not available.".to_string(),
                ));
            }
            crate::commands::setup::install_linux_polkit_policy_if_needed()?;
            Ok((
                Keystore::gnome_keyring(),
                pay_core::accounts::Keystore::GnomeKeyring,
                "GNOME Keyring",
                None,
            ))
        }
        #[cfg(not(target_os = "linux"))]
        "gnome-keyring" => Err(pay_core::Error::Config(
            "GNOME Keyring is only available on Linux".to_string(),
        )),

        #[cfg(target_os = "windows")]
        "windows-hello" => {
            if !Keystore::windows_hello_available() {
                return Err(pay_core::Error::Config(
                    "Windows Hello is not configured.".to_string(),
                ));
            }
            Ok((
                Keystore::windows_hello(),
                pay_core::accounts::Keystore::WindowsHello,
                "Windows Hello",
                None,
            ))
        }
        #[cfg(not(target_os = "windows"))]
        "windows-hello" => Err(pay_core::Error::Config(
            "Windows Hello is only available on Windows".to_string(),
        )),

        "1password" => {
            if !Keystore::onepassword_available() {
                return Err(pay_core::Error::Config(
                    "1Password CLI (`op`) is not installed or not signed in.".to_string(),
                ));
            }
            let op_account = resolve_op_account()?;
            let ks = match vault {
                Some(v) => Keystore::onepassword_with_vault(v, op_account.clone()),
                None => Keystore::onepassword(op_account.clone()),
            };
            Ok((
                ks,
                pay_core::accounts::Keystore::OnePassword,
                "1Password",
                Some(OpAccountInfo {
                    vault: vault.map(|v| v.to_string()),
                    account: op_account,
                }),
            ))
        }

        other => Err(pay_core::Error::Config(format!(
            "Unknown backend: {other}. Use {}.",
            available_backends_hint()
        ))),
    }
}

fn create_dcp_account(name: &str, force: bool) -> pay_core::Result<(String, &'static str)> {
    let network = account_network();
    let accounts = AccountsFile::load()?;
    if accounts.named_account_for_network(&network, name).is_some() && !force {
        crate::components::print_notice(
            crate::components::NoticeLevel::Info,
            "Account already exists",
            &format!("`{name}` is already registered for `{network}`.\nUse --force to replace it."),
        );
        let account = accounts
            .named_account_for_network(&network, name)
            .expect("checked account existence");
        let pubkey = account.pubkey.clone().ok_or_else(|| {
            pay_core::Error::Config(format!(
                "Account `{name}` is registered for `{network}` but has no public key"
            ))
        })?;
        return Ok((pubkey, "DCP"));
    }

    let dcp_url = configured_dcp_url();
    let pubkey_b58 = fetch_dcp_pubkey(&dcp_url)?;
    save_dcp_account(name, &pubkey_b58, &dcp_url)?;
    Ok((pubkey_b58, "DCP"))
}

fn save_dcp_account(name: &str, pubkey: &str, dcp_url: &str) -> pay_core::Result<()> {
    let mut accounts = pay_core::accounts::AccountsFile::load()?;
    for network in [MAINNET_NETWORK, "localnet", "devnet"] {
        accounts.upsert(
            network,
            name,
            pay_core::accounts::Account {
                keystore: pay_core::accounts::Keystore::Dcp,
                active: false,
                auth_required: Some(false),
                pubkey: Some(pubkey.to_string()),
                vault: None,
                account: None,
                path: None,
                dcp_url: Some(dcp_url.to_string()),
                secret_key_b58: None,
                created_at: None,
            },
        );
    }
    accounts.save()
}

fn configured_dcp_url() -> String {
    std::env::var("DCP_URL")
        .or_else(|_| std::env::var("PAY_DCP_URL"))
        .unwrap_or_else(|_| DEFAULT_DCP_URL.to_string())
        .trim_end_matches('/')
        .to_string()
}

#[derive(Deserialize)]
struct DcpAddressResponse {
    address: String,
}

fn fetch_dcp_pubkey(dcp_url: &str) -> pay_core::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| pay_core::Error::Config(format!("Failed to create HTTP client: {e}")))?;

    let health = client
        .get(format!("{dcp_url}/health"))
        .send()
        .map_err(|e| pay_core::Error::Config(format!("DCP is not reachable at {dcp_url}: {e}")))?;
    if !health.status().is_success() {
        return Err(pay_core::Error::Config(format!(
            "DCP health check failed at {dcp_url}: {}",
            health.status()
        )));
    }

    let address = client
        .get(format!("{dcp_url}/address/solana"))
        .send()
        .map_err(|e| pay_core::Error::Config(format!("DCP address lookup failed: {e}")))?;
    if !address.status().is_success() {
        return Err(pay_core::Error::Config(format!(
            "DCP address lookup returned {}. Open and unlock DCP first.",
            address.status()
        )));
    }
    let body: DcpAddressResponse = address
        .json()
        .map_err(|e| pay_core::Error::Config(format!("DCP address response invalid: {e}")))?;
    body.address.parse::<Pubkey>().map_err(|e| {
        pay_core::Error::Config(format!("DCP returned an invalid Solana address: {e}"))
    })?;
    Ok(body.address)
}

/// Comma-separated list of backends that work on the current OS.
/// Used in error messages so we don't suggest `keychain` to a Linux user.
fn available_backends_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "'keychain' or 'dcp'"
    } else if cfg!(target_os = "linux") {
        "'gnome-keyring' or 'dcp'"
    } else if cfg!(target_os = "windows") {
        "'windows-hello' or 'dcp'"
    } else {
        "'dcp' or a supported platform backend"
    }
}

/// Resolve which 1Password account to use. If only one account is
/// configured, use it automatically. If multiple, prompt the user.
pub fn resolve_op_account() -> pay_core::Result<Option<String>> {
    let output = std::process::Command::new("op")
        .args(["account", "list", "--format=json"])
        .output()
        .map_err(|e| pay_core::Error::Config(format!("op account list: {e}")))?;

    if !output.status.success() {
        return Ok(None);
    }

    #[derive(serde::Deserialize)]
    struct OpAccount {
        account_uuid: String,
        email: String,
        url: String,
    }

    let accounts: Vec<OpAccount> = serde_json::from_slice(&output.stdout).unwrap_or_default();

    match accounts.len() {
        0 => Ok(None),
        1 => Ok(Some(accounts[0].account_uuid.clone())),
        _ => {
            let labels: Vec<String> = accounts
                .iter()
                .map(|a| format!("{} ({})", a.email, a.url))
                .collect();

            let selection =
                dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
                    .with_prompt("Which 1Password account?")
                    .items(&labels)
                    .default(0)
                    .interact()
                    .map_err(|e| pay_core::Error::Config(format!("Prompt error: {e}")))?;

            Ok(Some(accounts[selection].account_uuid.clone()))
        }
    }
}

/// Interactive backend picker. Returns the backend id string.
pub fn pick_backend() -> pay_core::Result<String> {
    let has_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    if !has_tty {
        return Err(pay_core::Error::Config(format!(
            "No --backend specified and no interactive terminal available.\n  \
             Pass --backend=<one of {}>.",
            available_backends_hint()
        )));
    }

    struct Opt {
        id: &'static str,
        label: String,
    }

    // Only show platform-native backend on the current OS
    #[cfg(target_os = "macos")]
    let options = [Opt {
        id: "keychain",
        label: "macOS Keychain (requires Touch ID)".into(),
    }];

    #[cfg(target_os = "linux")]
    let options = {
        if Keystore::gnome_keyring_available() {
            vec![Opt {
                id: "gnome-keyring",
                label: "GNOME Keyring (password prompt)".into(),
            }]
        } else {
            Vec::new()
        }
    };

    #[cfg(target_os = "windows")]
    let options = {
        if Keystore::windows_hello_available() {
            vec![Opt {
                id: "windows-hello",
                label: "Windows Hello (fingerprint / face / PIN)".into(),
            }]
        } else {
            Vec::new()
        }
    };

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    let options: Vec<Opt> = Vec::new();

    if options.is_empty() {
        return Err(pay_core::Error::Config(
            "No supported keystore backend is available on this system.".to_string(),
        ));
    }

    let items: Vec<String> = options.iter().map(|o| o.label.clone()).collect();

    eprintln!();
    let selection = Select::new()
        .with_prompt("Where should pay store your account?")
        .items(&items)
        .default(0)
        .interact()
        .map_err(|e| pay_core::Error::Config(format!("Selection cancelled: {e}")))?;

    Ok(options[selection].id.to_string())
}

pub fn save_account(
    name: &str,
    keystore: pay_core::accounts::Keystore,
    pubkey: &str,
    vault: Option<String>,
    path: Option<String>,
    dcp_url: Option<String>,
    account: Option<String>,
) -> pay_core::Result<()> {
    let mut accounts = pay_core::accounts::AccountsFile::load()?;
    let network = account_network();
    accounts.upsert(
        &network,
        name,
        pay_core::accounts::Account {
            keystore,
            active: false,
            auth_required: Some(true),
            pubkey: Some(pubkey.to_string()),
            vault,
            account,
            path,
            dcp_url,
            secret_key_b58: None,
            created_at: None,
        },
    );
    accounts.save()
}

fn account_network() -> String {
    std::env::var("PAY_NETWORK_ENFORCED")
        .ok()
        .filter(|network| !network.trim().is_empty())
        .unwrap_or_else(|| MAINNET_NETWORK.to_string())
}

/// Print the post-setup summary and next-step hints.
///
/// Shows `✔` confirmation lines for keystore and (if funded) the received
/// amount. Skips the topup hint when the user already funded during setup.
pub fn print_next_steps(
    name: &str,
    backend_name: &str,
    received: Option<&pay_core::client::balance::ReceivedFunds>,
) {
    eprintln!();
    eprintln!(
        "  {} Account secured in {}",
        "✔".green(),
        backend_name.green()
    );

    if let Some(r) = received {
        let amount = format_received(r);
        if !amount.is_empty() {
            eprintln!("  {} Account funded with {}", "✔".green(), amount.green());
        }
        eprintln!();
        eprintln!(
            "  {}",
            "Ready to go. Time to make HTTP pay for itself.".dimmed()
        );
        eprintln!();
        eprintln!("  {}", "$ pay claude".bold());
        eprintln!("  {}", "$ pay codex".bold());
    } else {
        eprintln!();
        crate::components::print_notice(
            crate::components::NoticeLevel::Warning,
            "Top-up required",
            &topup_required_body(name),
        );
    }

    eprintln!();
}

fn topup_required_body(name: &str) -> String {
    format!(
        "A top-up is required before making paid requests.\n$ {}",
        crate::commands::topup::topup_retry_command(name)
    )
}

pub fn format_received(r: &pay_core::client::balance::ReceivedFunds) -> String {
    if let Some(usdc) = r.tokens.iter().find(|t| t.symbol == Some("USDC")) {
        return format!("${:.2}", usdc.ui_amount);
    }
    if let Some(token) = r.tokens.first() {
        let sym = token.symbol.unwrap_or("tokens");
        return format!("{:.2} {sym}", token.ui_amount);
    }
    if r.sol_lamports > 0 {
        return format!("{:.4} SOL", r.sol_lamports as f64 / 1_000_000_000.0);
    }
    String::new()
}

pub fn generate_keypair() -> (Vec<u8>, String) {
    let signing_key = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
    let verifying_key = signing_key.verifying_key();

    let mut keypair_bytes = Vec::with_capacity(64);
    keypair_bytes.extend_from_slice(&signing_key.to_bytes());
    keypair_bytes.extend_from_slice(&verifying_key.to_bytes());

    let pubkey_b58 = bs58::encode(&verifying_key.to_bytes()).into_string();
    (keypair_bytes, pubkey_b58)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topup_required_body_uses_default_topup_command_for_default_account() {
        assert_eq!(
            topup_required_body("default"),
            "A top-up is required before making paid requests.\n$ pay topup"
        );
    }

    #[test]
    fn topup_required_body_uses_named_account_topup_command() {
        assert_eq!(
            topup_required_body("test-2"),
            "A top-up is required before making paid requests.\n$ pay topup --account test-2"
        );
    }
}
