#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"

mkdir -p "$BIN_DIR"

cat > "$BIN_DIR/pay-dcp" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$ROOT_DIR/scripts/pay-dcp.sh" "\$@"
EOF

cat > "$BIN_DIR/pay-dcp-setup" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$ROOT_DIR/scripts/pay-dcp.sh" setup
EOF

chmod +x "$BIN_DIR/pay-dcp" "$BIN_DIR/pay-dcp-setup"

echo "Installed:"
echo "  $BIN_DIR/pay-dcp"
echo "  $BIN_DIR/pay-dcp-setup"
echo
echo "If your shell cannot find them, add this to your shell profile:"
echo "  export PATH=\"\$HOME/.local/bin:\$PATH\""
