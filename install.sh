#!/usr/bin/env bash
set -euo pipefail

REPO="ytaskiran/tws"
INSTALL_DIR="$HOME/.local/bin"
BINARY_NAME="tws"
BIND_LINE='bind-key s display-popup -E -w 80% -h 80% "tws"'
tmpdir=""

# --- Helpers ---

info()  { printf '\033[1;34m::\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m✓\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m!\033[0m %s\n' "$*"; }
err()   { printf '\033[1;31m✗\033[0m %s\n' "$*" >&2; exit 1; }

# --- 1. Detect platform ---

detect_target() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin) os="apple-darwin" ;;
        Linux)  os="unknown-linux-gnu" ;;
        *)      err "Unsupported OS: $os" ;;
    esac

    case "$arch" in
        x86_64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)       err "Unsupported architecture: $arch" ;;
    esac

    echo "${arch}-${os}"
}

# --- 2. Get binary ---

get_binary() {
    local script_dir
    script_dir="$(cd "$(dirname "$0")" && pwd)"

    # If a tws binary sits next to this script (local install), use it
    if [ -x "$script_dir/$BINARY_NAME" ]; then
        info "Found local binary at $script_dir/$BINARY_NAME"
        cp "$script_dir/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
        return
    fi

    # Otherwise download the latest release
    local target="$1"
    info "Fetching latest release from GitHub..."

    local latest_tag
    latest_tag="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep '"tag_name"' | head -1 | sed 's/.*: "//;s/".*//')"

    [ -n "$latest_tag" ] || err "Could not determine latest release"
    info "Latest release: $latest_tag"

    local archive="tws-${latest_tag}-${target}.tar.gz"
    local url="https://github.com/$REPO/releases/download/${latest_tag}/${archive}"

    info "Downloading $archive..."
    tmpdir="$(mktemp -d)"
    trap 'rm -rf "$tmpdir"' EXIT  # safe: tmpdir is global, initialized to ""

    curl -fSL --progress-bar "$url" -o "$tmpdir/$archive" \
        || err "Download failed. Is there a release for $target?"

    tar xzf "$tmpdir/$archive" -C "$tmpdir"
    cp "$tmpdir/tws-${latest_tag}-${target}/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
}

# --- 3. Install binary ---

install_binary() {
    local target="$1"
    mkdir -p "$INSTALL_DIR"

    get_binary "$target"
    chmod +x "$INSTALL_DIR/$BINARY_NAME"
    ok "Installed $BINARY_NAME to $INSTALL_DIR/$BINARY_NAME"

    # PATH check
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            warn "$INSTALL_DIR is not in your PATH"
            echo "  Add this to your shell rc (~/.bashrc, ~/.zshrc, etc.):"
            echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
            echo ""
            ;;
    esac
}

# --- 4. Configure tmux ---

configure_tmux() {
    printf '%s' "Add tws keybinding to tmux.conf? (prefix + s) [y/N] "
    read -r answer

    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped. To add manually, put this in your tmux.conf:"
        echo "  $BIND_LINE"
        return
    fi

    # Find tmux.conf
    local tmux_conf=""
    if [ -f "$HOME/.config/tmux/tmux.conf" ]; then
        tmux_conf="$HOME/.config/tmux/tmux.conf"
    elif [ -f "$HOME/.tmux.conf" ]; then
        tmux_conf="$HOME/.tmux.conf"
    else
        tmux_conf="$HOME/.tmux.conf"
    fi

    # Check if binding already exists
    if grep -q "tws" "$tmux_conf" 2>/dev/null; then
        ok "tws binding already exists in $tmux_conf — skipping"
        return
    fi

    # Back up and append
    if [ -f "$tmux_conf" ]; then
        cp "$tmux_conf" "${tmux_conf}.bak"
        info "Backed up $tmux_conf → ${tmux_conf}.bak"
    fi

    echo "" >> "$tmux_conf"
    echo "# tws — tmux workspace manager (prefix + s)" >> "$tmux_conf"
    echo "$BIND_LINE" >> "$tmux_conf"
    ok "Added keybinding to $tmux_conf"
}

# --- Main ---

main() {
    echo ""
    info "Installing tws — tmux workspace manager"
    echo ""

    local target
    target="$(detect_target)"
    info "Detected platform: $target"

    install_binary "$target"
    configure_tmux

    echo ""
    ok "Done!"
    echo "  Binary:   $INSTALL_DIR/$BINARY_NAME"
    echo "  Run:      tws"
    echo "  In tmux:  prefix + s"
    echo ""
}

main
