use std::str::FromStr;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use solana_mpp::solana_keychain::transaction_util::TransactionUtil;
use solana_mpp::solana_keychain::{SignTransactionResult, SignerError, SolanaSigner};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::Transaction;

pub const DEFAULT_DCP_URL: &str = "http://127.0.0.1:8421";
const DCP_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const DCP_CONSENT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DCP_DEFAULT_CONSENT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone)]
pub struct DcpPaymentContext {
    pub protocol: String,
    pub amount: String,
    pub currency: String,
    pub recipient: Option<String>,
    pub resource: Option<String>,
    pub purpose: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DcpSigner {
    client: reqwest::Client,
    dcp_url: String,
    pubkey: Pubkey,
    agent_name: String,
    context: Option<DcpPaymentContext>,
}

#[derive(Debug, Deserialize)]
struct DcpAddressResponse {
    address: String,
}

#[derive(Debug, Deserialize)]
struct DcpSignResponse {
    signature: Option<String>,
    requires_consent: Option<bool>,
    consent_id: Option<String>,
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DcpConsentStatus {
    status: String,
}

#[derive(Debug, Serialize)]
struct DcpSignRequest<'a> {
    chain: &'static str,
    protocol: &'a str,
    payload: String,
    amount: Option<&'a str>,
    currency: Option<&'a str>,
    recipient: Option<&'a str>,
    resource: Option<&'a str>,
    purpose: Option<&'a str>,
    agent_name: &'a str,
}

impl DcpSigner {
    pub async fn connect(
        dcp_url: impl Into<String>,
        pubkey: Option<String>,
        agent_name: impl Into<String>,
    ) -> Result<Self, SignerError> {
        let dcp_url = dcp_url.into().trim_end_matches('/').to_string();
        let client = reqwest::Client::builder()
            .timeout(DCP_HTTP_TIMEOUT)
            .build()
            .map_err(|e| {
                SignerError::RemoteApiError(format!("Failed to create DCP HTTP client: {e}"))
            })?;
        let dcp_pubkey = fetch_dcp_pubkey(&client, &dcp_url).await?;
        let pubkey = if let Some(value) = pubkey {
            let configured = parse_pubkey(&value)?;
            if configured != dcp_pubkey {
                return Err(SignerError::InvalidPublicKey(format!(
                    "Configured DCP account pubkey {configured} does not match DCP wallet {dcp_pubkey}"
                )));
            }
            configured
        } else {
            dcp_pubkey
        };

        Ok(Self {
            client,
            dcp_url,
            pubkey,
            agent_name: agent_name.into(),
            context: None,
        })
    }

    pub fn with_context(mut self, context: DcpPaymentContext) -> Self {
        self.context = Some(context);
        self
    }

    async fn sign_payment_message(&self, message: &[u8]) -> Result<Signature, SignerError> {
        let fallback;
        let context = if let Some(context) = self.context.as_ref() {
            context
        } else {
            fallback = DcpPaymentContext {
                protocol: "generic".to_string(),
                amount: String::new(),
                currency: String::new(),
                recipient: None,
                resource: None,
                purpose: Some("Pay.sh signing request".to_string()),
            };
            &fallback
        };

        let request = DcpSignRequest {
            chain: "solana",
            protocol: &context.protocol,
            payload: base64::engine::general_purpose::STANDARD.encode(message),
            amount: (!context.amount.is_empty()).then_some(context.amount.as_str()),
            currency: (!context.currency.is_empty()).then_some(context.currency.as_str()),
            recipient: context.recipient.as_deref(),
            resource: context.resource.as_deref(),
            purpose: context.purpose.as_deref(),
            agent_name: &self.agent_name,
        };

        self.sign_with_retry(&request).await
    }

    async fn sign_with_retry(
        &self,
        request: &DcpSignRequest<'_>,
    ) -> Result<Signature, SignerError> {
        let mut response = self.post_sign_request(request).await?;
        if response.requires_consent == Some(true) {
            let consent_id = response.consent_id.clone().ok_or_else(|| {
                SignerError::RemoteApiError("DCP requested consent without consent_id".to_string())
            })?;
            self.wait_for_consent(&consent_id, response.expires_at.as_deref())
                .await?;
            response = self.post_sign_request(request).await?;
        }

        let signature = response.signature.ok_or_else(|| {
            SignerError::RemoteApiError("DCP response did not include signature".to_string())
        })?;
        parse_signature(&signature)
    }

    async fn post_sign_request(
        &self,
        request: &DcpSignRequest<'_>,
    ) -> Result<DcpSignResponse, SignerError> {
        let response = self
            .client
            .post(format!("{}/v1/vault/sign_payment_message", self.dcp_url))
            .json(request)
            .send()
            .await
            .map_err(|e| SignerError::RemoteApiError(format!("DCP signing request failed: {e}")))?;

        if !response.status().is_success() {
            return Err(SignerError::RemoteApiError(format!(
                "DCP signing request returned {}",
                response.status()
            )));
        }

        response.json::<DcpSignResponse>().await.map_err(|e| {
            SignerError::SerializationError(format!("DCP signing response invalid: {e}"))
        })
    }

    async fn wait_for_consent(
        &self,
        consent_id: &str,
        expires_at: Option<&str>,
    ) -> Result<(), SignerError> {
        let deadline = expires_at
            .and_then(parse_rfc3339_deadline)
            .unwrap_or_else(|| Instant::now() + DCP_DEFAULT_CONSENT_TIMEOUT);

        while Instant::now() < deadline {
            tokio::time::sleep(DCP_CONSENT_POLL_INTERVAL).await;
            let response = self
                .client
                .get(format!("{}/consent/{consent_id}/status", self.dcp_url))
                .send()
                .await;

            let Ok(response) = response else {
                continue;
            };
            if !response.status().is_success() {
                continue;
            }

            let status = response.json::<DcpConsentStatus>().await.map_err(|e| {
                SignerError::SerializationError(format!("DCP consent response invalid: {e}"))
            })?;
            match status.status.as_str() {
                "approved" => return Ok(()),
                "denied" => {
                    return Err(SignerError::SigningFailed("DCP consent denied".to_string()));
                }
                "expired" | "not_found" => {
                    return Err(SignerError::SigningFailed(format!(
                        "DCP consent {}",
                        status.status
                    )));
                }
                _ => {}
            }
        }

        Err(SignerError::SigningFailed(
            "DCP consent timed out".to_string(),
        ))
    }
}

#[async_trait::async_trait]
impl SolanaSigner for DcpSigner {
    fn pubkey(&self) -> Pubkey {
        self.pubkey
    }

    async fn sign_transaction(
        &self,
        tx: &mut Transaction,
    ) -> Result<SignTransactionResult, SignerError> {
        let signature = self.sign_payment_message(&tx.message_data()).await?;
        TransactionUtil::add_signature_to_transaction(tx, &self.pubkey(), signature)?;
        let signed_transaction = (TransactionUtil::serialize_transaction(tx)?, signature);
        Ok(TransactionUtil::classify_signed_transaction(
            tx,
            signed_transaction,
        ))
    }

    async fn sign_message(&self, message: &[u8]) -> Result<Signature, SignerError> {
        self.sign_payment_message(message).await
    }

    async fn is_available(&self) -> bool {
        self.client
            .get(format!("{}/health", self.dcp_url))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }
}

async fn fetch_dcp_pubkey(client: &reqwest::Client, dcp_url: &str) -> Result<Pubkey, SignerError> {
    let response = client
        .get(format!("{dcp_url}/address/solana"))
        .send()
        .await
        .map_err(|e| SignerError::RemoteApiError(format!("DCP address request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(SignerError::RemoteApiError(format!(
            "DCP address request returned {}",
            response.status()
        )));
    }
    let address = response.json::<DcpAddressResponse>().await.map_err(|e| {
        SignerError::SerializationError(format!("DCP address response invalid: {e}"))
    })?;
    parse_pubkey(&address.address)
}

fn parse_pubkey(value: &str) -> Result<Pubkey, SignerError> {
    Pubkey::from_str(value)
        .map_err(|e| SignerError::InvalidPublicKey(format!("Invalid DCP Solana address: {e}")))
}

fn parse_signature(value: &str) -> Result<Signature, SignerError> {
    Signature::from_str(value)
        .map_err(|e| SignerError::SigningFailed(format!("Invalid DCP signature: {e}")))
}

fn parse_rfc3339_deadline(value: &str) -> Option<Instant> {
    let expires = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    let now = chrono::Utc::now();
    let remaining = expires
        .with_timezone(&chrono::Utc)
        .signed_duration_since(now);
    remaining
        .to_std()
        .ok()
        .map(|duration| Instant::now() + duration)
}
