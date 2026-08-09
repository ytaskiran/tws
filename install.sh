#!/usr/bin/env bash
set -euo pipefail

REPO="ytaskiran/tws"
INSTALL_DIR="$HOME/.local/bin"
BINARY_NAME="tws"
tmpdir=""
hooks_configured=0

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
# Writes the word to ~/.config/tws/agents/$TMUX_PANE only when it changes, then
# touches agent.trigger. With a third argument the unchanged case refreshes the
# file's mtime instead of doing nothing, which tws reads as a liveness heartbeat
# (see core::status::expire_stale_working). agent.trigger stays guarded either
# way: ringing it per tool call would force a full tmux+ps rescan every few
# seconds. The refresh is `touch -c` rather than a rewrite because `>` truncates
# before writing, so a concurrent reader can see an empty file, and -c avoids
# recreating a just-deleted file as an empty one.
status_hook_entry() {
    local word="$1"
    local matcher="$2"      # "" for match-all
    local heartbeat="${3:-}"
    local cmd
    cmd='f="$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p "#{pane_id}")}"; '
    cmd+='mkdir -p "$HOME/.config/tws/agents"; '
    cmd+='cur=$(cat "$f" 2>/dev/null); '
    if [ -n "$heartbeat" ]; then
        cmd+="if [ \"\$cur\" != $word ]; then printf $word > \"\$f\"; touch \"\$HOME/.config/tws/agent.trigger\"; else touch -c \"\$f\"; fi; :"
    else
        cmd+="[ \"\$cur\" != $word ] && { printf $word > \"\$f\"; touch \"\$HOME/.config/tws/agent.trigger\"; }; :"
    fi
    printf '[{"matcher": "%s", "hooks": [{"type": "command", "command": %s}]}]' \
        "$matcher" "$(printf '%s' "$cmd" | jq -Rs .)"
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
    local e_prompt e_pretool e_question e_notify e_stop e_compact e_fail e_end
    e_prompt=$(status_hook_entry working "")
    # Claude runs matching hooks in parallel, so keep these matchers disjoint.
    # Only the working entry heartbeats: a pane parked on a question is meant to
    # go quiet, and that entry writes waiting anyway.
    e_pretool=$(status_hook_entry working "^(?!AskUserQuestion$).*" heartbeat)
    e_question=$(status_hook_entry waiting "^AskUserQuestion$")
    e_notify=$(status_hook_entry waiting "permission_prompt|agent_needs_input")
    e_stop=$(status_hook_entry review "")
    # Compaction and API errors end a turn without firing Stop, which otherwise
    # leaves the pane pinned at working until stale expiry catches it.
    e_compact=$(status_hook_entry review "manual|auto")
    e_fail=$(status_hook_entry review "")
    e_end='[{"matcher": "", "hooks": [{"type": "command", "command": "rm -f \"$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p \"#{pane_id}\")}\"; touch \"$HOME/.config/tws/agent.trigger\""}]}]'

    jq \
        --argjson prompt "$e_prompt" \
        --argjson pretool "$e_pretool" \
        --argjson question "$e_question" \
        --argjson notify "$e_notify" \
        --argjson stop "$e_stop" \
        --argjson compact "$e_compact" \
        --argjson fail "$e_fail" \
        --argjson end "$e_end" '
        # A tws hook entry is identified by the config/tws/agents marker in its command.
        def is_tws: (.hooks // []) | any((.command // "") | contains("config/tws/agents"));
        .hooks //= {} |
        # Strip any prior tws entries (of any version/shape) from every event array,
        # leaving non-tws hooks untouched. Makes re-runs idempotent.
        .hooks |= with_entries(.value |= (if type == "array" then map(select(is_tws | not)) else . end)) |
        # Append the current, correct tws entries.
        .hooks.UserPromptSubmit = ((.hooks.UserPromptSubmit // []) + $prompt) |
        .hooks.PreToolUse       = ((.hooks.PreToolUse // []) + $pretool + $question) |
        .hooks.Notification     = ((.hooks.Notification // []) + $notify) |
        .hooks.Stop             = ((.hooks.Stop // []) + $stop) |
        .hooks.PostCompact      = ((.hooks.PostCompact // []) + $compact) |
        .hooks.StopFailure      = ((.hooks.StopFailure // []) + $fail) |
        .hooks.SessionEnd       = ((.hooks.SessionEnd // []) + $end) |
        # Drop any event arrays left empty (e.g. a legacy event we no longer populate).
        .hooks |= with_entries(select((.value | length) > 0))
    ' "$settings" > "$tmp" && mv "$tmp" "$settings"
    ok "Configured Claude Code agent status hooks"
    hooks_configured=1
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
    local e_work e_pretool e_wait e_review e_compact e_end
    e_work=$(status_hook_entry working "")
    e_pretool=$(status_hook_entry working "" heartbeat)
    e_wait=$(status_hook_entry waiting "")
    e_review=$(status_hook_entry review "")
    # Codex has no API-error event, so stale expiry is the only backstop there.
    e_compact=$(status_hook_entry review "manual|auto")
    e_end='[{"matcher": "", "hooks": [{"type": "command", "command": "rm -f \"$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p \"#{pane_id}\")}\"; touch \"$HOME/.config/tws/agent.trigger\""}]}]'

    jq \
        --argjson work "$e_work" --argjson pretool "$e_pretool" --argjson wait "$e_wait" \
        --argjson review "$e_review" --argjson compact "$e_compact" --argjson end "$e_end" '
        # A tws hook entry is identified by the config/tws/agents marker in its command.
        def is_tws: (.hooks // []) | any((.command // "") | contains("config/tws/agents"));
        .hooks //= {} |
        # Strip any prior tws entries (of any version/shape) from every event array,
        # leaving non-tws hooks untouched. Makes re-runs idempotent.
        .hooks |= with_entries(.value |= (if type == "array" then map(select(is_tws | not)) else . end)) |
        # Append the current, correct tws entries.
        .hooks.UserPromptSubmit   = ((.hooks.UserPromptSubmit // []) + $work) |
        .hooks.PreToolUse         = ((.hooks.PreToolUse // []) + $pretool) |
        .hooks.PermissionRequest  = ((.hooks.PermissionRequest // []) + $wait) |
        .hooks.Stop               = ((.hooks.Stop // []) + $review) |
        .hooks.PostCompact        = ((.hooks.PostCompact // []) + $compact) |
        .hooks.SessionEnd         = ((.hooks.SessionEnd // []) + $end) |
        # Drop any event arrays left empty (e.g. a legacy event we no longer populate).
        .hooks |= with_entries(select((.value | length) > 0))
    ' "$hooks_file" > "$tmp" && mv "$tmp" "$hooks_file"
    ok "Configured Codex agent status hooks"
    hooks_configured=1

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
import { existsSync, mkdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";

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

function readWord(path: string): string | undefined {
  try {
    return readFileSync(path, "utf8");
  } catch {
    return undefined;
  }
}

// Only writes on a real change, so TRIGGER stays quiet during a run — tws does a
// full tmux+ps rescan every time it is rung.
function writeWord(path: string, word: string) {
  if (readWord(path) === word) return;
  mkdirSync(AGENTS_DIR, { recursive: true });
  writeFileSync(path, word);
  writeFileSync(TRIGGER, "");
}

// Heartbeat: refresh mtime without rewriting, which tws reads as proof of life
// (see core::status::expire_stale_working). Never creates the file; a missing one
// is restored by the next writeWord.
function beat(path: string) {
  const now = new Date();
  try {
    utimesSync(path, now, now);
  } catch {
    // Pane file not there yet — the next state change writes it.
  }
}

export default function (pi: any) {
  pi.on("turn_start", async () => {
    const path = await panePath(pi);
    if (path) writeWord(path, "working");
  });
  // Pi's only per-tool-call event, and so the only place a heartbeat can live.
  pi.on("tool_execution_start", async () => {
    const path = await panePath(pi);
    if (!path) return;
    if (readWord(path) === "working") beat(path);
    else writeWord(path, "working");
  });
  // Compaction can end a turn without agent_settled firing.
  pi.on("session_compact", async () => {
    const path = await panePath(pi);
    if (path) writeWord(path, "review");
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
    hooks_configured=1
}

configure_agent_hooks() {
    configure_claude_hooks
    configure_codex_hooks
    configure_pi_hooks

    # Agents snapshot their hook config at session start, so panes running during
    # this upgrade keep the old config — and any file already stuck at `working`
    # would outlive it. Clearing once makes the upgrade clean; live panes rewrite
    # their file on the next hook fire.
    if [ "$hooks_configured" -eq 1 ]; then
        rm -f "$HOME"/.config/tws/agents/* 2>/dev/null || true
        mkdir -p "$HOME/.config/tws"
        touch "$HOME/.config/tws/agent.trigger"
        info "Cleared stale agent status files"
    fi
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
    configure_glow

    echo ""
    ok "Done!"
    echo "  Binary:   $INSTALL_DIR/$BINARY_NAME"
    echo "  Run:      tws"
    echo "  Detach:   prefix + d"
    echo ""
}

main
