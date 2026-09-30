#!/usr/bin/env bash
set -euo pipefail

REPO="ytaskiran/tws"
INSTALL_DIR="$HOME/.local/bin"
BINARY_NAME="tws"
tmpdir=""
hooks_configured=0
# Minutes a subagent marker counts as live. Equals STALE_WORKING_SECS (15 min) in
# src/core/status.rs: tws expires a silent `working` pane after that long, so an
# older marker must not hold the pane working.
SUBAGENT_FRESH_MINS=15

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
#   tool   PreToolUse for Claude. Reads the payload: a call with an `agent_id`, or
#          one whose payload cannot be read, is a subagent and behaves as `live`
#          (and refreshes its marker). A call
#          without one is the main loop, so it proves the turn is live: it also
#          resumes `review` and `idle`. Only `waiting` is left alone, because a
#          background subagent can hold the pane there for a permission prompt.
#   stop   turn end. While a fresh subagent marker exists it writes `working`,
#          but keeps `waiting`: a background subagent can hold the pane there for
#          a permission prompt. Without a marker it writes the word. Also deletes
#          stale markers.
#   idle_alert  `alert` for `idle_prompt`, skipped while a fresh marker exists.
status_hook_entry() {
    local word="$1"
    local matcher="$2"      # "" for match-all
    local mode="${3:-set}"
    local cmd trig
    trig='touch "$HOME/.config/tws/agent.trigger"'
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='f="$HOME/.config/tws/agents/$TMUX_PANE"; '
    cmd+='mkdir -p "$HOME/.config/tws/agents"; '
    cmd+='cur=$(cat "$f" 2>/dev/null); '
    cmd+='sd="$HOME/.config/tws/subagents/$TMUX_PANE"; '
    local fresh="[ -n \"\$(find \"\$sd\" -type f -mmin -$SUBAGENT_FRESH_MINS 2>/dev/null | head -n 1)\" ]"
    case "$mode" in
        tool)
            # A failed jq (missing, bad JSON) cannot prove this is the main loop,
            # so it takes the conservative subagent path.
            cmd+='if ! aid=$(jq -r ".agent_id // empty" 2>/dev/null); then aid=.unknown; fi; '
            cmd+='if [ -n "$aid" ]; then touch -c "$sd/$aid" 2>/dev/null; '
            cmd+="if [ \"\$cur\" = $word ]; then touch -c \"\$f\"; "
            cmd+="elif [ -z \"\$cur\" ]; then printf $word > \"\$f\"; $trig; fi; "
            cmd+='else case "$cur" in '
            cmd+="$word) touch -c \"\$f\" ;; waiting) ;; "
            cmd+="*) printf $word > \"\$f\"; $trig ;; esac; fi; :"
            ;;
        stop)
            cmd+="find \"\$sd\" -type f ! -mmin -$SUBAGENT_FRESH_MINS -delete 2>/dev/null; "
            cmd+="if $fresh; then case \"\$cur\" in waiting) w=waiting ;; *) w=working ;; esac; "
            cmd+="else w=$word; fi; "
            cmd+="[ \"\$cur\" != \"\$w\" ] && { printf %s \"\$w\" > \"\$f\"; $trig; }; :"
            ;;
        idle_alert)
            cmd+="if $fresh; then :; "
            cmd+="elif [ \"\$cur\" = working ] || [ -z \"\$cur\" ]; then printf $word > \"\$f\"; $trig; fi; :"
            ;;
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

# Emits the SubagentStart / SubagentStop hook entry. A marker file named for the
# subagent tells `stop` mode that work continues after the main loop ends its turn.
# A stop with no marker is normal (compaction sends one), so it removes nothing.
subagent_hook_entry() {
    local kind="$1"    # start | stop
    local cmd
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='aid=$(jq -r ".agent_id // empty" 2>/dev/null); '
    cmd+='case "$aid" in ""|*/*|.*) exit 0 ;; esac; '
    cmd+='sd="$HOME/.config/tws/subagents/$TMUX_PANE"; '
    if [ "$kind" = start ]; then
        cmd+='mkdir -p "$sd" && touch "$sd/$aid"; :'
    else
        cmd+='rm -f "$sd/$aid"; :'
    fi
    printf '[{"matcher": "", "hooks": [{"type": "command", "command": %s}]}]' \
        "$(printf '%s' "$cmd" | jq -Rs .)"
}

# The end-of-session counterpart: drops this pane's status file and markers.
session_end_hook_entry() {
    local cmd
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='rm -f "$HOME/.config/tws/agents/$TMUX_PANE"; '
    cmd+='rm -rf "$HOME/.config/tws/subagents/$TMUX_PANE"; '
    cmd+='touch "$HOME/.config/tws/agent.trigger"'
    printf '[{"matcher": "", "hooks": [{"type": "command", "command": %s}]}]' \
        "$(printf '%s' "$cmd" | jq -Rs .)"
}

# Emits the SessionStart hook entry that records <session_id>\t<cwd> for this
# pane, so `tws fork-pane` can fork the session that runs in it.
fork_pointer_entry() {
    local cmd
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='input=$(cat); '
    cmd+='id=$(printf "%s" "$input" | jq -r ".session_id // empty"); '
    cmd+='[ -z "$id" ] && exit 0; '
    cmd+='cwd=$(printf "%s" "$input" | jq -r ".cwd // empty"); '
    cmd+='[ -z "$cwd" ] && cwd=$PWD; '
    cmd+='mkdir -p "$HOME/.config/tws/sessions"; '
    cmd+='printf "%s\t%s\n" "$id" "$cwd" > "$HOME/.config/tws/sessions/$TMUX_PANE"; :'
    printf '[{"matcher": "", "hooks": [{"type": "command", "command": %s}]}]' \
        "$(printf '%s' "$cmd" | jq -Rs .)"
}

# The end-of-session counterpart: drops this pane's fork pointer.
fork_pointer_end_entry() {
    local cmd
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='rm -f "$HOME/.config/tws/sessions/$TMUX_PANE"; :'
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
    local e_prompt e_pretool e_question e_posttool e_notify e_idle e_stop e_compact e_fail e_end
    local e_substart e_substop
    # Submitting a prompt is the only event that starts a turn, so it is the only
    # unconditional route back to `working`.
    e_prompt=$(status_hook_entry working "")
    # Claude runs matching hooks in parallel, so keep these matchers disjoint.
    e_pretool=$(status_hook_entry working "^(?!AskUserQuestion$).*" tool)
    e_question=$(status_hook_entry waiting "^AskUserQuestion$")
    # The "turn resumed" signal, scoped to the question it answers. A match-all
    # PostToolUse would hand every background subagent the same power, and its
    # tool calls would repaint the pane the moment `Stop` set `review`.
    e_posttool=$(status_hook_entry working "^AskUserQuestion$")
    # `idle_prompt` is the real event name — Claude sends it 60s after the main
    # loop goes quiet. It is also the backstop that heals a pane no other hook
    # reached. `agent_needs_input`, the name used before, never existed.
    # It yields to a live subagent marker: the main loop is quiet then, but work
    # goes on in the pane.
    e_notify=$(status_hook_entry waiting "permission_prompt" alert)
    e_idle=$(status_hook_entry waiting "idle_prompt" idle_alert)
    e_stop=$(status_hook_entry review "" stop)
    # Compaction and API errors end a turn without firing Stop.
    e_compact=$(status_hook_entry review "manual|auto")
    e_fail=$(status_hook_entry review "" stop)
    e_substart=$(subagent_hook_entry start)
    e_substop=$(subagent_hook_entry stop)
    e_end=$(session_end_hook_entry)
    local e_forkptr e_forkptr_end
    e_forkptr=$(fork_pointer_entry)
    e_forkptr_end=$(fork_pointer_end_entry)

    jq \
        --argjson prompt "$e_prompt" \
        --argjson pretool "$e_pretool" \
        --argjson question "$e_question" \
        --argjson posttool "$e_posttool" \
        --argjson notify "$e_notify" \
        --argjson idle "$e_idle" \
        --argjson substart "$e_substart" \
        --argjson substop "$e_substop" \
        --argjson stop "$e_stop" \
        --argjson compact "$e_compact" \
        --argjson fail "$e_fail" \
        --argjson end "$e_end" \
        --argjson forkptr "$e_forkptr" \
        --argjson forkptrend "$e_forkptr_end" '
        # A tws hook entry is identified by the config/tws/ path in its command.
        def is_tws: (.hooks // []) | any((.command // "") | test("config/tws/"));
        .hooks //= {} |
        # Strip any prior tws entries (of any version/shape) from every event array,
        # leaving non-tws hooks untouched. Makes re-runs idempotent.
        .hooks |= with_entries(.value |= (if type == "array" then map(select(is_tws | not)) else . end)) |
        # Append the current, correct tws entries.
        .hooks.UserPromptSubmit = ((.hooks.UserPromptSubmit // []) + $prompt) |
        .hooks.PreToolUse       = ((.hooks.PreToolUse // []) + $pretool + $question) |
        .hooks.PostToolUse      = ((.hooks.PostToolUse // []) + $posttool) |
        .hooks.Notification     = ((.hooks.Notification // []) + $notify + $idle) |
        .hooks.Stop             = ((.hooks.Stop // []) + $stop) |
        .hooks.SubagentStart    = ((.hooks.SubagentStart // []) + $substart) |
        .hooks.SubagentStop     = ((.hooks.SubagentStop // []) + $substop) |
        .hooks.SessionStart     = ((.hooks.SessionStart // []) + $forkptr) |
        .hooks.PostCompact      = ((.hooks.PostCompact // []) + $compact) |
        .hooks.StopFailure      = ((.hooks.StopFailure // []) + $fail) |
        .hooks.SessionEnd       = ((.hooks.SessionEnd // []) + $end + $forkptrend) |
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
    local e_work e_pretool e_wait e_review e_compact e_end e_substart e_substop
    e_work=$(status_hook_entry working "")
    e_pretool=$(status_hook_entry working "" live)
    e_wait=$(status_hook_entry waiting "" alert)
    e_review=$(status_hook_entry review "" stop)
    e_substart=$(subagent_hook_entry start)
    e_substop=$(subagent_hook_entry stop)
    # Codex has no API-error event, so stale expiry is the only backstop there.
    e_compact=$(status_hook_entry review "manual|auto")
    e_end=$(session_end_hook_entry)

    jq \
        --argjson work "$e_work" --argjson pretool "$e_pretool" --argjson wait "$e_wait" \
        --argjson review "$e_review" --argjson compact "$e_compact" --argjson end "$e_end" \
        --argjson substart "$e_substart" --argjson substop "$e_substop" '
        # A tws hook entry is identified by the config/tws/ path in its command.
        def is_tws: (.hooks // []) | any((.command // "") | test("config/tws/"));
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
        .hooks.SubagentStart      = ((.hooks.SubagentStart // []) + $substart) |
        .hooks.SubagentStop       = ((.hooks.SubagentStop // []) + $substop) |
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

// Only $TMUX_PANE names the pane this agent runs in. Asking tmux instead
// answers with the current client's active pane, so an agent outside a pane
// would stamp its status onto whichever pane the user is watching.
function panePath(): string | undefined {
  const pane = process.env.TMUX_PANE;
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
    const path = panePath();
    if (path) writeWord(path, "working");
  });
  // Pi's only per-tool-call event, and so the only place a heartbeat can live.
  pi.on("tool_execution_start", async () => {
    const path = panePath();
    if (!path) return;
    const cur = readWord(path);
    // A tool call proves liveness, it does not start a turn. A pane resting in
    // review or waiting stays there; only an empty file is claimed.
    if (cur === "working") beat(path);
    else if (cur === undefined) writeWord(path, "working");
  });
  // Compaction can end a turn without agent_settled firing.
  pi.on("session_compact", async () => {
    const path = panePath();
    if (path) writeWord(path, "review");
  });
  pi.on("agent_settled", async () => {
    const path = panePath();
    if (path) writeWord(path, "review");
  });
  pi.on("session_shutdown", async () => {
    const path = panePath();
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
        rm -rf "$HOME/.config/tws/subagents" 2>/dev/null || true
        mkdir -p "$HOME/.config/tws"
        touch "$HOME/.config/tws/agent.trigger"
        info "Cleared stale agent status files"
    fi
}

# --- 5. Optional: tmux fork binding (experimental) ---

# Idempotent: drop any earlier marked block, then append the new one. The text
# goes back into the existing file, because mv would replace a symlinked
# ~/.tmux.conf with a plain file. The blank lines before a dropped block go
# with it, and a blank line separates the new block from your lines only when
# the file does not already end with one. So repeated runs add no blank lines.
rewrite_conf_block() {
    local conf="$1" marker="$2" stale="$3" block="$4" tmp
    tmp="$(mktemp)"
    MARKER="$marker" STALE="$stale" awk '
        index($0, ENVIRON["MARKER"]) || index($0, ENVIRON["STALE"]) { blanks = 0; next }
        /^[[:space:]]*$/ { held[++blanks] = $0; next }
        { for (i = 1; i <= blanks; i++) print held[i]; blanks = 0; print }
        END { for (i = 1; i <= blanks; i++) print held[i] }
    ' "$conf" > "$tmp"
    if [ -s "$tmp" ] && [ -n "$(tail -n 1 "$tmp" | tr -d '[:space:]')" ]; then
        printf '\n' >> "$tmp"
    fi
    printf '%s\n' "$block" >> "$tmp"
    cat "$tmp" > "$conf"
    rm -f "$tmp"
}

# tmux does not expand #{pane_id} in a split-window command, but run-shell
# expands it first, so the fork pane learns which pane is its parent.
FORK_BINDING='bind-key F run-shell "tmux split-window -h -l 45% -t #{pane_id} \"tws fork-pane #{pane_id}\""'
FORK_MARKER='# tws fork binding'
# Matches a bind or bind-key line that targets the plain key F, with any
# number of leading flags (e.g. "bind F ...", "bind-key -r F ...",
# "bind-key -r -T prefix F ..."). Anchored at the start of the line (after
# optional leading whitespace), so a commented-out line never matches.
FORK_KEY_PATTERN='^[[:space:]]*bind(-key)?[[:space:]]+(-[[:alnum:]]+[[:space:]]+|-T[[:space:]]+[^[:space:]]+[[:space:]]+)*F([[:space:]]|$)'
# A bind with -n, or with -T root, targets the ROOT key table, not the
# prefix table, so it can never collide with prefix+F. Lines that match
# FORK_KEY_PATTERN but also match this are excluded from the conflict check.
FORK_ROOT_TABLE_PATTERN='(^|[[:space:]])-n([[:space:]]|$)|-T[[:space:]]+root([[:space:]]|$)'

configure_fork_binding() {
    local conf="$HOME/.tmux.conf"

    if [ ! -f "$conf" ]; then
        info "No ~/.tmux.conf — skipping fork binding"
        return
    fi

    # hooks_configured turns 1 when any agent's hooks install succeeds, but the
    # SessionStart hook that prefix+F needs comes only from configure_claude_hooks.
    # We accept that looseness here: it matches how the rest of the installer
    # already reads this shared flag, and a false positive just offers a binding
    # that finds no session to fork, which is harmless.
    if [ "$hooks_configured" -ne 1 ]; then
        info "Claude Code agent hooks are not configured — prefix+F needs them to find a session to fork"
        info "Skipping fork binding. Re-run install and accept the Claude Code hooks step, then add it manually with:"
        printf '  %s\n' "$FORK_BINDING"
        return
    fi

    printf '%s' "Add tws fork binding (prefix+F) to ~/.tmux.conf? [EXPERIMENTAL] [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped fork binding — add it manually with:"
        printf '  %s\n' "$FORK_BINDING"
        return
    fi

    # A conflict can come from the live tmux server (already-loaded config)
    # or from the file text itself (added by hand but not yet sourced).
    # Either source counts. Our own previously written marker+binding lines
    # are excluded from the file check, so re-runs stay idempotent.
    local live_conflict=0 file_conflict=0
    # `list-keys -T prefix` only ever lists prefix-table bindings, so a -n /
    # -T root exclusion isn't needed here — those never show up in this table.
    if tmux list-keys -T prefix 2>/dev/null | grep -qE '^bind-key[[:space:]]+(-[[:alnum:]]+[[:space:]]+)*-T[[:space:]]+prefix[[:space:]]+F([[:space:]]|$)'; then
        live_conflict=1
    fi
    if grep -vF -e "$FORK_MARKER" -e "$FORK_BINDING" "$conf" \
        | grep -vE "$FORK_ROOT_TABLE_PATTERN" \
        | grep -qE "$FORK_KEY_PATTERN"; then
        file_conflict=1
    fi

    if { [ "$live_conflict" -eq 1 ] || [ "$file_conflict" -eq 1 ]; } \
        && ! grep -qF "$FORK_MARKER" "$conf"; then
        warn "prefix+F is already bound to something else — not overwriting"
        info "Add this manually under a different key if you want it:"
        printf '  %s\n' "$FORK_BINDING"
        return
    fi

    rewrite_conf_block "$conf" "$FORK_MARKER" "tws fork-pane" "$FORK_MARKER"$'\n'"$FORK_BINDING"
    ok "Added fork binding (prefix+F) — EXPERIMENTAL"
    info "Run: tmux source-file ~/.tmux.conf"
}

# --- 5b. Optional: tmux ack hooks ---

ACK_MARKER='# tws ack hooks'
# A fixed hook index makes a reload replace the tws entry. The -ga flags would
# add one more entry, and one more process, on each source-file.
ACK_HOOK_INDEX=89

# tmux splits the run-shell string with its own quoting and expands #{...} in
# it, so a path with one of these characters breaks the hook line. A broken
# line can stop tmux from loading the rest of ~/.tmux.conf.
ack_path_is_safe() {
    case "$1" in
        *[[:space:]\'\"\\\$\#\;]*) return 1 ;;
    esac
    return 0
}

# Prints the marked block. run-shell does not use your shell PATH, so the hook
# names the binary by its absolute path. Your own hooks at other indexes stay.
# tmux expands #{pane_id} before the shell runs, so tws learns the pane that
# the client lands on. The two window-pane-changed and session-window-changed
# hooks cover last-pane, kill-pane and kill-window, which fire none of the
# after-* hooks.
ack_hook_block() {
    local cmd="run-shell -b \"$INSTALL_DIR/$BINARY_NAME ack-pane #{pane_id}\""
    local hook
    printf '%s\n' "$ACK_MARKER"
    for hook in after-select-pane after-select-window client-session-changed \
        window-pane-changed session-window-changed; do
        printf "set-hook -g %s[%s] '%s'\n" "$hook" "$ACK_HOOK_INDEX" "$cmd"
    done
}

write_ack_hooks() {
    rewrite_conf_block "$1" "$ACK_MARKER" "tws ack-pane" "$(ack_hook_block)"
}

configure_ack_hooks() {
    local conf="$HOME/.tmux.conf"

    if [ ! -f "$conf" ]; then
        info "No ~/.tmux.conf — skipping ack hooks"
        return
    fi

    if [ "$hooks_configured" -ne 1 ]; then
        info "Agent hooks are not configured — the ack hooks have no status to clear"
        info "Skipping ack hooks. Re-run install and accept the agent hooks step, then add them manually with:"
        ack_hook_block | sed 's/^/  /'
        return
    fi

    if ! ack_path_is_safe "$INSTALL_DIR/$BINARY_NAME"; then
        warn "The binary path has a space or one of ' \" \\ \$ # ; — not writing the ack hooks"
        info "Put the binary at a path without these characters, then add the hooks manually with:"
        ack_hook_block | sed 's/^/  /'
        return
    fi

    printf '%s' "Mark a pane as read when you move into it with tmux, and add ack hooks to ~/.tmux.conf? [y/N] "
    read -r answer < /dev/tty
    if [[ ! "$answer" =~ ^[Yy]$ ]]; then
        info "Skipped ack hooks — add them manually with:"
        ack_hook_block | sed 's/^/  /'
        return
    fi

    write_ack_hooks "$conf"
    ok "Added ack hooks to ~/.tmux.conf"
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
    configure_ack_hooks
    configure_glow

    echo ""
    ok "Done!"
    echo "  Binary:   $INSTALL_DIR/$BINARY_NAME"
    echo "  Run:      tws"
    echo "  Detach:   prefix + d"
    echo ""
}

main
