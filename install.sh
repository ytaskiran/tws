#!/usr/bin/env bash
set -euo pipefail

REPO="ytaskiran/tws"
INSTALL_DIR="$HOME/.local/bin"
BINARY_NAME="tws"
tmpdir=""
hooks_configured=0
claude_configured=0
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
}

# Older tws saved threads inside collections. Flatten them into the plain
# thread array that tws reads now. Silent; does nothing on a new file or without jq.
migrate_state() {
    local f="$HOME/.config/tws/state.json"
    [[ -f "$f" ]] && command -v jq &>/dev/null || return 0
    jq -e '.[0] | has("threads")' "$f" &>/dev/null || return 0
    # Write through "$f", so a symlink and the file mode stay. `|| true` keeps
    # a failure from stopping the installer under `set -e`; .bak holds the data.
    { jq '[.[].threads[]]' "$f" > "$f.tmp" && cp "$f" "$f.bak" && cat "$f.tmp" > "$f"; } || true
    rm -f "$f.tmp"
}

PATH_EXPORT_LINE='export PATH="$HOME/.local/bin:$PATH"'

# Sets plan_rc and plan_profile for the user's shell, and leaves them empty for
# a shell the installer does not know. Two variables, not one split string,
# because $HOME can hold a space.
find_shell_rc_files() {
    case "$(basename "${SHELL:-}")" in
        zsh)  plan_rc="$HOME/.zshrc";  plan_profile="$HOME/.zprofile" ;;
        bash) plan_rc="$HOME/.bashrc"; plan_profile="$HOME/.bash_profile" ;;
    esac
}

configure_path() {
    local rc_file="$1" profile_file="$2" export_line="$PATH_EXPORT_LINE"
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
# force a full tmux+ps rescan. Every status write goes through `put`, because `>`
# truncates before writing and exposes an empty file to concurrent readers. `put`
# writes a dot temp file in the same directory, then runs `mv -f`. A reader sees
# the old word or the new word, never an empty file, and `live` mode cannot claim
# a file that is only mid-write.
#
# Every other hook writes the status file only when the word changes, so its mtime
# is the state entry time. `prompt` is the exception: it always writes, so for a
# `working` pane the mtime is the start of the turn. A tool call in a `working`
# pane touches the heartbeat file
# `heartbeat/$TMUX_PANE` instead (`live`, `tool` and `begin` do this). tws reads
# the newer of the two mtimes to find a silent pane.
#
# Modes:
#   set    unconditional — the event names the new state outright.
#   prompt UserPromptSubmit (Claude and Codex). It always writes the word, so the
#          rename stamps a fresh mtime. After an Esc there is no Stop and no
#          interrupt hook: the file still says `working` with the old turn's
#          mtime, and a tool call touches only the heartbeat, so the new turn
#          would show the old turn's age. It rings the trigger only if the word
#          changed. It also removes the pane's permission keys: a new prompt
#          means the user answered every request of the last turn. A denied tool,
#          or an Esc at the dialog, aborts the turn without firing Stop, so this
#          is the only event that clears its key.
#   live   liveness only — touch the heartbeat of a `working` pane, or claim an
#          empty file. A pane in a resting state stays there. Background subagents share the pane with the
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
#          The same jq call reads the tool_use_id. The hook then makes the in-flight
#          marker `inflight/$TMUX_PANE/<m|s>.<tool_use_id>` (`s` for a subagent). A
#          tool call that runs for hours sends no heartbeat, and tws reads the
#          marker as proof that the pane still works.
#   stop   turn end (Stop, StopFailure, and PostCompact for a manual /compact).
#          While a fresh subagent marker exists it writes `working`, but keeps
#          `waiting`: a background subagent can hold the pane there for a
#          permission prompt. Without a marker it writes the word, or `idle` if
#          the user is looking at the pane. tmux answers with three flags:
#          pane_active, window_active and session_attached. The pane is in view
#          when the first two are 1 and the third is 1 or more. If the query
#          fails, the answer is the word. tmux does not know if the terminal has
#          focus, so a pane in a background terminal counts as in view. Also
#          deletes stale markers and the permission key files. It deletes the
#          `m.*` in-flight markers and keeps `s.*`: background subagents work on
#          after the main loop ends its turn.
#          With the fourth argument `keep` (Codex) it deletes no in-flight marker.
#          Codex tool hooks carry no `agent_id`, so a Codex subagent call gets the
#          `m` prefix too, and Stop cannot tell it from a main-loop call. Deleting
#          it would let tws expire the pane after 15 minutes while the subagent's
#          tool still runs. A Codex marker ends at its PostToolUse, at Interrupt, at
#          SessionEnd, or at the 4 h cap.
#   idle_alert  `alert` for `idle_prompt`, skipped while a fresh marker exists.
#   reset  Claude SessionStart. A new conversation in the pane owns nothing of the
#          last one, so it writes the word (`idle`) over any state and deletes the
#          pane's subagent markers, permission keys, in-flight markers and
#          heartbeat. It rings the trigger only if the word changed.
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
#          it hashes anything. A denied tool fires neither event, and an interrupt
#          fires no Stop: `prompt` clears the keys at the next prompt. It also
#          removes the in-flight marker of the call. The one jq call gives the
#          tool_use_id and the key input, so this mode always starts jq.
#   interrupt Codex Interrupt. An ESC interrupt fires no Stop, so this event ends the
#          turn. It writes the word (`idle`) over any state, and rings the trigger
#          only if the word changed. The turn gave no result, so there is nothing
#          to review. This also ends an approval wait: Codex PermissionRequest
#          uses `alert` and writes no key file. It removes the pane's subagent
#          markers, so the next Stop does not see a fresh marker of a stopped
#          subagent and write `working`. If a subagent survives, its next
#          PostToolUse repaints `working` within seconds. It also deletes the
#          pane's in-flight markers, because a running tool dies with the turn. It
#          starts no jq and no tmux, because the hook timeout is 1 s.
#   begin  Codex PreToolUse. Its payload has a tool_use_id, so it works as `live`
#          and also makes the marker `m.<tool_use_id>`. Codex has no `agent_id` in
#          the documented payload, so every marker gets the `m` prefix.
#   done   Codex PostToolUse. It works as `set` and also removes the marker of the
#          call.
#   rest   Codex SessionStart. Codex also fires it when a subagent starts, and that
#          must not end the turn of the main loop. It changes `review` or an empty
#          file to the word (`idle`). It leaves `working`, `waiting` and `idle`.
#          It keeps the markers.
status_hook_entry() {
    local word="$1"
    local matcher="$2"      # "" for match-all
    local mode="${3:-set}"
    local keep="${4:-}"     # `stop` only: "keep" leaves the in-flight markers
    local cmd trig
    trig='touch "$HOME/.config/tws/agent.trigger"'
    cmd='[ -n "${TMUX_PANE:-}" ] || exit 0; '
    cmd+='f="$HOME/.config/tws/agents/$TMUX_PANE"; '
    cmd+='mkdir -p "$HOME/.config/tws/agents"; '
    cmd+='put() { t="$HOME/.config/tws/agents/.$TMUX_PANE.$$"; printf %s "$1" > "$t" && mv -f "$t" "$f" || rm -f "$t"; }; '
    cmd+='cur=$(cat "$f" 2>/dev/null); '
    cmd+='sd="$HOME/.config/tws/subagents/$TMUX_PANE"; '
    cmd+='pd="$HOME/.config/tws/permissions/$TMUX_PANE"; '
    cmd+='ifd="$HOME/.config/tws/inflight/$TMUX_PANE"; '
    cmd+='hb="$HOME/.config/tws/heartbeat/$TMUX_PANE"; '
    # A tool_use_id names a file, so only [A-Za-z0-9_-]+ is safe. The modes decode
    # the id in two ways (`@tsv` in `tool`, a `-c` JSON string in `granted`), and
    # they agree only on this alphabet: a quote or a control character would make a
    # marker that `granted` never removes. `mark` makes the marker of a call (prefix
    # in $p). `unmark` removes it: a PostToolUse payload does not say which prefix
    # made the marker, and the id is unique, so it tries both. `LC_ALL=C` keeps the
    # ranges to ASCII: some shells (macOS sh) let `A-Z` match accented letters.
    # A tool call in a working pane touches the heartbeat file. It leaves the status
    # file alone, so that file keeps the state entry time. The plain touch makes the
    # file, and the directory only when the first touch fails.
    local beat='touch "$hb" 2>/dev/null || { mkdir -p "$HOME/.config/tws/heartbeat" && touch "$hb"; }'
    local badid='""|*[!A-Za-z0-9_-]*'
    local mark="LC_ALL=C; case \"\$tid\" in $badid) ;; *) mkdir -p \"\$ifd\" 2>/dev/null && touch \"\$ifd/\$p.\$tid\" 2>/dev/null ;; esac; "
    local unmark="LC_ALL=C; case \"\$tid\" in $badid) ;; *) rm -f \"\$ifd/m.\$tid\" \"\$ifd/s.\$tid\" ;; esac; "
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
            # One jq call gives both ids, split on a tab. A failed jq leaves no id
            # to mark.
            cmd+='tab=$(printf "\t"); '
            cmd+='if ids=$(jq -r "[.agent_id // \"\", .tool_use_id // \"\"] | @tsv" 2>/dev/null); '
            cmd+='then aid=${ids%%"$tab"*}; tid=${ids#*"$tab"}; else aid=.unknown; tid=; fi; '
            cmd+='if [ -n "$aid" ]; then p=s; else p=m; fi; '
            cmd+="$mark"
            cmd+='if [ -n "$aid" ]; then touch -c "$sd/$aid" 2>/dev/null; '
            cmd+="if [ \"\$cur\" = $word ]; then $beat; "
            cmd+="elif [ -z \"\$cur\" ]; then put $word; $trig; fi; "
            cmd+='else case "$cur" in '
            cmd+="$word) $beat ;; waiting) ;; "
            cmd+="*) put $word; $trig ;; esac; fi; :"
            ;;
        stop)
            cmd+='rm -rf "$pd"; '
            [ "$keep" = keep ] || cmd+='rm -f "$ifd"/m.* 2>/dev/null; '
            cmd+="find \"\$sd\" -type f ! -mmin -$SUBAGENT_FRESH_MINS -delete 2>/dev/null; "
            cmd+="if $fresh; then case \"\$cur\" in waiting) w=waiting ;; *) w=working ;; esac; "
            cmd+="else w=$word; $seen fi; "
            cmd+="[ \"\$cur\" != \"\$w\" ] && { put \"\$w\"; $trig; }; :"
            ;;
        reset)
            cmd+="if [ \"\$cur\" = working ] && $fresh; then :; else "
            cmd+='rm -rf "$sd" "$pd" "$ifd" "$hb"; '
            cmd+="put $word; [ \"\$cur\" = $word ] || { $trig; }; fi; :"
            ;;
        prompt)
            cmd+='rm -rf "$pd"; '
            cmd+="put $word; [ \"\$cur\" = $word ] || { $trig; }; :"
            ;;
        permit)
            cmd+="$keyof"
            cmd+='if [ -n "${k:-}" ] && mkdir -p "$pd"; then : > "$pd/$k"; fi; '
            cmd+="if [ \"\$cur\" = working ] || [ -z \"\$cur\" ]; then put $word; $trig; fi; :"
            ;;
        granted)
            # One jq call gives the tool_use_id on its first line and the key input
            # on its second. The id is a JSON string here, so it loses its quotes.
            cmd+='nl=$(printf "\nx"); nl=${nl%x}; '
            cmd+='out=$(jq -cS "(.tool_use_id // \"\"), {tool_name, tool_input}" 2>/dev/null); j=; tid=; '
            cmd+='case "$out" in *"$nl"*) tid=${out%%"$nl"*}; tid=${tid#\"}; tid=${tid%\"}; j=${out#*"$nl"} ;; esac; '
            cmd+="$unmark"
            cmd+='[ -n "$(ls -A "$pd" 2>/dev/null)" ] || exit 0; '
            cmd+='k=; [ -n "$j" ] && k=$(printf %s "$j" | cksum | tr " " -); '
            cmd+='[ -n "$k" ] && [ -e "$pd/$k" ] || exit 0; rm -f "$pd/$k"; '
            cmd+='[ "$cur" = waiting ] && [ -z "$(ls -A "$pd" 2>/dev/null)" ] '
            cmd+="&& { put $word; $trig; }; :"
            ;;
        interrupt)
            cmd+='rm -rf "$sd" "$ifd"; '
            cmd+="[ \"\$cur\" != $word ] && { put $word; $trig; }; :"
            ;;
        begin)
            cmd+='tid=$(jq -r ".tool_use_id // empty" 2>/dev/null) || tid=; p=m; '
            cmd+="$mark"
            cmd+="if [ \"\$cur\" = $word ]; then $beat; "
            cmd+="elif [ -z \"\$cur\" ]; then put $word; $trig; fi; :"
            ;;
        done)
            cmd+='tid=$(jq -r ".tool_use_id // empty" 2>/dev/null) || tid=; '
            cmd+="$unmark"
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
            cmd+="if [ \"\$cur\" = $word ]; then $beat; "
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
    cmd+='rm -rf "$HOME/.config/tws/inflight/$TMUX_PANE"; '
    cmd+='rm -f "$HOME/.config/tws/heartbeat/$TMUX_PANE"; '
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

    # jq reads an empty file as no input and writes nothing back.
    [ -s "$settings" ] || echo '{}' > "$settings"

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

    # An `&&` list does not trip `set -e`, so a jq failure (a settings.json it
    # cannot parse) must be caught here, or the installer reports success and
    # writes the tmux steps for hooks that do not exist.
    if jq \
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
    ' "$settings" > "$tmp"; then
        mv "$tmp" "$settings"
        ok "Configured Claude Code agent status hooks"
        hooks_configured=1
        claude_configured=1
    else
        rm -f "$tmp"
        warn "Could not update $settings (jq failed — is it valid JSON?). Claude Code hooks are not changed."
    fi
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
        info "Install jq, then re-run install, or add hooks manually"
        return
    fi

    [ -s "$hooks_file" ] || echo '{}' > "$hooks_file"

    local tmp
    tmp="$(mktemp)"
    local e_work e_pretool e_posttool e_wait e_review e_compact e_end e_substart e_substop e_sessionstart e_interrupt
    # A new prompt stamps the turn start, as it does for Claude. Codex has no
    # permission key files, so the `rm -rf "$pd"` in `prompt` removes nothing.
    e_work=$(status_hook_entry working "" prompt)
    # Both tool hooks carry a tool_use_id, so a long call keeps an in-flight marker.
    e_pretool=$(status_hook_entry working "" begin)
    e_posttool=$(status_hook_entry working "" done)
    e_wait=$(status_hook_entry waiting "" alert)
    e_review=$(status_hook_entry review "" stop keep)
    e_substart=$(subagent_hook_entry start)
    e_substop=$(subagent_hook_entry stop)
    # Interrupt covers an ESC. Codex has no API-error event, so stale expiry still
    # covers an API error and a hard kill.
    # Compaction follows the Claude rule: PostCompact ends a manual /compact, and
    # PreCompact writes nothing because a cancelled /compact has no exit event. An
    # auto compaction is ended by the Stop of its turn.
    e_compact=$(status_hook_entry review "manual" stop keep)
    e_end=$(session_end_hook_entry)
    e_sessionstart=$(status_hook_entry idle "$SESSION_START_MATCHER" rest)
    e_interrupt=$(status_hook_entry idle "" interrupt)

    # See configure_claude_hooks: a jq failure must not count as success.
    if jq \
        --argjson work "$e_work" --argjson pretool "$e_pretool" --argjson posttool "$e_posttool" --argjson wait "$e_wait" \
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
        .hooks.PostToolUse        = ((.hooks.PostToolUse // []) + $posttool) |
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
    ' "$hooks_file" > "$tmp"; then
        mv "$tmp" "$hooks_file"
        ok "Configured Codex agent status hooks"
        hooks_configured=1
        configure_codex_feature_flag
    else
        rm -f "$tmp"
        warn "Could not update $hooks_file (jq failed — is it valid JSON?). Codex hooks are not changed."
    fi
}

configure_pi_hooks() {
    local ext_dir="$HOME/.pi/agent/extensions"
    local ext_file="$ext_dir/tws-status.ts"

    if [ ! -d "$HOME/.pi" ]; then
        info "Pi config not found — skipping agent hooks"
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
const HEARTBEAT_DIR = `${process.env.HOME}/.config/tws/heartbeat`;
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

// The heartbeat file is how a working pane proves liveness to tws. It is a file
// of its own: the status file keeps the state entry time, which tws shows as the
// turn age. A tool call makes the file when it is absent.
function beat(pane: string) {
  const now = new Date();
  try {
    utimesSync(`${HEARTBEAT_DIR}/${pane}`, now, now);
  } catch {
    try {
      mkdirSync(HEARTBEAT_DIR, { recursive: true });
      writeFileSync(`${HEARTBEAT_DIR}/${pane}`, "");
    } catch {
      // A heartbeat that cannot be written must not break the tool call.
    }
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
    const pane = process.env.TMUX_PANE;
    if (!path || !pane) return;
    const cur = readWord(path);
    // A tool call proves liveness, it does not start a turn. A pane resting in
    // review or waiting stays there; only an empty file is claimed.
    if (cur === "working") beat(pane);
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
    if (!path) return;
    rmSync(`${HEARTBEAT_DIR}/${process.env.TMUX_PANE}`, { force: true });
    if (existsSync(path)) {
      rmSync(path, { force: true });
      writeFileSync(TRIGGER, "");
    }
  });
}
PI_EXT_EOF

    ok "Configured Pi agent status hooks"
    hooks_configured=1
}

# Configures the agents that scan_plan found. The user already approved them
# with the single plan question, so nothing here asks.
configure_agent_hooks() {
    if [ "$plan_claude" -eq 1 ]; then configure_claude_hooks; fi
    if [ "$plan_codex" -eq 1 ]; then configure_codex_hooks; fi
    if [ "$plan_pi" -eq 1 ]; then configure_pi_hooks; fi

    # Agents snapshot hook config at session start, so a file already stuck at
    # `working` would outlive this upgrade. Live panes rewrite theirs on the next
    # hook fire.
    if [ "$hooks_configured" -eq 1 ]; then
        rm -f "$HOME"/.config/tws/agents/* 2>/dev/null || true
        rm -rf "$HOME/.config/tws/subagents" 2>/dev/null || true
        rm -rf "$HOME/.config/tws/permissions" 2>/dev/null || true
        rm -rf "$HOME/.config/tws/inflight" 2>/dev/null || true
        rm -rf "$HOME/.config/tws/heartbeat" 2>/dev/null || true
        mkdir -p "$HOME/.config/tws"
        touch "$HOME/.config/tws/agent.trigger"
        info "Cleared stale agent status files"
    fi
}

# --- 5. tmux config ---

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
    # Fails on a read-only config (a Nix store link, for example), so the caller
    # can warn and skip and the installer goes on.
    if ! cat "$tmp" > "$conf" 2>/dev/null; then
        rm -f "$tmp"
        return 1
    fi
    rm -f "$tmp"
}

# The config files that tmux reads. tmux loads each one that exists, in this
# order, so a check for the user's own keys must read all of them. With
# XDG_CONFIG_HOME unset, two entries name the same file, so repeats are dropped.
tmux_conf_candidates() {
    printf '%s\n' "$HOME/.tmux.conf" \
        "${XDG_CONFIG_HOME:-$HOME/.config}/tmux/tmux.conf" \
        "$HOME/.config/tmux/tmux.conf" | awk '!seen[$0]++'
}

# Prints the first config file that exists, or nothing. The tws blocks go into
# this one file.
tmux_conf_path() {
    local conf
    while IFS= read -r conf; do
        if [ -f "$conf" ]; then
            printf '%s\n' "$conf"
            return
        fi
    done <<< "$(tmux_conf_candidates)"
}

# Prints a dangling symlink that comes before any real config file, for example
# a dotfiles link whose repo is not cloned yet. A write through it fails, so the
# tmux steps must not run.
tmux_conf_broken_link() {
    local conf
    while IFS= read -r conf; do
        if [ -f "$conf" ]; then return; fi
        if [ -L "$conf" ]; then
            printf '%s\n' "$conf"
            return
        fi
    done <<< "$(tmux_conf_candidates)"
}

# Succeeds when a config loads other files or runs plugins (source-file, run),
# also inside if-shell. With no tmux server running, the keys that those bind
# cannot be seen without starting a server, and starting one runs the user's
# config. Comments do not count, and neither do key bindings: the common
# `bind r source-file ~/.tmux.conf` runs only when the key is pressed. The tws
# blocks do not count either: the ack hooks hold `run-shell`, and a re-run must
# see the same config as the first run.
tmux_conf_loads_more() {
    local conf
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        if grep -vE -e '^[[:space:]]*(#|(un)?bind(-key)?[[:space:]])' -e 'tws (ack|fork)-pane' "$conf" \
            | grep -E "(^|[[:space:]'\"{;])(source(-file)?|run(-shell)?)([[:space:]'\"]|$)" >/dev/null; then
            return 0
        fi
    done <<< "$(tmux_conf_candidates)"
    return 1
}

# Like tmux_conf_path, but creates ~/.tmux.conf when no config exists, so the
# blocks below always have a file to go into. Fails when it cannot create it.
tmux_conf_for_write() {
    local conf
    conf="$(tmux_conf_path)"
    if [ -z "$conf" ]; then
        conf="$HOME/.tmux.conf"
        : > "$conf" 2>/dev/null || return 1
    fi
    printf '%s\n' "$conf"
}

# Most tmux commands start a server when none runs, and a new server loads and
# runs the user's config (plugin managers, session restore). list-sessions does
# not start one, so it is the only safe probe. scan_plan sets tmux_live from it
# once, and every later tmux call checks tmux_live first.
tmux_live=0
tmux_server_runs() {
    tmux list-sessions >/dev/null 2>&1
}

# Loads a block into a running tmux server, so no source-file step is needed.
# Only the block is sourced: a second source of the whole config can repeat the
# user's own commands.
load_into_tmux() {
    local block="$1" tmp status
    [ "$tmux_live" -eq 1 ] || return 1
    tmp="$(mktemp)"
    printf '%s\n' "$block" > "$tmp"
    tmux source-file "$tmp" 2>/dev/null
    status=$?
    rm -f "$tmp"
    return "$status"
}

# --- 5a. tmux ack hooks (part of the agent hooks) ---

ACK_MARKER='# tws ack hooks'
# A fixed hook index makes a reload replace the tws entry. The -ga flags would
# add one more entry, and one more process, on each source-file.
ACK_HOOK_INDEX=89

# tmux splits the run-shell string with its own quoting and expands #{...} in
# it, so a path with one of these characters breaks the hook line. A broken
# line can stop tmux from loading the rest of the config.
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

# The hooks only clear the `review` state that the agent hooks write, so they
# come with that step. They make a pane you move into with plain tmux count as
# read. scan_plan already left out an unsafe binary path. hooks_configured is
# still checked: it stays 0 when no agent's hooks could be written (for example
# a settings.json that jq cannot parse), and then the tmux steps have no use.
configure_ack_hooks() {
    [ "$hooks_configured" -eq 1 ] || return 0

    local conf
    if ! conf="$(tmux_conf_for_write)"; then
        warn "Could not create $HOME/.tmux.conf — skipping this tmux step"
        return
    fi

    if ! write_ack_hooks "$conf"; then
        warn "Could not write $conf — skipping the tmux ack hooks"
        return
    fi
    ok "Added tmux ack hooks to $conf (a pane you move into counts as read)"
    if load_into_tmux "$(ack_hook_block)"; then
        ok "Loaded them into the running tmux server"
    fi
}

# --- 5b. tmux fork binding (experimental, part of the agent hooks) ---

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

# scan_plan plans this step only with the Claude hooks, because prefix+F needs
# their SessionStart session pointer. claude_configured tells whether the
# Claude hooks were really written.
configure_fork_binding() {
    [ "$claude_configured" -eq 1 ] || return 0

    local conf
    if ! conf="$(tmux_conf_for_write)"; then
        warn "Could not create $HOME/.tmux.conf — skipping this tmux step"
        return
    fi

    if fork_key_taken; then
        warn "prefix+F is already bound to something else — not overwriting"
        info "Add this manually under a different key if you want it:"
        printf '  %s\n' "$FORK_BINDING"
        return
    fi

    if ! rewrite_conf_block "$conf" "$FORK_MARKER" "tws fork-pane" "$FORK_MARKER"$'\n'"$FORK_BINDING"; then
        warn "Could not write $conf — skipping the fork binding"
        return
    fi
    ok "Added fork binding (prefix+F) to $conf — EXPERIMENTAL"
    if load_into_tmux "$FORK_BINDING"; then
        ok "Loaded it into the running tmux server"
    fi
}

# Succeeds when prefix+F is bound to something that is not the tws binding.
# A conflict can come from the live tmux server (already-loaded config) or from
# the text of any config file that tmux loads (added by hand but not yet
# sourced). A line or binding with `tws fork-pane` is ours, so re-runs stay
# idempotent, and a prefix+F that the user adds after an install still counts.
#
# The pipelines end in `grep … >/dev/null`, not `grep -q`: under pipefail, a
# `grep -q` that exits at its first match sends SIGPIPE to the writer, and the
# pipeline then fails although grep matched.
fork_key_taken() {
    # `list-keys -T prefix` only ever lists prefix-table bindings, so a -n /
    # -T root exclusion isn't needed here — those never show up in this table.
    if [ "$tmux_live" -eq 1 ] && tmux list-keys -T prefix 2>/dev/null \
        | grep -vF 'tws fork-pane' \
        | grep -E '^bind-key[[:space:]]+(-[[:alnum:]]+[[:space:]]+)*-T[[:space:]]+prefix[[:space:]]+F([[:space:]]|$)' >/dev/null; then
        return 0
    fi
    local conf
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        if grep -vF -e "$FORK_MARKER" -e 'tws fork-pane' "$conf" \
            | grep -vE "$FORK_ROOT_TABLE_PATTERN" \
            | grep -E "$FORK_KEY_PATTERN" >/dev/null; then
            return 0
        fi
    done <<< "$(tmux_conf_candidates)"
    return 1
}

# --- 5c. tmux status bar (part of the agent hooks) ---

BAR_MARKER='# tws status bar'
# A config line with this text keeps the installer from adding the bar again.
BAR_OFF='# tws status bar off'
# Marks the replace form of the nova right side, so a re-run can tell an
# earlier yes from the default `tws` line.
BAR_REPLACE_MARKER='# tws status bar replace'
BAR_OPTIONS='status-style status-bg status-fg status-justify status-left status-left-style status-right status-right-style status-format window-status-format window-status-current-format window-status-style window-status-current-style'
# bar_replace is 1 when the right side of the nova bar is only tws: after a
# yes, or kept from an earlier yes. bar_backup is 1 only after a new yes.
bar_replace=0
bar_backup=0

# The window tabs show the agent glyphs of their panes, and the right end shows
# "thread › session". #() jobs get the tmux server environment, not your shell
# PATH, so the block names the binary by its absolute path. Every line carries
# the marker, so a re-run drops the block and keeps your own lines, also a line
# of yours that calls `tws bar`.
# The tmux default bar is green, and the green working glyph does not show on
# it, so the block also sets the tws palette bg and fg. The block goes after
# your lines, so it leaves out status-interval and status-right-length when
# you set them. After a yes to confirm_nova_right, it raises a
# status-right-length below 80 to 80. The nova form needs both options too:
# nova sets neither, and the tmux default right length of 40 cuts the tws
# segment. --since gives the server start time: an older status file is from
# a pane of an earlier server with the same ID. Both tab formats run the same
# command, and its output picks the colors for the current tab, so a window
# change starts no new job. The tabs keep the zoom and bell flags.
bar_block() {
    local bin="$INSTALL_DIR/$BINARY_NAME" nova len
    printf '%s\n' "$BAR_MARKER"
    tmux_conf_sets status-interval || printf '%s\n' "set -g status-interval 5  $BAR_MARKER"
    len="$(user_conf_lines | sed -nE "s/^.*status-right-length[[:space:]]+['\"]?([0-9]+).*$/\1/p" | tail -n 1)"
    if ! tmux_conf_sets status-right-length \
        || { [ "$bar_replace" -eq 1 ] && [ -n "$len" ] && [ "$len" -lt 80 ]; }; then
        printf '%s\n' "set -g status-right-length 80  $BAR_MARKER"
    fi
    if nova="$(nova_script)"; then
        nova_block "$bin" "$nova"
        return
    fi
    printf '%s\n' \
        "set -g status-style 'bg=#1e1e1e,fg=#d4d4d4'  $BAR_MARKER" \
        "set -g status-left ' '  $BAR_MARKER" \
        "set -g window-status-format ' #I #W#{s/[*-]//:window_flags}#($bin bar window --since #{start_time} #{P:#{pane_id} }) '  $BAR_MARKER" \
        "set -g window-status-current-format '#[bg=#c88e68,fg=#121212] #I #W#{s/[*-]//:window_flags}#($bin bar window --since #{start_time} #{P:#{pane_id} }) #[default]'  $BAR_MARKER" \
        "set -g status-right '#[bg=#c88e68,fg=#121212] #($bin bar where -- #{q:session_name}) '  $BAR_MARKER"
}

# Succeeds when the user has a status bar of their own: a config line sets one
# of BAR_OPTIONS, or, with a live server, one of them differs from the tmux
# default (a plugin such as nova sets them at runtime). A value from a tws
# block does not count, also from an older block or another install path, so
# a re-run sees the same result. A theme added after the install still
# counts. The defaults come from a probe server that reads no config and
# exits at once.
tmux_has_own_bar() {
    local conf opt live ours default pattern block
    pattern="(^|[[:space:]'\"])($(printf '%s' "$BAR_OPTIONS" | tr ' ' '|'))([[:space:]'\"[]|$)"
    if user_conf_lines | grep -E "$pattern" >/dev/null; then
        return 0
    fi
    [ "$tmux_live" -eq 1 ] || return 1
    block="$(bar_block)"
    for opt in $BAR_OPTIONS; do
        live="$(tmux show-options -gv "$opt" 2>/dev/null)"
        case "$live" in *" bar window "* | *" bar where "*) continue ;; esac
        ours="$(printf '%s\n' "$block" | sed -n "s/^set -g $opt '\(.*\)'.*/\1/p")"
        [ -z "$ours" ] || [ "$live" != "$ours" ] || continue
        default="$(tmux -L tws-defaults -f /dev/null start-server \; show-options -gv "$opt" 2>/dev/null)"
        [ "$live" = "$default" ] || return 0
    done
    return 1
}

# Prints the tmux-nova plugin script when a config line loads the plugin and
# the script is there. With nova, the bar goes on top of your nova bar.
nova_script() {
    local conf dir
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        if grep -vE '^[[:space:]]*#' "$conf" | grep -E "@plugin[[:space:]]+['\"]o0th/tmux-nova['\"]" >/dev/null; then
            for dir in "${TMUX_PLUGIN_MANAGER_PATH:-$HOME/.tmux/plugins}" "${XDG_CONFIG_HOME:-$HOME/.config}/tmux/plugins"; do
                if [ -f "${dir%/}/tmux-nova/nova.tmux" ]; then
                    printf '%s\n' "${dir%/}/tmux-nova/nova.tmux"
                    return 0
                fi
            done
        fi
    done <<< "$(tmux_conf_candidates)"
    return 1
}

# The nova form of the block: the glyphs go at the end of each tab label, and
# "thread › session" is one more segment at the right end. nova reads its
# options only when it runs, and the block comes after `run tpm`, so the
# block runs nova again. An option you set gets the tws part appended; an
# option you do not set gets the nova default plus the tws part. A part that
# your own lines already have (the README recipe, for example) is left out.
# After a yes to confirm_nova_right, the right side is only the tws segment.
# The current tab gets the tws color when you set none.
nova_block() {
    local bin="$1" nova="$2" tab
    tab="#($bin bar window --since #{start_time} #{P:#{pane_id} })"
    if tmux_conf_has '@nova-pane[[:space:]].* bar window '; then
        :
    elif tmux_conf_sets @nova-pane; then
        printf '%s\n' "set -ga @nova-pane '$tab'  $BAR_MARKER"
    else
        printf '%s\n' "set -g @nova-pane '#S:#I:#W$tab'  $BAR_MARKER"
    fi
    if [ "$bar_replace" -eq 1 ]; then
        printf '%s\n' "set -g @nova-segments-0-right 'tws'  $BAR_REPLACE_MARKER"
    elif tmux_conf_has '@nova-segments-0-right[[:space:]].*tws'; then
        :
    elif tmux_conf_sets @nova-segments-0-right; then
        printf '%s\n' "set -ga @nova-segments-0-right ' tws'  $BAR_MARKER"
    else
        printf '%s\n' "set -g @nova-segments-0-right 'tws'  $BAR_MARKER"
    fi
    if ! tmux_conf_has '@nova-segment-tws[[:space:]]'; then
        printf '%s\n' \
            "set -g @nova-segment-tws '#($bin bar where -- #{q:session_name})'  $BAR_MARKER" \
            "set -g @nova-segment-tws-colors '#c88e68 #121212'  $BAR_MARKER"
    fi
    tmux_conf_sets @nova-status-style-active-bg \
        || printf '%s\n' "set -g @nova-status-style-active-bg '#c88e68'  $BAR_MARKER"
    printf '%s\n' "run-shell '$nova'  $BAR_MARKER"
}

# Prints your config lines: not the tws block, not a comment. With no
# argument it reads every config that tmux loads, else only the files given.
# `|| :` keeps a file with no such line from failing a caller's pipeline
# under pipefail.
user_conf_lines() {
    local conf files
    if [ "$#" -gt 0 ]; then files="$(printf '%s\n' "$@")"; else files="$(tmux_conf_candidates)"; fi
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        grep -vF "$BAR_MARKER" "$conf" | grep -vE '^[[:space:]]*#' || :
    done <<< "$files"
}

# Succeeds when a config line of yours matches the extended regex $1.
tmux_conf_has() {
    user_conf_lines | grep -E "$1" >/dev/null
}

# The block for a running server. A server that already has the tws part in
# a nova option skips that line, so a re-run does not append it twice. The
# replace form of the right side (set -g) always goes through.
bar_block_live() {
    local skip=""
    case "$(tmux show-options -gv @nova-pane 2>/dev/null)" in *" bar window "*) skip="@nova-pane " ;; esac
    case "$(tmux show-options -gv @nova-segments-0-right 2>/dev/null)" in *tws*) skip="$skip@nova-segments-0-right " ;; esac
    bar_block | while IFS= read -r line; do
        case "$line" in
            "set -ga @nova-segments-0-right "*) case "$skip" in *"@nova-segments-0-right "*) continue ;; esac ;;
            *" @nova-pane "*) case "$skip" in *"@nova-pane "*) continue ;; esac ;;
        esac
        printf '%s\n' "$line"
    done
}

# Succeeds when a config line of yours sets the option $1.
tmux_conf_sets() {
    user_conf_lines | grep -E "(^|[[:space:]])$1([[:space:]]|\$)" >/dev/null
}

NOVA_RIGHT_PATTERN='(^|[[:space:]])(@nova-segments-0-right|status-right-length)([[:space:]]|$)'

# Prints your config lines that set the right side of a nova bar, so the
# question can show them. The file arguments work as in user_conf_lines.
nova_right_lines() {
    user_conf_lines "$@" | grep -E "$NOVA_RIGHT_PATTERN" | sed 's/^[[:space:]]*//' || :
}

# Succeeds when your nova right side has a segment that is not tws. A line
# with only tws (also the ' tws' of an append) does not count.
nova_right_has_more() {
    nova_right_lines | grep -E '@nova-segments-0-right' \
        | grep -vE "@nova-segments-0-right[[:space:]]+['\"]?[[:space:]]*tws[[:space:]]*['\"]?[[:space:]]*(#.*)?$" >/dev/null
}

# Succeeds when the tws block already holds the replace form, so a re-run
# keeps the earlier yes with no new question and no new backup.
nova_right_replaced() {
    local conf
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        if grep -F "$BAR_REPLACE_MARKER" "$conf" >/dev/null; then
            return 0
        fi
    done <<< "$(tmux_conf_candidates)"
    return 1
}

# Asked during the scan, only for a nova bar whose right side has segments of
# yours. No terminal counts as no, so a piped install keeps your right side.
confirm_nova_right() {
    local answer="" line
    echo ""
    echo "tws status bar — your tmux-nova bar has its own right side. These are your lines now:"
    while IFS= read -r line; do printf '    %s\n' "$line"; done <<< "$(nova_right_lines)"
    tmux_conf_sets status-right-length || echo "    (no status-right-length line: tmux uses 40)"
    echo "  y = replace: the right side shows only tws (thread › session),"
    echo "      and status-right-length becomes at least 80. tws backs up your tmux config first."
    echo "  n = keep: tws adds its segment after yours, and keeps your status-right-length."
    printf '%s' "Replace the right side of your nova bar with the tws version? [y/N] "
    if ! read -r answer 2>/dev/null < /dev/tty; then
        echo ""
        return 1
    fi
    [[ "$answer" =~ ^[Yy] ]]
}

# Before the tws bar replaces your nova right side, copies to
# ~/.config/tws/backups the config that tws writes ($1) and each config that
# holds your right-side lines, and tells how to restore each one. The backup
# name keeps the path, so ~/.tmux.conf and ~/.config/tmux/tmux.conf do not
# collide.
backup_conf() {
    local dir="$HOME/.config/tws/backups" stamp conf name backup done_list=""
    stamp="$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$dir" 2>/dev/null || return 1
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        [ "$conf" = "$1" ] || [ -n "$(nova_right_lines "$conf")" ] || continue
        case "$done_list" in *"|$conf|"*) continue ;; esac
        done_list="$done_list|$conf|"
        name="${conf#"$HOME"/}"
        name="$(printf '%s' "${name#.}" | tr '/' '_')"
        backup="$dir/$name.$stamp"
        cp "$conf" "$backup" 2>/dev/null || return 1
        ok "Backed up $conf to $backup"
        info "To restore it: cp '$backup' '$conf', then restart tmux"
    done <<< "$(printf '%s\n' "$1"; tmux_conf_candidates)"
}

# Succeeds when a config holds the opt-out line.
tmux_conf_bar_off() {
    local conf
    while IFS= read -r conf; do
        [ -f "$conf" ] || continue
        if grep -F "$BAR_OFF" "$conf" >/dev/null; then
            return 0
        fi
    done <<< "$(tmux_conf_candidates)"
    return 1
}

configure_status_bar() {
    [ "$hooks_configured" -eq 1 ] || return 0

    local conf
    if ! conf="$(tmux_conf_for_write)"; then
        warn "Could not create $HOME/.tmux.conf — skipping this tmux step"
        return
    fi

    if [ "$bar_backup" -eq 1 ] && ! backup_conf "$conf"; then
        warn "Could not back up $conf — skipping the status bar"
        return
    fi
    if ! rewrite_conf_block "$conf" "$BAR_MARKER" "$BAR_MARKER" "$(bar_block)"; then
        warn "Could not write $conf — skipping the status bar"
        return
    fi
    ok "Added the tws status bar to $conf (agents on each window tab)"
    if load_into_tmux "$(bar_block_live)"; then
        ok "Loaded it into the running tmux server"
    fi
}

# --- 6. glow (rich markdown rendering) ---

GLOW_GO_PKG='github.com/charmbracelet/glow@latest'

install_glow() {
    case "$plan_glow_via" in
        brew)
            info "Installing glow via Homebrew..."
            brew install glow && ok "glow installed" || warn "glow installation failed — notes will use basic rendering"
            ;;
        go)
            info "Installing glow via Go..."
            go install "$GLOW_GO_PKG" && ok "glow installed" || warn "glow installation failed — notes will use basic rendering"
            ;;
    esac
}

# --- 7. Scan, plan, and the questions ---

# The scan only reads. It records what the installer can change in plan_*
# flags, and what it found but leaves alone in plan_notes. The user then
# answers one question for the whole plan. Before it, the scan can ask about
# a status bar of the user's own (confirm_own_bar, confirm_nova_right).
plan_path=0 plan_claude=0 plan_codex=0 plan_pi=0 plan_ack=0 plan_fork=0 plan_bar=0 plan_glow=0
plan_rc="" plan_profile="" plan_conf="" plan_glow_via=""
plan_found=()
plan_notes=()

# Succeeds for a JSON object or an empty file; the apply step turns an empty
# file into {}. `jq empty` alone is not enough: it accepts an empty file (and
# the filter then writes the empty file back) and a non-object such as [].
json_object_or_empty() {
    [ ! -s "$1" ] || jq -e 'type == "object"' "$1" >/dev/null 2>&1
}

# Shows a path with ~ for $HOME, so the plan stays short.
tilde() {
    local home_mark="~"
    printf '%s\n' "${1/#$HOME/$home_mark}"
}

scan_plan() {
    local have_jq=0
    command -v jq &>/dev/null && have_jq=1
    if tmux_server_runs; then tmux_live=1; fi

    case ":$PATH:" in
        *":$INSTALL_DIR:"*) ;;
        *)
            find_shell_rc_files
            if [ -n "$plan_rc" ]; then
                # A past run already added the line; this shell only has not read it.
                if grep -q '$HOME/.local/bin' "$plan_rc" 2>/dev/null \
                    && grep -q '$HOME/.local/bin' "$plan_profile" 2>/dev/null; then
                    plan_notes+=("$(tilde "$INSTALL_DIR") is in $(tilde "$plan_rc") — restart your shell to use it")
                else
                    plan_path=1
                fi
            else
                plan_notes+=("$(tilde "$INSTALL_DIR") is not on PATH — add this to your shell profile: $PATH_EXPORT_LINE")
            fi
            ;;
    esac

    # A file that is not a JSON object goes under "Not changed", so the plan never
    # lists a step that the apply step then cannot do.
    local settings="$HOME/.claude/settings.json" codex_hooks="$HOME/.codex/hooks.json"
    if [ -f "$settings" ]; then
        plan_found+=("Claude Code")
        if [ "$have_jq" -eq 0 ]; then
            plan_notes+=("Claude Code hooks need jq — install jq, then run install again")
        elif ! json_object_or_empty "$settings"; then
            plan_notes+=("Claude Code hooks — $(tilde "$settings") is not a JSON object; fix it, then run install again")
        else
            plan_claude=1
        fi
    fi
    if [ -d "$HOME/.codex" ]; then
        plan_found+=("Codex")
        if [ "$have_jq" -eq 0 ]; then
            plan_notes+=("Codex hooks need jq — install jq, then run install again")
        elif [ -f "$codex_hooks" ] && ! json_object_or_empty "$codex_hooks"; then
            plan_notes+=("Codex hooks — $(tilde "$codex_hooks") is not a JSON object; fix it, then run install again")
        else
            plan_codex=1
        fi
    fi
    if [ -d "$HOME/.pi" ]; then
        plan_found+=("Pi")
        plan_pi=1
    fi

    plan_conf="$(tmux_conf_path)"
    if [ -n "$plan_conf" ]; then
        plan_found+=("tmux config $(tilde "$plan_conf")")
    fi

    # The tmux steps only make sense next to agent hooks: the ack hooks clear the
    # review state that they write, and prefix+F needs the Claude session pointer.
    # A note that starts with a tab is a detail line under the note before it.
    local broken_link tmux_blocked=0
    broken_link="$(tmux_conf_broken_link)"
    if [ $((plan_claude + plan_codex + plan_pi)) -gt 0 ]; then
        if [ -n "$broken_link" ]; then
            plan_notes+=("tmux ack hooks, fork binding, and status bar — $(tilde "$broken_link") is a broken symlink; fix it, then run install again")
            tmux_blocked=1
        elif [ -n "$plan_conf" ] && [ ! -w "$plan_conf" ]; then
            plan_notes+=("tmux ack hooks, fork binding, and status bar — $(tilde "$plan_conf") is not writable (a read-only or managed file)")
            tmux_blocked=1
        fi
    fi
    if [ $((plan_claude + plan_codex + plan_pi)) -gt 0 ] && [ "$tmux_blocked" -eq 0 ]; then
        if ack_path_is_safe "$INSTALL_DIR/$BINARY_NAME"; then
            plan_ack=1
        else
            plan_notes+=("tmux ack hooks — the binary path has a space or one of ' \" \\ \$ # ;")
            plan_notes+=($'\t'"Put the binary at a path without these characters, then add these to your tmux config:")
            local line
            while IFS= read -r line; do plan_notes+=($'\t'"  $line"); done <<< "$(ack_hook_block)"
        fi
    fi
    # The bar shows what the agent hooks write, so it comes with the ack hooks.
    if [ "$plan_ack" -eq 1 ] && ! tmux_conf_bar_off; then
        if nova_script >/dev/null; then
            if nova_right_replaced; then
                bar_replace=1
            elif nova_right_has_more && confirm_nova_right; then
                bar_replace=1 bar_backup=1
            fi
            plan_bar=1
        elif [ "$tmux_live" -eq 0 ] && tmux_conf_loads_more; then
            plan_notes+=("tmux status bar — your tmux config loads other files or plugins, and their bar")
            plan_notes+=($'\t'"cannot be checked with no tmux server running. Start tmux, then run install again.")
        elif tmux_has_own_bar && ! confirm_own_bar; then
            plan_notes+=("tmux status bar — your tmux config has its own bar. To show the agents on it, see")
            plan_notes+=($'\t'"https://github.com/ytaskiran/tws#status-bar")
        else
            plan_bar=1
        fi
    fi
    if [ "$plan_claude" -eq 1 ] && [ "$tmux_blocked" -eq 0 ]; then
        if fork_key_taken; then
            plan_notes+=("tmux fork binding — prefix+F is already bound to something else")
            plan_notes+=($'\t'"To use it on another key, change F in this line and add it to your tmux config:")
            plan_notes+=($'\t'"  $FORK_BINDING")
        elif [ "$tmux_live" -eq 0 ] && tmux_conf_loads_more; then
            plan_notes+=("tmux fork binding — your tmux config loads other files or plugins, and their prefix+F keys")
            plan_notes+=($'\t'"cannot be checked with no tmux server running. Start tmux, then run install again.")
        else
            plan_fork=1
        fi
    fi

    if ! command -v glow &>/dev/null; then
        if command -v brew &>/dev/null; then
            plan_glow=1 plan_glow_via=brew
        elif command -v go &>/dev/null; then
            plan_glow=1 plan_glow_via=go
        else
            plan_notes+=("glow is missing and neither brew nor go is available — notes use basic rendering")
        fi
    fi
}

print_plan() {
    local conf_label row
    conf_label="$(tilde "${plan_conf:-$HOME/.tmux.conf}")"
    [ -n "$plan_conf" ] || conf_label="$conf_label (new file)"

    echo ""
    if [ "${#plan_found[@]}" -gt 0 ]; then
        info "Found: $(IFS=,; printf '%s' "${plan_found[*]}" | sed 's/,/, /g')"
    else
        info "Found no agent and no tmux config"
    fi

    if plan_is_empty; then
        info "Nothing else to set up"
    else
        info "tws will set up or update:"
        if [ "$plan_claude" -eq 1 ]; then plan_row "Claude Code status hooks" "$(tilde "$HOME/.claude/settings.json")"; fi
        if [ "$plan_codex" -eq 1 ]; then plan_row "Codex status hooks" "$(tilde "$HOME/.codex/hooks.json"), $(tilde "$HOME/.codex/config.toml")"; fi
        if [ "$plan_pi" -eq 1 ]; then plan_row "Pi status extension" "$(tilde "$HOME/.pi/agent/extensions/tws-status.ts")"; fi
        if [ "$plan_ack" -eq 1 ]; then plan_row "tmux ack hooks" "$conf_label"; fi
        if [ "$plan_fork" -eq 1 ]; then plan_row "tmux fork binding (prefix+F)" "$conf_label  [experimental]"; fi
        if [ "$plan_bar" -eq 1 ] && [ "$bar_replace" -eq 1 ]; then
            plan_row "tmux status bar (replaces right)" "$conf_label$([ "$bar_backup" -eq 0 ] || printf '  [backup first]')"
        elif [ "$plan_bar" -eq 1 ]; then
            plan_row "tmux status bar (agents on tabs)" "$conf_label"
        fi
        if [ "$plan_path" -eq 1 ]; then plan_row "add $(tilde "$INSTALL_DIR") to PATH" "$(tilde "$plan_rc"), $(tilde "$plan_profile")"; fi
        if [ "$plan_glow" -eq 1 ]; then
            if [ "$plan_glow_via" = brew ]; then plan_row "install glow" "brew install glow"; else plan_row "install glow" "go install $GLOW_GO_PKG"; fi
        fi
    fi

    if [ "${#plan_notes[@]}" -gt 0 ]; then
        info "Not changed:"
        for row in "${plan_notes[@]}"; do
            case "$row" in
                $'\t'*) printf '     %s\n' "${row#$'\t'}" ;;
                *) printf '   • %s\n' "$row" ;;
            esac
        done
    fi
}

# One line of the plan: what changes, and where.
plan_row() {
    printf '   • %-32s %s\n' "$1" "$2"
}

plan_is_empty() {
    [ $((plan_path + plan_claude + plan_codex + plan_pi + plan_ack + plan_fork + plan_bar + plan_glow)) -eq 0 ]
}

# Asked during the scan, only when the user has a bar of their own. The tws
# block goes after their lines, so a yes replaces their bar; the default is no.
# No terminal counts as no.
confirm_own_bar() {
    local answer=""
    printf '%s' "Your tmux config has its own status bar. Replace it with the tws bar? [y/N] "
    if ! read -r answer 2>/dev/null < /dev/tty; then
        echo ""
        return 1
    fi
    [[ "$answer" =~ ^[Yy] ]]
}

# The main question. No terminal (a piped install with no tty) counts as no, so
# such a run changes nothing outside the binary.
confirm_plan() {
    local answer=""
    printf '%s' "Apply these changes? [Y/n] "
    if ! read -r answer 2>/dev/null < /dev/tty; then
        echo ""
        return 1
    fi
    [[ ! "$answer" =~ ^[Nn] ]]
}

apply_plan() {
    if [ "$plan_path" -eq 1 ]; then configure_path "$plan_rc" "$plan_profile"; fi
    configure_agent_hooks
    if [ "$plan_ack" -eq 1 ]; then configure_ack_hooks; fi
    if [ "$plan_fork" -eq 1 ]; then configure_fork_binding; fi
    if [ "$plan_bar" -eq 1 ]; then configure_status_bar; fi
    if [ "$plan_glow" -eq 1 ]; then install_glow; fi
}

# After a "no", show the user what to add by hand. The agent hooks are left out:
# they are long JSON, and running install again is the way to get them.
print_manual_steps() {
    if [ $((plan_path + plan_ack + plan_fork + plan_bar + plan_glow)) -eq 0 ]; then
        info "No changes made. Run install again to set them up."
        return
    fi
    info "No changes made. Run install again to set them up, or add these by hand:"
    if [ "$plan_path" -eq 1 ]; then
        echo "  In $(tilde "$plan_rc") and $(tilde "$plan_profile"):"
        echo "    $PATH_EXPORT_LINE"
    fi
    if [ $((plan_ack + plan_fork + plan_bar)) -gt 0 ]; then
        echo "  In $(tilde "${plan_conf:-$HOME/.tmux.conf}"):"
        if [ "$plan_ack" -eq 1 ]; then ack_hook_block | sed 's/^/    /'; fi
        if [ "$plan_fork" -eq 1 ]; then echo "    $FORK_BINDING"; fi
        if [ "$plan_bar" -eq 1 ]; then bar_block | sed 's/^/    /'; fi
    fi
    if [ "$plan_glow" -eq 1 ]; then
        if [ "$plan_glow_via" = brew ]; then echo "  Install glow: brew install glow"; else echo "  Install glow: go install $GLOW_GO_PKG"; fi
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
    migrate_state

    scan_plan
    print_plan
    if ! plan_is_empty; then
        if confirm_plan; then
            apply_plan
        else
            print_manual_steps
        fi
    fi

    echo ""
    ok "Done!"
    echo "  Binary:   $INSTALL_DIR/$BINARY_NAME"
    echo "  Run:      tws"
    echo "  Detach:   prefix + d"
    echo ""
}

main
