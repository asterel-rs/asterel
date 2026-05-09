#!/bin/sh
set -eu

repo="${ASTEREL_REPO:-asterel-rs/asterel}"
url="${ASTEREL_INSTALLER_URL:-https://raw.githubusercontent.com/${repo}/main/scripts/release/install.sh}"

if ! command -v bash >/dev/null 2>&1; then
  printf '%s\n' "error: bash is required to run the Asterel installer" >&2
  exit 1
fi

if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" | bash -s -- "$@"
elif command -v wget >/dev/null 2>&1; then
  wget -qO- "$url" | bash -s -- "$@"
else
  printf '%s\n' "error: curl or wget is required to download the Asterel installer" >&2
  exit 1
fi
