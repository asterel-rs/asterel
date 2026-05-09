#!/usr/bin/env bash
set -euo pipefail

# Asterel Installer (Bootstrap v2)
# Usage: curl -fsSL https://raw.githubusercontent.com/asterel-rs/asterel/main/scripts/release/install.sh | bash

REPO="${ASTEREL_REPO:-asterel-rs/asterel}"
BINARY_NAME="${ASTEREL_BINARY_NAME:-asterel}"
INSTALL_DIR="${ASTEREL_INSTALL_DIR:-/usr/local/bin}"
INSTALL_METHOD="${ASTEREL_INSTALL_METHOD:-auto}" # auto | prebuilt | source
VERSION="${ASTEREL_VERSION:-}"

GUIDED=false
ASSUME_YES=false
RUN_ONBOARD=false
DRY_RUN=false

HOST_OS=""
PLATFORM=""
PREBUILT_SUPPORTED=true
INSTALL_PATH=""
INSTALLED_METHOD=""

# ── Color output ──
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

info()  { printf "${CYAN}info${NC}  %s\n" "$1"; }
ok()    { printf "${GREEN}  ✓${NC}  %s\n" "$1"; }
warn()  { printf "${YELLOW}warn${NC}  %s\n" "$1"; }
error() { printf "${RED}error${NC} %s\n" "$1" >&2; }
fail()  { error "$1"; exit 1; }

usage() {
    cat <<'USAGE'
Asterel installer (bootstrap v2)

Options:
  --guided               Interactive guided flow
  --yes, -y              Non-interactive yes to prompts
  --method <name>        Install method: auto | prebuilt | source
  --install-dir <path>   Install directory (default: /usr/local/bin)
  --version <tag>        Release tag or source ref (default: latest release)
  --repo <owner/name>    GitHub repository (default: asterel-rs/asterel)
  --run-onboard          Run `asterel onboard` after install
  --dry-run              Print selected flow without downloading/building
  --help, -h             Show this help

Env overrides:
  ASTEREL_INSTALL_DIR
  ASTEREL_INSTALL_METHOD
  ASTEREL_VERSION
  ASTEREL_REPO
USAGE
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1 || fail "Required command not found: $1"
}

confirm() {
    local prompt="$1"
    if [[ "$ASSUME_YES" == "true" ]]; then
        return 0
    fi
    if [[ ! -t 0 ]]; then
        return 1
    fi

    local reply
    read -r -p "${prompt} [y/N]: " reply
    case "${reply:-}" in
        y|Y|yes|YES) return 0 ;;
        *) return 1 ;;
    esac
}

detect_platform() {
    local raw_os raw_arch os arch

    raw_os="$(uname -s)"
    raw_arch="$(uname -m)"
    HOST_OS="$raw_os"

    case "$raw_os" in
        Linux)  os="unknown-linux-gnu" ;;
        Darwin) os="apple-darwin" ;;
        *)      fail "Unsupported OS: $raw_os (Linux and macOS only)" ;;
    esac

    case "$raw_arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)
            arch="$raw_arch"
            PREBUILT_SUPPORTED=false
            ;;
    esac

    # Current release matrix does not include aarch64-linux binaries.
    if [[ "$os" == "unknown-linux-gnu" && "$arch" == "aarch64" ]]; then
        PREBUILT_SUPPORTED=false
    fi

    PLATFORM="${arch}-${os}"
    ok "Platform: ${PLATFORM}"

    if [[ "$PREBUILT_SUPPORTED" != "true" ]]; then
        warn "Prebuilt archive may be unavailable for ${PLATFORM}; source fallback will be used"
    fi
}

get_latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    local version

    version="$(curl -fsSL "$url" | grep '"tag_name"' | head -1 | sed -E 's/.*"tag_name":\s*"([^"]+)".*/\1/')"
    if [[ -z "$version" ]]; then
        fail "Could not determine latest version from GitHub"
    fi

    echo "$version"
}

resolve_version() {
    if [[ -n "$VERSION" ]]; then
        ok "Version: ${VERSION}"
        return
    fi

    require_cmd curl
    info "Fetching latest release..."
    VERSION="$(get_latest_version)"
    ok "Version: ${VERSION}"
}

print_source_dependency_help() {
    cat <<'HELP'
Missing dependencies for source install.
Install required tooling and retry with `--method source`:
HELP

    case "$HOST_OS" in
        Darwin)
            cat <<'HELP'
  - Install Homebrew if missing: https://brew.sh/
  - Then run: brew install git rustup-init
  - Then run: rustup-init -y
HELP
            ;;
        Linux)
            cat <<'HELP'
  - Debian/Ubuntu example:
      sudo apt-get update
      sudo apt-get install -y git build-essential curl pkg-config libssl-dev
      curl https://sh.rustup.rs -sSf | sh -s -- -y
HELP
            ;;
        *)
            cat <<'HELP'
  - Install `git` and Rust toolchain (`cargo`) for your OS.
HELP
            ;;
    esac
}

maybe_install_rustup() {
    if command -v cargo >/dev/null 2>&1; then
        return 0
    fi

    if ! command -v curl >/dev/null 2>&1; then
        return 1
    fi

    warn "Rust toolchain (cargo) is missing"
    if [[ "$ASSUME_YES" != "true" ]] && ! confirm "Install rustup now?"; then
        return 1
    fi

    if [[ "$DRY_RUN" == "true" ]]; then
        ok "Dry-run: would install Rust via rustup"
        return 0
    fi

    info "Installing Rust toolchain with rustup..."
    curl https://sh.rustup.rs -sSf | sh -s -- -y >/dev/null

    if [[ -f "${HOME}/.cargo/env" ]]; then
        # shellcheck disable=SC1090
        source "${HOME}/.cargo/env"
    fi

    command -v cargo >/dev/null 2>&1
}

ensure_source_dependencies() {
    local missing=()

    if ! command -v git >/dev/null 2>&1; then
        missing+=("git")
    fi

    if ! command -v cargo >/dev/null 2>&1; then
        if ! maybe_install_rustup; then
            missing+=("cargo")
        fi
    fi

    if (( ${#missing[@]} > 0 )); then
        warn "Missing source dependencies: ${missing[*]}"
        print_source_dependency_help
        return 1
    fi

    return 0
}

ensure_install_dir() {
    local parent
    parent="$(dirname "$INSTALL_DIR")"

    if [[ -d "$INSTALL_DIR" ]]; then
        return 0
    fi

    if [[ -w "$parent" ]]; then
        mkdir -p "$INSTALL_DIR"
    else
        warn "Need sudo to create ${INSTALL_DIR}"
        sudo mkdir -p "$INSTALL_DIR"
    fi
}

install_binary() {
    local source_path="$1"
    local dest_path="${INSTALL_DIR}/${BINARY_NAME}"

    INSTALL_PATH="$dest_path"

    if [[ "$DRY_RUN" == "true" ]]; then
        ok "Dry-run: would install ${BINARY_NAME} to ${dest_path}"
        return 0
    fi

    ensure_install_dir

    info "Installing to ${dest_path}..."
    if [[ -w "$INSTALL_DIR" ]]; then
        cp "$source_path" "$dest_path"
        chmod +x "$dest_path"
    else
        warn "Need sudo to write to ${INSTALL_DIR}"
        sudo cp "$source_path" "$dest_path"
        sudo chmod +x "$dest_path"
    fi

    ok "Installed: ${dest_path}"
}

install_prebuilt() {
    local archive_name url tmpdir

    if [[ "$PREBUILT_SUPPORTED" != "true" ]]; then
        warn "Prebuilt install not supported on ${PLATFORM}"
        return 1
    fi

    if ! command -v curl >/dev/null 2>&1 || ! command -v tar >/dev/null 2>&1; then
        warn "Missing tools for prebuilt install (need: curl, tar)"
        return 1
    fi

    archive_name="${BINARY_NAME}-${PLATFORM}.tar.gz"
    url="https://github.com/${REPO}/releases/download/${VERSION}/${archive_name}"

    if [[ "$DRY_RUN" == "true" ]]; then
        info "Dry-run: would download ${url}"
        install_binary "/dev/null"
        INSTALLED_METHOD="prebuilt"
        return 0
    fi

    info "Downloading ${archive_name}..."
    tmpdir="$(mktemp -d)"

    if ! curl -fsSL "$url" -o "${tmpdir}/${archive_name}"; then
        warn "Prebuilt download failed: ${url}"
        rm -rf "$tmpdir"
        return 1
    fi

    info "Extracting archive..."
    if ! tar xzf "${tmpdir}/${archive_name}" -C "$tmpdir"; then
        warn "Failed to extract ${archive_name}"
        rm -rf "$tmpdir"
        return 1
    fi

    if [[ ! -f "${tmpdir}/${BINARY_NAME}" ]]; then
        warn "Binary '${BINARY_NAME}' not found in archive"
        rm -rf "$tmpdir"
        return 1
    fi

    install_binary "${tmpdir}/${BINARY_NAME}"
    rm -rf "$tmpdir"
    INSTALLED_METHOD="prebuilt"
    return 0
}

install_from_source() {
    local tmpdir repo_url source_dir binary_path

    if ! ensure_source_dependencies; then
        return 1
    fi

    repo_url="https://github.com/${REPO}.git"

    if [[ "$DRY_RUN" == "true" ]]; then
        info "Dry-run: would clone ${repo_url} at ref ${VERSION}"
        info "Dry-run: would run cargo build --release --locked"
        install_binary "/dev/null"
        INSTALLED_METHOD="source"
        return 0
    fi

    tmpdir="$(mktemp -d)"
    source_dir="${tmpdir}/source"

    info "Cloning source (${VERSION})..."
    if ! git clone --depth 1 --branch "$VERSION" "$repo_url" "$source_dir"; then
        warn "Shallow clone at ref '${VERSION}' failed, retrying full clone + checkout"
        if ! git clone "$repo_url" "$source_dir"; then
            rm -rf "$tmpdir"
            return 1
        fi
        (cd "$source_dir" && git checkout "$VERSION") || {
            rm -rf "$tmpdir"
            return 1
        }
    fi

    info "Building from source..."
    (
        cd "$source_dir"
        cargo build --release --locked
    )

    binary_path="${source_dir}/target/release/${BINARY_NAME}"
    if [[ ! -f "$binary_path" ]]; then
        warn "Built binary not found at ${binary_path}"
        rm -rf "$tmpdir"
        return 1
    fi

    install_binary "$binary_path"
    rm -rf "$tmpdir"
    INSTALLED_METHOD="source"
    return 0
}

run_guided_flow() {
    if [[ "$GUIDED" != "true" ]]; then
        return
    fi

    if [[ ! -t 0 ]]; then
        warn "--guided requested without TTY; continuing with non-interactive flow"
        return
    fi

    echo
    info "Guided bootstrap v2"

    local method_input dir_input
    read -r -p "Install method [auto/prebuilt/source] (default: ${INSTALL_METHOD}): " method_input
    if [[ -n "${method_input:-}" ]]; then
        INSTALL_METHOD="$method_input"
    fi

    read -r -p "Install directory (default: ${INSTALL_DIR}): " dir_input
    if [[ -n "${dir_input:-}" ]]; then
        INSTALL_DIR="$dir_input"
    fi

    if confirm "Run 'asterel onboard' after install?"; then
        RUN_ONBOARD=true
    fi

    echo
}

validate_options() {
    case "${INSTALL_METHOD}" in
        auto|prebuilt|source) ;;
        *) fail "Invalid --method '${INSTALL_METHOD}' (expected: auto|prebuilt|source)" ;;
    esac
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --guided)
                GUIDED=true
                ;;
            --yes|-y)
                ASSUME_YES=true
                ;;
            --method)
                [[ $# -ge 2 ]] || fail "Missing value for --method"
                INSTALL_METHOD="$2"
                shift
                ;;
            --install-dir)
                [[ $# -ge 2 ]] || fail "Missing value for --install-dir"
                INSTALL_DIR="$2"
                shift
                ;;
            --version)
                [[ $# -ge 2 ]] || fail "Missing value for --version"
                VERSION="$2"
                shift
                ;;
            --repo)
                [[ $# -ge 2 ]] || fail "Missing value for --repo"
                REPO="$2"
                shift
                ;;
            --run-onboard)
                RUN_ONBOARD=true
                ;;
            --dry-run)
                DRY_RUN=true
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                fail "Unknown option: $1 (see --help)"
                ;;
        esac
        shift
    done
}

main() {
    parse_args "$@"
    validate_options

    info "Detecting platform..."
    detect_platform

    run_guided_flow
    validate_options

    resolve_version

    case "$INSTALL_METHOD" in
        prebuilt)
            install_prebuilt || fail "Prebuilt install failed. Try --method source"
            ;;
        source)
            install_from_source || fail "Source install failed"
            ;;
        auto)
            if install_prebuilt; then
                :
            else
                warn "Falling back to source install..."
                install_from_source || fail "Auto install failed (both prebuilt and source paths failed)"
            fi
            ;;
    esac

    echo
    info "Asterel ${VERSION} installed successfully (${INSTALLED_METHOD})"
    echo
    echo "  Binary path:"
    echo "    ${INSTALL_PATH}"
    echo
    echo "  Suggested next steps:"
    echo "    ${INSTALL_PATH} onboard"
    echo "    ${INSTALL_PATH} agent"
    echo "    ${INSTALL_PATH} --help"
    echo
    echo "  Documentation:"
    echo "    https://github.com/${REPO}"
    echo

    if [[ "$RUN_ONBOARD" == "true" ]]; then
        if [[ "$DRY_RUN" == "true" ]]; then
            ok "Dry-run: would execute '${INSTALL_PATH} onboard'"
        else
            info "Running onboarding..."
            if ! "${INSTALL_PATH}" onboard; then
                warn "Onboarding command failed; run it manually later"
            fi
        fi
    fi
}

main "$@"
