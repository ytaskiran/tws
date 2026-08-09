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
# agent.trigger stays guarded in every mode — ringing it per tool call would
# force a full tmux+ps rescan. The mtime refresh is `touch -c` because `>`
# truncates before writing, exposing an empty file to concurrent readers.
#
# Modes:
#   set    unconditional — the event names the new state outright.
#   live   liveness only — refresh `working`, or claim an empty file. A pane in a
#          resting state stays there. Background subagents share the pane with the
#          main loop and fire the same tool hooks, so without this guard their
#          tool calls repaint a finished or question-blocked pane as `working`.
#   alert  raise `waiting`, but only over `working` or an empty file. Leaving
#          `review` alone keeps attach-time acknowledgment working.
status_hook_entry() {
    local word="$1"
    local matcher="$2"      # "" for match-all
    local mode="${3:-set}"
    local cmd trig
    trig='touch "$HOME/.config/tws/agent.trigger"'
    cmd='f="$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p "#{pane_id}")}"; '
    cmd+='mkdir -p "$HOME/.config/tws/agents"; '
    cmd+='cur=$(cat "$f" 2>/dev/null); '
    case "$mode" in
        live)
            cmd+="if [ \"\$cur\" = $word ]; then touch -c \"\$f\"; "
            cmd+="elif [ -z \"\$cur\" ]; then printf $word > \"\$f\"; $trig; fi; :"
            ;;
        alert)
            cmd+="if [ \"\$cur\" = working ] || [ -z \"\$cur\" ]; then printf $word > \"\$f\"; $trig; fi; :"
            ;;
        *)
            cmd+="[ \"\$cur\" != $word ] && { printf $word > \"\$f\"; $trig; }; :"
            ;;
    esac
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
    local e_prompt e_pretool e_question e_posttool e_notify e_stop e_compact e_fail e_end
    # Submitting a prompt is the only event that starts a turn, so it is the only
    # unconditional route back to `working`.
    e_prompt=$(status_hook_entry working "")
    # Claude runs matching hooks in parallel, so keep these matchers disjoint.
    e_pretool=$(status_hook_entry working "^(?!AskUserQuestion$).*" live)
    e_question=$(status_hook_entry waiting "^AskUserQuestion$")
    # The "turn resumed" signal, scoped to the question it answers. A match-all
    # PostToolUse would hand every background subagent the same power, and its
    # tool calls would repaint the pane the moment `Stop` set `review`.
    e_posttool=$(status_hook_entry working "^AskUserQuestion$")
    # `idle_prompt` is the real event name — Claude sends it 60s after the main
    # loop goes quiet. It is also the backstop that heals a pane no other hook
    # reached. `agent_needs_input`, the name used before, never existed.
    e_notify=$(status_hook_entry waiting "permission_prompt|idle_prompt" alert)
    e_stop=$(status_hook_entry review "")
    # Compaction and API errors end a turn without firing Stop.
    e_compact=$(status_hook_entry review "manual|auto")
    e_fail=$(status_hook_entry review "")
    e_end='[{"matcher": "", "hooks": [{"type": "command", "command": "rm -f \"$HOME/.config/tws/agents/${TMUX_PANE:-$(tmux display-message -p \"#{pane_id}\")}\"; touch \"$HOME/.config/tws/agent.trigger\""}]}]'

    jq \
        --argjson prompt "$e_prompt" \
        --argjson pretool "$e_pretool" \
        --argjson question "$e_question" \
        --argjson posttool "$e_posttool" \
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
        .hooks.PostToolUse      = ((.hooks.PostToolUse // []) + $posttool) |
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
    e_pretool=$(status_hook_entry working "" live)
    e_wait=$(status_hook_entry waiting "" alert)
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
        # PermissionRequest enters waiting; this is its only bounded exit.
        .hooks.PostToolUse        = ((.hooks.PostToolUse // []) + $work) |
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

// TRIGGER stays quiet during a run: tws does a full tmux+ps rescan when rung.
function writeWord(path: string, word: string) {
  if (readWord(path) === word) return;
  mkdirSync(AGENTS_DIR, { recursive: true });
  writeFileSync(path, word);
  writeFileSync(TRIGGER, "");
}

// Refreshing mtime is how a pane proves liveness to tws. Never creates the file;
// a missing one is restored by the next writeWord.
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
    const cur = readWord(path);
    // A tool call proves liveness, it does not start a turn. A pane resting in
    // review or waiting stays there; only an empty file is claimed.
    if (cur === "working") beat(path);
    else if (cur === undefined) writeWord(path, "working");
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

    # Agents snapshot hook config at session start, so a file already stuck at
    # `working` would outlive this upgrade. Live panes rewrite theirs on the next
    # hook fire.
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
