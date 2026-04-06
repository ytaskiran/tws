#!/usr/bin/env bash
set -euo pipefail

REPO="ytaskiran/tws"
INSTALL_DIR="$HOME/.local/bin"
BINARY_NAME="tws"
BIND_LINE='bind-key s display-popup -E -w 100% -h 100% -b none "tws"'
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

    # Strip macOS quarantine/provenance attributes so Gatekeeper
    # doesn't kill the ad-hoc-signed binary on first launch.
    if [ "$(uname -s)" = "Darwin" ]; then
        xattr -dr com.apple.quarantine "$INSTALL_DIR/$BINARY_NAME" 2>/dev/null || true
        xattr -dr com.apple.provenance "$INSTALL_DIR/$BINARY_NAME" 2>/dev/null || true
    fi

    ok "Installed $BINARY_NAME to $INSTALL_DIR/$BINARY_NAME"

    # PATH check
    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            warn "$INSTALL_DIR is not in your PATH"
            configure_path
            ;;
    esac
}

configure_path() {
    local export_line='export PATH="$HOME/.local/bin:$PATH"'

    # Detect shell rc and profile files
    local rc_file="" profile_file=""
    case "$(basename "$SHELL")" in
        zsh)
            rc_file="$HOME/.zshrc"
            profile_file="$HOME/.zprofile"
            ;;
        bash)
            rc_file="$HOME/.bashrc"
            profile_file="$HOME/.bash_profile"
            ;;
    esac

    if [ -z "$rc_file" ]; then
        info "Add this to your shell rc and profile:"
        echo "  $export_line"
        return
    fi

    printf '%s' "Add $INSTALL_DIR to PATH in $rc_file and $profile_file? [y/N] "
    read -r answer < /dev/tty

    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped. Add this manually to $rc_file and $profile_file:"
        echo "  $export_line"
        return
    fi

    for file in "$rc_file" "$profile_file"; do
        if grep -q '$HOME/.local/bin' "$file" 2>/dev/null; then
            ok "PATH entry already exists in $file — skipping"
            continue
        fi
        echo "" >> "$file"
        echo '# tws' >> "$file"
        echo "$export_line" >> "$file"
        ok "Added PATH entry to $file"
    done

    info "Restart your shell or run: source $rc_file"
}

# --- 4. Configure tmux ---

configure_tmux() {
    printf '%s' "Add tws keybinding to tmux.conf? (prefix + s) [y/N] "
    read -r answer < /dev/tty

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
    if grep -q '^[^#]*display-popup.*tws' "$tmux_conf" 2>/dev/null; then
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

    # Ensure pre-existing tmux sessions pick up PATH changes
    if ! grep -q 'update-environment.*PATH' "$tmux_conf" 2>/dev/null; then
        echo 'set -ga update-environment " PATH"' >> "$tmux_conf"
    fi

    ok "Added keybinding to $tmux_conf"
}

# --- 5. Agent hooks (Claude Code + Codex) ---

TRIGGER_CMD="touch \$HOME/.config/tws/agent.trigger"
HOOK_ENTRY='[{"matcher": "", "hooks": [{"type": "command", "command": "'"$TRIGGER_CMD"'"}]}]'

configure_claude_hooks() {
    local settings="$HOME/.claude/settings.json"

    if [ ! -f "$settings" ]; then
        info "Claude Code settings not found — skipping agent hooks"
        return
    fi

    if ! command -v jq &>/dev/null; then
        warn "jq not found — cannot auto-configure Claude Code hooks"
        info "Install jq, then re-run install, or add hooks manually"
        return
    fi

    # Already configured?
    if grep -q 'agent\.trigger' "$settings" 2>/dev/null; then
        ok "Claude Code agent hooks already configured"
        return
    fi

    printf '%s' "Configure Claude Code agent hooks for tws? [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped Claude Code hooks"
        return
    fi

    local tmp
    tmp="$(mktemp)"
    jq --argjson entry "$HOOK_ENTRY" '
        .hooks //= {} |
        .hooks.SessionStart //= [] |
        .hooks.SessionEnd //= [] |
        .hooks.SessionStart += $entry |
        .hooks.SessionEnd += $entry
    ' "$settings" > "$tmp" && mv "$tmp" "$settings"
    ok "Configured Claude Code agent hooks"
}

configure_codex_feature_flag() {
    local config_file="$HOME/.codex/config.toml"

    # Already enabled?
    if grep -q 'codex_hooks\s*=\s*true' "$config_file" 2>/dev/null; then
        return
    fi

    local tmp
    tmp="$(mktemp)"

    if [ ! -f "$config_file" ]; then
        printf '[features]\ncodex_hooks = true\n' > "$config_file"
    elif grep -q '^\[features\]' "$config_file"; then
        # [features] section exists — insert codex_hooks = true after the header
        awk '/^\[features\]/{print; print "codex_hooks = true"; next}1' "$config_file" > "$tmp" && mv "$tmp" "$config_file"
    else
        # No [features] section — append it
        printf '\n[features]\ncodex_hooks = true\n' >> "$config_file"
    fi
    ok "Enabled codex_hooks feature in ~/.codex/config.toml"
}

configure_codex_hooks() {
    local hooks_file="$HOME/.codex/hooks.json"

    if [ ! -d "$HOME/.codex" ]; then
        info "Codex config not found — skipping agent hooks"
        return
    fi

    if ! command -v jq &>/dev/null; then
        warn "jq not found — cannot auto-configure Codex hooks"
        return
    fi

    local hooks_ok=false
    local feature_ok=false
    grep -q 'agent\.trigger' "$hooks_file" 2>/dev/null && hooks_ok=true
    grep -q 'codex_hooks\s*=\s*true' "$HOME/.codex/config.toml" 2>/dev/null && feature_ok=true

    if $hooks_ok && $feature_ok; then
        ok "Codex agent hooks already configured"
        return
    fi

    printf '%s' "Configure Codex agent hooks for tws? [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped Codex hooks"
        return
    fi

    if ! $hooks_ok; then
        [ -f "$hooks_file" ] || echo '{}' > "$hooks_file"

        local tmp
        tmp="$(mktemp)"
        jq --argjson entry "$HOOK_ENTRY" '
            .SessionStart //= [] |
            .Stop //= [] |
            .SessionStart += $entry |
            .Stop += $entry
        ' "$hooks_file" > "$tmp" && mv "$tmp" "$hooks_file"
        ok "Configured Codex agent hooks"
    fi

    if ! $feature_ok; then
        configure_codex_feature_flag
    fi
}

configure_agent_hooks() {
    configure_claude_hooks
    configure_codex_hooks
}

# --- 6. Optional: glow (rich markdown rendering) ---

configure_glow() {
    if command -v glow &>/dev/null; then
        ok "glow found — rich markdown rendering enabled"
        return
    fi

    warn "glow not found — notes will use basic markdown rendering"
    printf '%s' "Install glow for rich markdown preview? [y/N] "
    read -r answer < /dev/tty

    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped. Install later: brew install glow (macOS) or go install github.com/charmbracelet/glow@latest"
        return
    fi

    if command -v brew &>/dev/null; then
        info "Installing glow via Homebrew..."
        brew install glow && ok "glow installed" || warn "glow installation failed — notes will use basic rendering"
    elif command -v go &>/dev/null; then
        info "Installing glow via Go..."
        go install github.com/charmbracelet/glow@latest && ok "glow installed" || warn "glow installation failed"
    else
        warn "Could not auto-install glow. Install manually:"
        echo "  macOS:  brew install glow"
        echo "  Linux:  go install github.com/charmbracelet/glow@latest"
    fi
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
    configure_agent_hooks
    configure_glow

    echo ""
    ok "Done!"
    echo "  Binary:   $INSTALL_DIR/$BINARY_NAME"
    echo "  Run:      tws"
    echo "  In tmux:  prefix + s"
    echo ""
}

main
