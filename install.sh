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
# The SessionStart sources that begin a new conversation. `compact` fires in the
# middle of a session, and `fork` does not start a new one, so neither may reset.
SESSION_START_MATCHER='startup|resume|clear'

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
# truncates before writing, exposing an empty file to concurrent readers. For the
# same reason every status write goes through `put`: a dot temp file in the same
# directory, then `mv -f`. A reader sees the old word or the new word, never an
# empty file, and `live` mode cannot claim a file that is only mid-write.
#
# Modes:
#   set    unconditional — the event names the new state outright.
#   prompt UserPromptSubmit (Claude). The same write as `set`. It also removes the
#          pane's permission keys: a new prompt means the user answered every
#          request of the last turn. A denied tool, or an Esc at the dialog, aborts
#          the turn without firing Stop, so this is the only event that clears
#          its key.
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
#   stop   turn end (Stop, StopFailure, and PostCompact for a manual /compact).
#          While a fresh subagent marker exists it writes `working`, but keeps
#          `waiting`: a background subagent can hold the pane there for a
#          permission prompt. Without a marker it writes the word, or `idle` if
#          the user is looking at the pane. tmux answers with three flags:
#          pane_active, window_active and session_attached. The pane is in view
#          when the first two are 1 and the third is 1 or more. If the query
#          fails, the answer is the word. tmux does not know if the terminal has
#          focus, so a pane in a background terminal counts as in view. Also
#          deletes stale markers and the permission key files.
#   idle_alert  `alert` for `idle_prompt`, skipped while a fresh marker exists.
#   reset  Claude SessionStart. A new conversation in the pane owns nothing of the
#          last one, so it writes the word (`idle`) over any state and deletes the
#          pane's subagent markers and permission keys. It rings the trigger only
#          if the word changed.
#          Exception: if the word is `working` and a fresh marker exists, it does
#          nothing. A nested `claude -p` that the pane's agent runs inherits
#          TMUX_PANE and fires SessionStart in the middle of a turn.
#   permit PermissionRequest (Claude). Records the request as a key file, then raises
#          `waiting` with the `alert` rules. The key is a checksum of the tool name
#          and input, because the request carries no tool_use_id. Claude gives the
#          same name and input to PostToolUse when the tool runs after a grant.
#   granted PostToolUse and PostToolUseFailure (Claude). The tool ran, so its
#          request was answered. It removes the key of that call, and writes
#          `working` if the pane waits and no other request is open. A payload with
#          no key file changes nothing. A pane with no open request exits before
#          it starts jq. A denied tool fires neither event, and an interrupt fires no
#          Stop: `prompt` clears the keys at the next prompt.
#   interrupt Codex Interrupt. An ESC interrupt fires no Stop, so this event ends the
#          turn. It writes the word (`idle`) over any state, and rings the trigger
#          only if the word changed. The turn gave no result, so there is nothing
#          to review. It deletes the pane's permission keys, because a pending
#          approval dies with the turn. It keeps the subagent markers: the docs do
#          not say that an interrupt stops subagents. It starts no jq and no tmux,
#          because the hook timeout is 1 s.
#   rest   Codex SessionStart. Codex also fires it when a subagent starts, and that
#          must not end the turn of the main loop. It changes `review` or an empty
#          file to the word (`idle`). It leaves `working`, `waiting` and `idle`.
#          It keeps the markers.
status_hook_entry() {
    local word="$1"
    local matcher="$2"      # "" for match-all
    local mode="${3:-set}"
    local cmd trig
    trig='touch "$HOME/.config/tws/agent.trigger"'
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='f="$HOME/.config/tws/agents/$TMUX_PANE"; '
    cmd+='mkdir -p "$HOME/.config/tws/agents"; '
    cmd+='put() { t="$HOME/.config/tws/agents/.$TMUX_PANE.$$"; printf %s "$1" > "$t" && mv -f "$t" "$f" || rm -f "$t"; }; '
    cmd+='cur=$(cat "$f" 2>/dev/null); '
    cmd+='sd="$HOME/.config/tws/subagents/$TMUX_PANE"; '
    cmd+='pd="$HOME/.config/tws/permissions/$TMUX_PANE"; '
    # One jq call gives the whole key input. cksum is POSIX, and shasum is not on
    # every Linux. The size joins the checksum to make a collision less likely.
    local keyof='k=; j=$(jq -cS "{tool_name, tool_input}" 2>/dev/null); '
    keyof+='[ -n "$j" ] && k=$(printf %s "$j" | cksum | tr " " -); '
    local fresh="[ -n \"\$(find \"\$sd\" -type f -mmin -$SUBAGENT_FRESH_MINS 2>/dev/null | head -n 1)\" ]"
    # Only the -t "$TMUX_PANE" form is allowed: it asks about the caller's own pane.
    local seen='v=$(tmux display-message -p -t "$TMUX_PANE" "#{pane_active}#{window_active}#{session_attached}" 2>/dev/null); '
    seen+='case "$v" in 11[1-9]*) w=idle ;; esac; '
    case "$mode" in
        tool)
            # A failed jq (missing, bad JSON) cannot prove this is the main loop,
            # so it takes the conservative subagent path.
            cmd+='if ! aid=$(jq -r ".agent_id // empty" 2>/dev/null); then aid=.unknown; fi; '
            cmd+='if [ -n "$aid" ]; then touch -c "$sd/$aid" 2>/dev/null; '
            cmd+="if [ \"\$cur\" = $word ]; then touch -c \"\$f\"; "
            cmd+="elif [ -z \"\$cur\" ]; then put $word; $trig; fi; "
            cmd+='else case "$cur" in '
            cmd+="$word) touch -c \"\$f\" ;; waiting) ;; "
            cmd+="*) put $word; $trig ;; esac; fi; :"
            ;;
        stop)
            cmd+='rm -rf "$pd"; '
            cmd+="find \"\$sd\" -type f ! -mmin -$SUBAGENT_FRESH_MINS -delete 2>/dev/null; "
            cmd+="if $fresh; then case \"\$cur\" in waiting) w=waiting ;; *) w=working ;; esac; "
            cmd+="else w=$word; $seen fi; "
            cmd+="[ \"\$cur\" != \"\$w\" ] && { put \"\$w\"; $trig; }; :"
            ;;
        reset)
            cmd+="if [ \"\$cur\" = working ] && $fresh; then :; else "
            cmd+='rm -rf "$sd" "$pd"; '
            cmd+="put $word; [ \"\$cur\" = $word ] || { $trig; }; fi; :"
            ;;
        prompt)
            cmd+='rm -rf "$pd"; '
            cmd+="[ \"\$cur\" != $word ] && { put $word; $trig; }; :"
            ;;
        permit)
            cmd+="$keyof"
            cmd+='if [ -n "${k:-}" ] && mkdir -p "$pd"; then : > "$pd/$k"; fi; '
            cmd+="if [ \"\$cur\" = working ] || [ -z \"\$cur\" ]; then put $word; $trig; fi; :"
            ;;
        granted)
            cmd+='[ -n "$(ls -A "$pd" 2>/dev/null)" ] || exit 0; '
            cmd+="$keyof"
            cmd+='[ -n "${k:-}" ] && [ -e "$pd/$k" ] || exit 0; rm -f "$pd/$k"; '
            cmd+='[ "$cur" = waiting ] && [ -z "$(ls -A "$pd" 2>/dev/null)" ] '
            cmd+="&& { put $word; $trig; }; :"
            ;;
        interrupt)
            cmd+='rm -rf "$pd"; '
            cmd+="[ \"\$cur\" != $word ] && { put $word; $trig; }; :"
            ;;
        rest)
            cmd+="case \"\$cur\" in ''|review) put $word; $trig ;; esac; :"
            ;;
        idle_alert)
            cmd+="if $fresh; then :; "
            cmd+="elif [ \"\$cur\" = working ] || [ -z \"\$cur\" ]; then put $word; $trig; fi; :"
            ;;
        live)
            cmd+="if [ \"\$cur\" = $word ]; then touch -c \"\$f\"; "
            cmd+="elif [ -z \"\$cur\" ]; then put $word; $trig; fi; :"
            ;;
        alert)
            cmd+="if [ \"\$cur\" = working ] || [ -z \"\$cur\" ]; then put $word; $trig; fi; :"
            ;;
        *)
            cmd+="[ \"\$cur\" != $word ] && { put $word; $trig; }; :"
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
    cmd+='rm -rf "$HOME/.config/tws/permissions/$TMUX_PANE"; '
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
    local e_substart e_substop e_sessionstart e_permit e_granted e_granted_fail
    # Submitting a prompt is the only event that starts a turn, so it is the only
    # unconditional route back to `working`.
    e_prompt=$(status_hook_entry working "" prompt)
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
    # A grant is the exit from the `waiting` that a permission request enters. The
    # request has no tool_use_id, so a key from the tool name and input pairs the
    # grant with its request. The Notification above stays as the backstop. The
    # matcher is disjoint from the question entry: Claude runs matches in parallel.
    # PermissionRequest fires for AskUserQuestion too, and no hook removes that
    # key, so permit skips it exactly as granted does.
    e_permit=$(status_hook_entry waiting "^(?!AskUserQuestion\$).*" permit)
    e_granted=$(status_hook_entry working "^(?!AskUserQuestion\$).*" granted)
    e_granted_fail=$(status_hook_entry working "" granted)
    e_idle=$(status_hook_entry waiting "idle_prompt" idle_alert)
    e_stop=$(status_hook_entry review "" stop)
    # A manual /compact fires no UserPromptSubmit and no Stop, so PostCompact ends
    # it. There is no PreCompact write: a /compact that is cancelled, fails or is
    # blocked fires no PostCompact, so a `working` from PreCompact would have no
    # exit. An auto compaction happens in a turn, and the Stop of that turn ends
    # it, so it has no hook. StopFailure is the API error event.
    e_compact=$(status_hook_entry review "manual" stop)
    e_fail=$(status_hook_entry review "" stop)
    e_substart=$(subagent_hook_entry start)
    e_substop=$(subagent_hook_entry stop)
    e_end=$(session_end_hook_entry)
    e_sessionstart=$(status_hook_entry idle "$SESSION_START_MATCHER" reset)
    local e_forkptr e_forkptr_end
    e_forkptr=$(fork_pointer_entry)
    e_forkptr_end=$(fork_pointer_end_entry)

    jq \
        --argjson prompt "$e_prompt" \
        --argjson pretool "$e_pretool" \
        --argjson question "$e_question" \
        --argjson posttool "$e_posttool" \
        --argjson notify "$e_notify" \
        --argjson permit "$e_permit" \
        --argjson granted "$e_granted" \
        --argjson grantedfail "$e_granted_fail" \
        --argjson idle "$e_idle" \
        --argjson substart "$e_substart" \
        --argjson substop "$e_substop" \
        --argjson stop "$e_stop" \
        --argjson compact "$e_compact" \
        --argjson fail "$e_fail" \
        --argjson end "$e_end" \
        --argjson sessionstart "$e_sessionstart" \
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
        .hooks.PostToolUse      = ((.hooks.PostToolUse // []) + $posttool + $granted) |
        .hooks.PostToolUseFailure = ((.hooks.PostToolUseFailure // []) + $grantedfail) |
        .hooks.PermissionRequest = ((.hooks.PermissionRequest // []) + $permit) |
        .hooks.Notification     = ((.hooks.Notification // []) + $notify + $idle) |
        .hooks.Stop             = ((.hooks.Stop // []) + $stop) |
        .hooks.SubagentStart    = ((.hooks.SubagentStart // []) + $substart) |
        .hooks.SubagentStop     = ((.hooks.SubagentStop // []) + $substop) |
        .hooks.SessionStart     = ((.hooks.SessionStart // []) + $forkptr + $sessionstart) |
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
    local e_work e_pretool e_wait e_review e_compact e_end e_substart e_substop e_sessionstart e_interrupt
    e_work=$(status_hook_entry working "")
    e_pretool=$(status_hook_entry working "" live)
    e_wait=$(status_hook_entry waiting "" alert)
    e_review=$(status_hook_entry review "" stop)
    e_substart=$(subagent_hook_entry start)
    e_substop=$(subagent_hook_entry stop)
    # Interrupt covers an ESC. Codex has no API-error event, so stale expiry still
    # covers an API error and a hard kill.
    # Compaction follows the Claude rule: PostCompact ends a manual /compact, and
    # PreCompact writes nothing because a cancelled /compact has no exit event. An
    # auto compaction is ended by the Stop of its turn.
    e_compact=$(status_hook_entry review "manual" stop)
    e_end=$(session_end_hook_entry)
    e_sessionstart=$(status_hook_entry idle "$SESSION_START_MATCHER" rest)
    e_interrupt=$(status_hook_entry idle "" interrupt)

    jq \
        --argjson work "$e_work" --argjson pretool "$e_pretool" --argjson wait "$e_wait" \
        --argjson review "$e_review" --argjson compact "$e_compact" \
        --argjson end "$e_end" \
        --argjson substart "$e_substart" --argjson substop "$e_substop" \
        --argjson sessionstart "$e_sessionstart" --argjson interrupt "$e_interrupt" '
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
        # Codex fires SessionStart for subagents too, so `rest` never overwrites a live state.
        .hooks.SessionStart       = ((.hooks.SessionStart // []) + $sessionstart) |
        .hooks.PostCompact        = ((.hooks.PostCompact // []) + $compact) |
        .hooks.SessionEnd         = ((.hooks.SessionEnd // []) + $end) |
        # An ESC interrupt fires Interrupt and no Stop; Claude Code has no such hook.
        .hooks.Interrupt          = ((.hooks.Interrupt // []) + $interrupt) |
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
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { basename } from "node:path";

const AGENTS_DIR = `${process.env.HOME}/.config/tws/agents`;
const TRIGGER = `${process.env.HOME}/.config/tws/agent.trigger`;

// Only $TMUX_PANE names the pane this agent runs in. Asking tmux instead
// answers with the current client's active pane, so an agent outside a pane
// would stamp its status onto whichever pane the user is watching.
function panePath(): string | undefined {
  const pane = process.env.TMUX_PANE;
  return pane ? `${AGENTS_DIR}/${pane}` : undefined;
}

// A turn that ends in the pane the user is looking at is already read. tmux
// answers three flags for the pane: pane_active, window_active and
// session_attached. Any failure means the pane is not known to be in view. The
// query names the pane with -t, so it reads this pane and not the active one.
// tmux does not know if the terminal has focus: a pane in a background terminal
// counts as in view.
function turnEndWord(pane: string): string {
  try {
    const seen = execFileSync("tmux", ["display-message", "-p", "-t", pane, "#{pane_active}#{window_active}#{session_attached}"], {
      encoding: "utf8",
      timeout: 1000,
      stdio: ["ignore", "pipe", "ignore"],
    });
    return /^11[1-9]/.test(seen.trim()) ? "idle" : "review";
  } catch {
    return "review";
  }
}

function settle() {
  const pane = process.env.TMUX_PANE;
  const path = panePath();
  if (pane && path) writeWord(path, turnEndWord(pane));
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
  // Truncate-then-write would expose an empty file to a reader in between.
  const tmp = `${AGENTS_DIR}/.${basename(path)}.${process.pid}`;
  try {
    writeFileSync(tmp, word);
    renameSync(tmp, path);
  } catch (err) {
    rmSync(tmp, { force: true });
    throw err;
  }
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
  // A new conversation in the pane owns nothing of the last one. `reload` keeps
  // the same session, so it must not reset a pane that waits for the user.
  pi.on("session_start", async (event: { reason: string }) => {
    const path = panePath();
    if (path && event.reason !== "reload") writeWord(path, "idle");
  });
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
  // agent_settled waits for the "threshold" and "overflow" compactions, and they
  // happen in a turn, so they must not end it here. Only a manual /compact needs
  // its own turn-end write.
  pi.on("session_compact", async (event: { reason: string }) => {
    if (event.reason === "manual") settle();
  });
  pi.on("agent_settled", async () => {
    settle();
  });
  // `reload` keeps the session, and its session_start skips the reset. Deleting
  // the file here would lose the pane's word anyway.
  pi.on("session_shutdown", async (event: { reason: string }) => {
    if (event.reason === "reload") return;
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
        rm -rf "$HOME/.config/tws/permissions" 2>/dev/null || true
        mkdir -p "$HOME/.config/tws"
        touch "$HOME/.config/tws/agent.trigger"
        info "Cleared stale agent status files"
    fi
}

# --- 5. Optional: tmux fork binding (experimental) ---

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

    # Idempotent: drop any previously marked block before re-adding.
    if grep -qF "$FORK_MARKER" "$conf"; then
        local tmp
        tmp="$(mktemp)"
        # grep exits 1 (no error) when the filter matches nothing, which
        # would abort the script under set -o pipefail if chained with &&.
        grep -vF -e "$FORK_MARKER" -e "tws fork-pane" "$conf" > "$tmp" || true
        mv "$tmp" "$conf"
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
