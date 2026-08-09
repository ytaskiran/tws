#!/usr/bin/env bash
set -euo pipefail

REPO="ytaskiran/tws"
INSTALL_DIR="$HOME/.local/bin"
BINARY_NAME="tws"
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

# --- 4. Agent hooks (Claude Code + Codex + Pi) ---

# Emits a Claude/Codex hook "entry" JSON array for a single status word.
# Writes the word to ~/.config/tws/agents/$TMUX_PANE only when it changes
# (keeps the file mtime = state-entry time), then touches agent.trigger.
status_hook_entry() {
    local word="$1"
    local matcher="$2"   # "" for match-all
    local cmd
    cmd='f="$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p "#{pane_id}")}"; '
    cmd+='mkdir -p "$HOME/.config/tws/agents"; '
    cmd+='cur=$(cat "$f" 2>/dev/null); '
    cmd+="[ \"\$cur\" != $word ] && { printf $word > \"\$f\"; touch \"\$HOME/.config/tws/agent.trigger\"; }; :"
    printf '[{"matcher": "%s", "hooks": [{"type": "command", "command": %s}]}]' \
        "$matcher" "$(printf '%s' "$cmd" | jq -Rs .)"
}

# Emits the SessionStart hook entry that records <session_id>\t<cwd> for the pane,
# so `tws fork-pane` can fork that session. Skipped when TWS_FORK is set, which is
# how a fork avoids overwriting its parent's pointer.
session_hook_entry() {
    local cmd
    cmd='[ -n "$TWS_FORK" ] && exit 0; '
    cmd+='input=$(cat); '
    cmd+='id=$(printf "%s" "$input" | jq -r ".session_id // empty"); '
    cmd+='[ -z "$id" ] && exit 0; '
    cmd+='cwd=$(printf "%s" "$input" | jq -r ".cwd // empty"); '
    cmd+='[ -z "$cwd" ] && cwd=$PWD; '
    cmd+='p=${TMUX_PANE:-$(tmux display-message -p "#{pane_id}")}; '
    cmd+='[ -z "$p" ] && exit 0; '
    cmd+='mkdir -p "$HOME/.config/tws/sessions"; '
    cmd+='printf "%s\t%s\n" "$id" "$cwd" > "$HOME/.config/tws/sessions/$p"; :'
    printf '[{"matcher": "", "hooks": [{"type": "command", "command": %s}]}]' \
        "$(printf '%s' "$cmd" | jq -Rs .)"
}

session_end_entry() {
    local cmd
    cmd='rm -f "$HOME/.config/tws/sessions/${TMUX_PANE:-$(tmux display-message -p "#{pane_id}")}"; :'
    printf '[{"matcher": "", "hooks": [{"type": "command", "command": %s}]}]' \
        "$(printf '%s' "$cmd" | jq -Rs .)"
}

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

    printf '%s' "Configure/update Claude Code agent status hooks for tws? [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped Claude Code hooks"
        return
    fi

    local tmp
    tmp="$(mktemp)"
    local e_prompt e_pretool e_question e_notify e_stop e_end
    e_prompt=$(status_hook_entry working "")
    # Claude runs matching hooks in parallel, so keep these matchers disjoint.
    e_pretool=$(status_hook_entry working "^(?!AskUserQuestion$).*")
    e_question=$(status_hook_entry waiting "^AskUserQuestion$")
    e_notify=$(status_hook_entry waiting "permission_prompt|agent_needs_input")
    e_stop=$(status_hook_entry review "")
    e_end='[{"matcher": "", "hooks": [{"type": "command", "command": "rm -f \"$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p \"#{pane_id}\")}\"; touch \"$HOME/.config/tws/agent.trigger\""}]}]'
    local e_sessionstart e_sessionrm
    e_sessionstart=$(session_hook_entry)
    e_sessionrm=$(session_end_entry)

    jq \
        --argjson prompt "$e_prompt" \
        --argjson pretool "$e_pretool" \
        --argjson question "$e_question" \
        --argjson notify "$e_notify" \
        --argjson stop "$e_stop" \
        --argjson end "$e_end" \
        --argjson sessionstart "$e_sessionstart" \
        --argjson sessionrm "$e_sessionrm" '
        # A tws hook entry is identified by the config/tws/agents or config/tws/sessions marker in its command.
        def is_tws: (.hooks // []) | any((.command // "") | test("config/tws/(agents|sessions)"));
        .hooks //= {} |
        # Strip any prior tws entries (of any version/shape) from every event array,
        # leaving non-tws hooks untouched. Makes re-runs idempotent.
        .hooks |= with_entries(.value |= (if type == "array" then map(select(is_tws | not)) else . end)) |
        # Append the current, correct tws entries.
        .hooks.UserPromptSubmit = ((.hooks.UserPromptSubmit // []) + $prompt) |
        .hooks.PreToolUse       = ((.hooks.PreToolUse // []) + $pretool + $question) |
        .hooks.Notification     = ((.hooks.Notification // []) + $notify) |
        .hooks.Stop             = ((.hooks.Stop // []) + $stop) |
        .hooks.SessionStart     = ((.hooks.SessionStart // []) + $sessionstart) |
        .hooks.SessionEnd       = ((.hooks.SessionEnd // []) + $end + $sessionrm) |
        # Drop any event arrays left empty (e.g. a legacy event we no longer populate).
        .hooks |= with_entries(select((.value | length) > 0))
    ' "$settings" > "$tmp" && mv "$tmp" "$settings"
    ok "Configured Claude Code agent status hooks"
}

configure_codex_feature_flag() {
    local config_file="$HOME/.codex/config.toml"

    # Already enabled?
    if grep -q '^\s*hooks\s*=\s*true' "$config_file" 2>/dev/null; then
        return
    fi

    local tmp
    tmp="$(mktemp)"

    if [ ! -f "$config_file" ]; then
        printf '[features]\nhooks = true\n' > "$config_file"
    elif grep -q '^\[features\]' "$config_file"; then
        # [features] section exists — insert hooks = true after the header
        awk '/^\[features\]/{print; print "hooks = true"; next}1' "$config_file" > "$tmp" && mv "$tmp" "$config_file"
    else
        # No [features] section — append it
        printf '\n[features]\nhooks = true\n' >> "$config_file"
    fi
    ok "Enabled hooks feature in ~/.codex/config.toml"
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

    printf '%s' "Configure/update Codex agent status hooks for tws? [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped Codex hooks"
        return
    fi

    [ -f "$hooks_file" ] || echo '{}' > "$hooks_file"

    local tmp
    tmp="$(mktemp)"
    local e_work e_wait e_review e_end
    e_work=$(status_hook_entry working "")
    e_wait=$(status_hook_entry waiting "")
    e_review=$(status_hook_entry review "")
    e_end='[{"matcher": "", "hooks": [{"type": "command", "command": "rm -f \"$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p \"#{pane_id}\")}\"; touch \"$HOME/.config/tws/agent.trigger\""}]}]'

    jq \
        --argjson work "$e_work" --argjson wait "$e_wait" \
        --argjson review "$e_review" --argjson end "$e_end" '
        # A tws hook entry is identified by the config/tws/agents marker in its command.
        def is_tws: (.hooks // []) | any((.command // "") | contains("config/tws/agents"));
        .hooks //= {} |
        # Strip any prior tws entries (of any version/shape) from every event array,
        # leaving non-tws hooks untouched. Makes re-runs idempotent.
        .hooks |= with_entries(.value |= (if type == "array" then map(select(is_tws | not)) else . end)) |
        # Append the current, correct tws entries.
        .hooks.UserPromptSubmit   = ((.hooks.UserPromptSubmit // []) + $work) |
        .hooks.PreToolUse         = ((.hooks.PreToolUse // []) + $work) |
        .hooks.PermissionRequest  = ((.hooks.PermissionRequest // []) + $wait) |
        .hooks.Stop               = ((.hooks.Stop // []) + $review) |
        .hooks.SessionEnd         = ((.hooks.SessionEnd // []) + $end) |
        # Drop any event arrays left empty (e.g. a legacy event we no longer populate).
        .hooks |= with_entries(select((.value | length) > 0))
    ' "$hooks_file" > "$tmp" && mv "$tmp" "$hooks_file"
    ok "Configured Codex agent status hooks"

    configure_codex_feature_flag
}

configure_pi_hooks() {
    local ext_dir="$HOME/.pi/agent/extensions"
    local ext_file="$ext_dir/tws-status.ts"

    if [ ! -d "$HOME/.pi" ]; then
        info "Pi config not found — skipping agent hooks"
        return
    fi

    printf '%s' "Configure/update Pi agent status hooks for tws? [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped Pi hooks"
        return
    fi

    mkdir -p "$ext_dir"
    # Pi has no declarative hooks.json — extensions are TS modules loaded from
    # ~/.pi/agent/extensions/*.ts. Overwriting this file wholesale is safe and
    # idempotent since tws owns it outright (unlike the Claude/Codex configs,
    # which are shared JSON we must merge into carefully).
    cat > "$ext_file" <<'PI_EXT_EOF'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";

const AGENTS_DIR = `${process.env.HOME}/.config/tws/agents`;
const TRIGGER = `${process.env.HOME}/.config/tws/agent.trigger`;

async function panePath(pi: any): Promise<string | undefined> {
  let pane = process.env.TMUX_PANE;
  if (!pane) {
    try {
      const result = await pi.exec("tmux", ["display-message", "-p", "#{pane_id}"]);
      pane = result.stdout?.trim();
    } catch {
      return undefined;
    }
  }
  return pane ? `${AGENTS_DIR}/${pane}` : undefined;
}

// Only writes when the word actually changes, so the file's mtime reflects
// state-entry time (tws reads mtime as status_since).
function writeWord(path: string, word: string) {
  let cur: string | undefined;
  try {
    cur = readFileSync(path, "utf8");
  } catch {
    cur = undefined;
  }
  if (cur === word) return;
  mkdirSync(AGENTS_DIR, { recursive: true });
  writeFileSync(path, word);
  writeFileSync(TRIGGER, "");
}

export default function (pi: any) {
  pi.on("turn_start", async () => {
    const path = await panePath(pi);
    if (path) writeWord(path, "working");
  });
  pi.on("agent_settled", async () => {
    const path = await panePath(pi);
    if (path) writeWord(path, "review");
  });
  pi.on("session_shutdown", async () => {
    const path = await panePath(pi);
    if (path && existsSync(path)) {
      rmSync(path, { force: true });
      writeFileSync(TRIGGER, "");
    }
  });
}
PI_EXT_EOF

    ok "Configured Pi agent status hooks"
}

configure_agent_hooks() {
    configure_claude_hooks
    configure_codex_hooks
    configure_pi_hooks
}

# --- 5. Optional: tmux fork binding (experimental) ---

FORK_BINDING='bind-key F display-popup -E -w 90% -h 85% "tws fork-pane"'
FORK_MARKER='# tws fork binding'

configure_fork_binding() {
    local conf="$HOME/.tmux.conf"

    if [ ! -f "$conf" ]; then
        info "No ~/.tmux.conf — skipping fork binding"
        return
    fi

    printf '%s' "Add tws fork binding (prefix+F) to ~/.tmux.conf? [EXPERIMENTAL] [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped fork binding — add it manually with:"
        printf '  %s\n' "$FORK_BINDING"
        return
    fi

    if tmux list-keys -T prefix 2>/dev/null | grep -qE '^bind-key[[:space:]]+(-T prefix[[:space:]]+)?F[[:space:]]' \
        && ! grep -qF "$FORK_MARKER" "$conf"; then
        warn "prefix+F is already bound to something else — not overwriting"
        info "Add this manually under a different key if you want it:"
        printf '  %s\n' "$FORK_BINDING"
        return
    fi

    # Idempotent: drop any previously marked block before re-adding.
    if grep -qF "$FORK_MARKER" "$conf"; then
        local tmp
        tmp="$(mktemp)"
        grep -vF -e "$FORK_MARKER" -e "tws fork-pane" "$conf" > "$tmp" && mv "$tmp" "$conf"
    fi

    printf '\n%s\n%s\n' "$FORK_MARKER" "$FORK_BINDING" >> "$conf"
    ok "Added fork binding (prefix+F) — EXPERIMENTAL"
    info "Run: tmux source-file ~/.tmux.conf"
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
    configure_agent_hooks
    configure_fork_binding
    configure_glow

    echo ""
    ok "Done!"
    echo "  Binary:   $INSTALL_DIR/$BINARY_NAME"
    echo "  Run:      tws"
    echo "  Detach:   prefix + d"
    echo ""
}

main
