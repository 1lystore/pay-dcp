# Pay.sh + DCP Local Signer

This fork adds an optional DCP signer backend to Pay.sh. Pay.sh handles paid HTTP requests; DCP Desktop handles wallet approval, budget policy, and signing.

With this setup, users and terminal-based agents can call any Pay.sh-supported MPP or x402 endpoint without receiving wallet keys.

## How It Works

```text
User or agent
  -> pay-dcp <paid-endpoint>
  -> Pay.sh detects the payment challenge
  -> Pay.sh asks DCP Desktop to sign the exact payment message
  -> DCP checks policy and asks for approval when required
  -> user approves in DCP Desktop or Telegram
  -> Pay.sh retries with payment proof
  -> the endpoint returns the paid response
```

## 1. Install DCP Desktop

Install DCP Desktop, create or unlock your vault, and make sure a Solana wallet is available.

Download:

```text
https://dcpagent.com/
```

Source and setup docs:

```text
https://github.com/1lystore/dcp
```

After opening DCP Desktop, verify it is running:

```bash
curl -sS http://127.0.0.1:8421/health
```

Verify the wallet address is available:

```bash
curl -sS http://127.0.0.1:8421/address/solana
```

The signer URL used by this quickstart is:

```text
http://127.0.0.1:8421
```

Do not expose this endpoint publicly.

## 2. Configure DCP Approval

In DCP Desktop, set your budget and approval rules.

For a visible approval request, set the approval threshold below the endpoint price. For example, if an endpoint charges `0.01 USDC`, set the USDC approval threshold below `0.01`.

Optional check:

```bash
curl -sS "http://127.0.0.1:8421/budget/check?amount=0.01&currency=USDC&chain=solana"
```

If approval is required, the response includes:

```json
{
  "requires_approval": true
}
```

## 3. Clone This Pay.sh Fork

```bash
git clone https://github.com/1lystore/pay-dcp.git
cd pay-dcp
```

## 4. Install The Local Helper

```bash
scripts/install-local-command.sh
```

If your shell cannot find `pay-dcp` after install, run:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 5. Connect Pay.sh To DCP Once

```bash
pay-dcp-setup
```

This creates a Pay.sh account named `dcp`. The account uses the Solana wallet address from DCP Desktop. Pay.sh does not store or receive the private key.

Raw command equivalent:

```bash
cd rust
PAY_DCP_URL=http://127.0.0.1:8421 cargo run -p pay -- account new dcp --backend dcp --force
cd ..
```

## 6. Call Any Paid Endpoint

For a normal paid endpoint, use:

```bash
pay-dcp <paid-endpoint-url>
```

For the hosted Pay.sh debugger, use sandbox mode:

```bash
pay-dcp sandbox https://debugger.pay.sh/mpp/quote/AAPL
```

Raw command equivalent:

```bash
cd rust
PAY_DCP_URL=http://127.0.0.1:8421 cargo run -p pay -- --account dcp curl <paid-endpoint-url>
cd ..
```

Raw debugger command equivalent:

```bash
cd rust
PAY_DCP_URL=http://127.0.0.1:8421 cargo run -p pay -- --sandbox --account dcp curl https://debugger.pay.sh/mpp/quote/AAPL
cd ..
```

Expected flow:

```text
Pay.sh detects the payment challenge.
DCP Desktop or Telegram shows an approval request.
User approves.
Pay.sh retries with payment proof.
The endpoint returns the paid response.
```

## 7. Use It From A Terminal Agent

Use a terminal-capable agent such as Claude Code, Cursor terminal, OpenClaw terminal, Codex, or a regular shell.

Ask the agent to run:

```bash
pay-dcp <paid-endpoint-url>
```

Example prompt:

```text
Call this paid API through Pay.sh:
pay-dcp sandbox https://debugger.pay.sh/mpp/quote/AAPL
```

## Support

This quickstart supports:

```text
Pay.sh fork -> local DCP Desktop on 127.0.0.1:8421
any Pay.sh-supported MPP/x402 endpoint
terminal users and terminal-capable agents
```

Remote agents and VPS signers should use a private DCP agent endpoint, not a public signer endpoint.
