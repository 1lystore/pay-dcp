#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUST_DIR="$ROOT_DIR/rust"
DCP_URL="${PAY_DCP_URL:-${DCP_URL:-http://127.0.0.1:8421}}"
ACCOUNT_NAME="${PAY_DCP_ACCOUNT:-dcp}"

usage() {
  cat <<EOF
Usage:
  scripts/pay-dcp.sh setup
  scripts/pay-dcp.sh sandbox <curl-args...>
  scripts/pay-dcp.sh curl <curl-args...>
  scripts/pay-dcp.sh <url>

Environment:
  PAY_DCP_URL        DCP Desktop URL (default: http://127.0.0.1:8421)
  PAY_DCP_ACCOUNT    Pay.sh account name (default: dcp)

Examples:
  scripts/pay-dcp.sh setup
  scripts/pay-dcp.sh sandbox https://debugger.pay.sh/mpp/quote/AAPL
  scripts/pay-dcp.sh https://example.com/paid-api
  scripts/pay-dcp.sh curl https://example.com/paid-api
EOF
}

require_dcp() {
  if ! curl -fsS "$DCP_URL/health" >/dev/null; then
    cat >&2 <<EOF
DCP Desktop is not reachable at $DCP_URL.

Open and unlock the DCP Desktop app, then try again.
EOF
    exit 1
  fi

  if ! curl -fsS "$DCP_URL/address/solana" >/dev/null; then
    cat >&2 <<EOF
DCP Desktop is reachable, but the Solana wallet address is not available.

Unlock DCP Desktop and make sure a Solana wallet exists.
EOF
    exit 1
  fi
}

setup_account() {
  require_dcp
  cd "$RUST_DIR"
  PAY_DCP_URL="$DCP_URL" cargo run -p pay -- account new "$ACCOUNT_NAME" --backend dcp --force
}

pay_curl() {
  local sandbox="${1:-false}"
  shift
  if [ "$#" -eq 0 ]; then
    echo "Missing curl arguments. Example: scripts/pay-dcp.sh curl https://example.com/paid-api" >&2
    exit 2
  fi

  require_dcp
  cd "$RUST_DIR"
  local pay_args=(--account "$ACCOUNT_NAME")
  if [ "$sandbox" = "true" ]; then
    pay_args=(--sandbox "${pay_args[@]}")
  fi
  PAY_DCP_URL="$DCP_URL" cargo run -p pay -- "${pay_args[@]}" curl "$@"
}

cmd="${1:-}"
case "$cmd" in
  setup)
    setup_account
    ;;
  sandbox)
    shift
    pay_curl true "$@"
    ;;
  curl)
    shift
    pay_curl false "$@"
    ;;
  -h|--help|help|"")
    usage
    ;;
  *)
    if [[ "$cmd" == http://* || "$cmd" == https://* ]]; then
      shift
      pay_curl false "$cmd" "$@"
    else
      echo "Unknown command: $cmd" >&2
      usage >&2
      exit 2
    fi
    ;;
esac
