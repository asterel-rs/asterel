#!/usr/bin/env bash
set -euo pipefail

SHA=$(git rev-parse --short HEAD)
RELEASE_DIR="$HOME/.local/share/asterel/releases/$SHA"
SYMLINK="$HOME/.local/bin/asterel-current"

echo "Building release..."
cargo build --release

echo "Deploying $SHA..."
mkdir -p "$RELEASE_DIR"
cp target/release/asterel "$RELEASE_DIR/asterel"
ln -sf "$RELEASE_DIR/asterel" "$SYMLINK"

echo "Restarting daemon..."
systemctl --user daemon-reload
systemctl --user restart asterel

sleep 3
if systemctl --user is-active asterel >/dev/null 2>&1; then
    echo "Deployed $SHA successfully."
    curl -sf http://127.0.0.1:3000/health | python3 -c "import sys,json; d=json.load(sys.stdin); print(f'  status={d[\"status\"]} paired={d[\"paired\"]}')" 2>/dev/null || true
else
    echo "ERROR: daemon failed to start"
    journalctl --user -u asterel -n 10 --no-pager
    exit 1
fi
