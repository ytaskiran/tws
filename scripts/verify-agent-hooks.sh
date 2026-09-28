#!/usr/bin/env bash
#
# Drives the Claude hook commands that install.sh generates against a throwaway
# HOME, and asserts the resulting status word. Run it from the repository root
# after any change to status_hook_entry or the Claude hook wiring:
#
#     bash scripts/verify-agent-hooks.sh
#
# Requires bash and jq, the same tools install.sh needs.
set -euo pipefail

cd "$(dirname "$0")/.."
# An explicit path lets you point the harness at another revision's install.sh,
# which is how you confirm a check still catches the bug it was written for.
INSTALL_SH="${1:-install.sh}"
eval "$(sed -n '/^SUBAGENT_FRESH_MINS=/p;/^SESSION_START_MATCHER=/p;/^status_hook_entry()/,/^}/p;/^session_end_hook_entry()/,/^}/p;/^subagent_hook_entry()/,/^}/p;/^fork_pointer_entry()/,/^}/p;/^fork_pointer_end_entry()/,/^}/p;/^configure_claude_hooks()/,/^}/p;/^configure_codex_feature_flag()/,/^}/p;/^configure_codex_hooks()/,/^}/p' "$INSTALL_SH" | sed 's# < /dev/tty##')"

export HOME
HOME="$(mktemp -d)"
export TMUX_PANE="%7"
STATUS_FILE="$HOME/.config/tws/agents/%7"
trap 'rm -rf "$HOME"' EXIT

failures=0

# A tmux that answers a bare pane query with a pane the caller does not own — the
# real one answers for the current client's active pane, which is the same
# thing from the hook's point of view. A query scoped with -t to the caller's
# pane (%7) answers with FAKE_TMUX_STATE, the three flags a hook reads:
# pane_active, window_active and session_attached. "fail" (the default) makes
# the query fail, as it does when no server answers. A query scoped to any other
# pane fails too, so a hook that asks about the wrong pane cannot pass.
FAKE_BIN="$HOME/fake-bin"
mkdir -p "$FAKE_BIN"
cat > "$FAKE_BIN/tmux" <<'FAKE_TMUX_EOF'
#!/bin/sh
target=""
while [ $# -gt 0 ]; do
    if [ "$1" = -t ]; then target="${2:-}"; shift; fi
    shift
done
if [ -z "$target" ]; then
    echo "%99"
elif [ "$target" = "%7" ] && [ "${FAKE_TMUX_STATE:-fail}" != fail ]; then
    echo "$FAKE_TMUX_STATE"
else
    exit 1
fi
FAKE_TMUX_EOF
chmod +x "$FAKE_BIN/tmux"

# A jq that counts its calls, so a check can assert a hook runs it once at most.
REAL_JQ="$(command -v jq)"
JQ_CALLS="$HOME/jq-calls"
SPY_BIN="$HOME/spy-bin"
mkdir -p "$SPY_BIN"
printf '#!/bin/sh\necho x >> "%s"\nexec "%s" "$@"\n' "$JQ_CALLS" "$REAL_JQ" > "$SPY_BIN/jq"
chmod +x "$SPY_BIN/jq"

# An mv that logs source, destination and the source's content, then does the
# move. It shows that a status word reaches its file only by a rename.
MV_CALLS="$HOME/mv-calls"
cat > "$SPY_BIN/mv" <<'MV_EOF'
#!/bin/sh
src=""; dst=""
for a in "$@"; do
    case "$a" in -*) ;; *) src="$dst"; dst="$a" ;; esac
done
printf '%s\t%s\t%s\n' "$src" "$dst" "$(cat "$src" 2>/dev/null)" >> "$MV_CALLS_FILE"
exec "$REAL_MV" "$@"
MV_EOF
chmod +x "$SPY_BIN/mv"
export MV_CALLS_FILE="$MV_CALLS" REAL_MV
REAL_MV="$(command -v mv)"

# A jq that is not there, as sh reports it. Put first on PATH with JQ_BROKEN=1.
BROKEN_BIN="$HOME/broken-bin"
mkdir -p "$BROKEN_BIN"
printf '#!/bin/sh\nexit 127\n' > "$BROKEN_BIN/jq"
chmod +x "$BROKEN_BIN/jq"

# Runs a hook command with a JSON payload on stdin, as Claude Code does. With
# PANE_LESS=1 the command runs the way a pane-less agent would: no TMUX_PANE to
# inherit. The fake tmux is on the PATH either way, so the real server is never
# asked. Set FAKE_TMUX_STATE to choose what the fake tmux says about the pane.
run_command() {
    local json="$1" command="$2" broken=""
    [ "${JQ_BROKEN:-0}" = 1 ] && broken="$BROKEN_BIN:"
    local state="FAKE_TMUX_STATE=${FAKE_TMUX_STATE:-fail}"
    if [ "${PANE_LESS:-0}" = 1 ]; then
        printf '%s' "$json" | env -u TMUX_PANE -u TMUX "$state" "PATH=$broken$FAKE_BIN:$SPY_BIN:$PATH" sh -c "$command"
    else
        printf '%s' "$json" | env "$state" "PATH=$broken$FAKE_BIN:$SPY_BIN:$PATH" sh -c "$command"
    fi
}

entry_command() { printf '%s' "$1" | jq -r '.[0].hooks[0].command'; }

# fire_in JSON WORD MATCHER [MODE]
fire_in() {
    local json="$1"
    shift
    run_command "$json" "$(entry_command "$(status_hook_entry "$@")")"
}

fire() { fire_in '{}' "$@"; }

fire_pane_less() { PANE_LESS=1 fire "$@"; }

session_end() {
    run_command '{}' "$(entry_command "$(session_end_hook_entry)")"
}

session_end_pane_less() { PANE_LESS=1 session_end; }

# Revisions older than the subagent markers have no such entry. A no-op here
# lets the checks that depend on it fail instead of killing the run.
subagent_event() {
    local kind="$1" json="$2"
    if ! declare -F subagent_hook_entry >/dev/null; then
        return 0
    fi
    run_command "$json" "$(entry_command "$(subagent_hook_entry "$kind")")"
}

# The events, named as the state machine names them.
prompt_submit()   { fire working "" ; }
tool_call()       { fire working "^(?!AskUserQuestion$).*" live ; }
question_shown()  { fire waiting "^AskUserQuestion$" ; }
question_answered() { fire working "^AskUserQuestion$" ; }
notification()    { fire waiting "permission_prompt|idle_prompt" alert ; }
turn_end()        { fire review "" ; }

SUB_JSON='{"agent_id":"a1","tool_name":"Bash","tool_use_id":"t1"}'
MAIN_JSON='{"tool_name":"Bash","tool_use_id":"t2"}'
TOOL_MATCHER='^(?!AskUserQuestion$).*'
sub_tool_call()    { fire_in "$SUB_JSON" working "$TOOL_MATCHER" tool ; }
main_tool_call()   { fire_in "$MAIN_JSON" working "$TOOL_MATCHER" tool ; }
sub_start()        { subagent_event start '{"agent_id":"a1","agent_type":"general-purpose"}' ; }
sub_stop()         { subagent_event stop '{"agent_id":"a1","agent_type":"general-purpose"}' ; }
claude_stop()      { fire review "" stop ; }
permission_prompt() { fire waiting "permission_prompt" alert ; }
idle_prompt()      { fire waiting "idle_prompt" idle_alert ; }
compact_done()     { fire review "manual|auto" settle ; }
claude_session_start() { fire idle "startup|resume|clear" reset ; }
codex_session_start()  { fire idle "startup|resume|clear" rest ; }

# PermissionRequest and the tool-done events carry the same tool_name and
# tool_input, but the rest of the payload differs, and so does the key order.
POST_MATCHER='^(?!AskUserQuestion$).*'
REQ_X='{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"description":"make x","command":"mkdir x"}}'
REQ_Y='{"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"description":"make y","command":"mkdir y"}}'
DONE_X='{"tool_response":{"stdout":""},"tool_use_id":"t1","tool_input":{"command":"mkdir x","description":"make x"},"tool_name":"Bash","hook_event_name":"PostToolUse"}'
DONE_Y='{"tool_response":{"stdout":""},"tool_use_id":"t2","tool_input":{"command":"mkdir y","description":"make y"},"tool_name":"Bash","hook_event_name":"PostToolUse"}'
DONE_Z='{"tool_use_id":"t3","tool_input":{"command":"ls","description":"list"},"tool_name":"Bash","hook_event_name":"PostToolUse"}'
# An open request, planted directly, so a clearing check does not depend on permit.
seed_key()    { mkdir -p "$PERM_DIR" && : > "$PERM_DIR/planted"; }
permit()      { fire_in "$1" waiting "" permit ; }
tool_done()   { fire_in "$1" working "$POST_MATCHER" granted ; }
tool_failed() { fire_in "$1" working "" granted ; }
PERM_DIR="$HOME/.config/tws/permissions/%7"

# What tmux says about the pane: the visible pane of an attached session, and
# the four ways to be out of sight.
VISIBLE=111

MARKER="$HOME/.config/tws/subagents/%7/a1"
OLD_TIME=200001010000
backdate() { touch -c -t "$OLD_TIME" "$1"; }

reset() { rm -rf "$HOME/.config/tws"; }

# GNU form first: BSD stat rejects -c outright, while GNU stat *accepts* -f as
# "filesystem status" and prints a block report instead of failing over.
mtime() {
    local t
    t="$(stat -c '%Y' "$1" 2>/dev/null || stat -f '%m' "$1" 2>/dev/null)"
    case "$t" in
        ''|*[!0-9]*) printf 'cannot read mtime of %s\n' "$1" >&2; exit 1 ;;
        *) printf '%s' "$t" ;;
    esac
}

expect() {
    local want="$1" name="$2" got
    got="$(cat "$STATUS_FILE" 2>/dev/null || echo '<absent>')"
    if [ "$got" = "$want" ]; then
        printf '  ok   %s\n' "$name"
    else
        printf '  FAIL %s — want %s, got %s\n' "$name" "$want" "$got"
        failures=$((failures + 1))
    fi
}

# expect_marker present|absent NAME
expect_marker() {
    local want="$1" name="$2" got=absent
    [ -e "$MARKER" ] && got=present
    if [ "$got" = "$want" ]; then
        printf '  ok   %s\n' "$name"
    else
        printf '  FAIL %s — want marker %s, got %s\n' "$name" "$want" "$got"
        failures=$((failures + 1))
    fi
}

# expect_keys N NAME: how many open permission requests the pane has.
expect_keys() {
    local want="$1" name="$2" got
    got="$(find "$PERM_DIR" -type f 2>/dev/null | wc -l | tr -d ' ' || true)"
    if [ "${got:-0}" = "$want" ]; then
        printf '  ok   %s\n' "$name"
    else
        printf '  FAIL %s — want %s key file(s), got %s\n' "$name" "$want" "${got:-0}"
        failures=$((failures + 1))
    fi
}

panes_written() {
    ls -A "$HOME/.config/tws/agents" 2>/dev/null | sort | tr '\n' ' ' | sed 's/ $//'
}

expect_panes() {
    local want="$1" name="$2" got
    got="$(panes_written || true)"
    if [ "$got" = "$want" ]; then
        printf '  ok   %s\n' "$name"
    else
        printf '  FAIL %s — want [%s], got [%s]\n' "$name" "$want" "$got"
        failures=$((failures + 1))
    fi
}

printf 'a finished turn survives a background subagent\n'
reset
prompt_submit;  expect working "prompt starts the turn"
tool_call;      expect working "tool calls keep it working"
turn_end;       expect review  "Stop hands the pane back"
tool_call;      expect review  "the subagent's tool call must not repaint it"
tool_call;      expect review  "nor the next one"

printf '\na displayed question survives a background subagent\n'
reset
prompt_submit
question_shown; expect waiting "the question raises waiting"
tool_call;      expect waiting "the subagent's tool call must not repaint it"
question_answered; expect working "answering resumes the turn"

printf '\nnotifications raise waiting without overwriting review\n'
reset
prompt_submit
notification;   expect waiting "a permission prompt blocks the turn"
reset
prompt_submit
turn_end
notification;   expect review  "idle_prompt must not downgrade review"

printf '\nliveness\n'
reset
tool_call;      expect working "an empty file is claimed by the first tool call"
before="$(mtime "$STATUS_FILE")"
sleep 1.1
tool_call
after="$(mtime "$STATUS_FILE")"
if [ "$after" -gt "$before" ]; then
    printf '  ok   the heartbeat refreshes mtime\n'
else
    printf '  FAIL the heartbeat refreshes mtime — %s did not advance past %s\n' "$after" "$before"
    failures=$((failures + 1))
fi

printf '\nsubagents keep the pane working\n'
reset
prompt_submit; sub_start; claude_stop
expect working "Stop with a live subagent keeps the pane working"
backdate "$MARKER"
old=0; [ -e "$MARKER" ] && old="$(mtime "$MARKER")"
sub_tool_call
expect working "a subagent tool call leaves it working"
if [ -e "$MARKER" ] && [ "$(mtime "$MARKER")" -gt "$old" ]; then
    printf '  ok   a subagent tool call refreshes its marker\n'
else
    printf '  FAIL a subagent tool call refreshes its marker\n'
    failures=$((failures + 1))
fi
expect_marker present "SubagentStart creates the marker"
sub_stop
expect working "SubagentStop alone does not end the turn"
expect_marker absent "SubagentStop removes the marker"
prompt_submit; claude_stop
expect review "Stop without subagents hands the pane back"

reset
prompt_submit; sub_start; backdate "$MARKER"
expect_marker present "a backdated marker is in place"
claude_stop
expect review "a stale marker does not hold the pane"
expect_marker absent "Stop deletes a stale marker"

reset
prompt_submit; sub_start; fire review "" stop
expect working "StopFailure shares the marker guard"

printf '\na main-thread tool call starts the turn\n'
reset
prompt_submit; claude_stop
main_tool_call; expect working "a main-thread tool call after review resumes the turn"
reset
prompt_submit
printf idle > "$STATUS_FILE"
main_tool_call; expect working "and after idle"
reset
main_tool_call; expect working "and over an empty file"
reset
prompt_submit; question_shown
main_tool_call; expect waiting "but never over waiting"
reset
prompt_submit
backdate "$STATUS_FILE"
old="$(mtime "$STATUS_FILE")"
main_tool_call
if [ "$(mtime "$STATUS_FILE")" -gt "$old" ]; then
    printf '  ok   a main-thread tool call refreshes a working pane\n'
else
    printf '  FAIL a main-thread tool call refreshes a working pane\n'
    failures=$((failures + 1))
fi
reset
prompt_submit; claude_stop
sub_tool_call; expect review "a subagent tool call after review changes nothing"
reset
prompt_submit; question_shown
sub_tool_call; expect waiting "nor does it repaint a question"

printf '\na failed jq never reads as the main loop\n'
reset
prompt_submit; claude_stop
JQ_BROKEN=1 main_tool_call
expect review "a tool call with no working jq leaves review"
reset
prompt_submit; claude_stop
JQ_BROKEN=1 sub_tool_call
expect review "and so does a subagent call"
reset
JQ_BROKEN=1 main_tool_call
expect working "an empty file is still claimed"
reset
prompt_submit; claude_stop
fire_in '{ not json' working "$TOOL_MATCHER" tool
expect review "bad JSON leaves review"


TRIGGER="$HOME/.config/tws/agent.trigger"
printf '\na permission grant resumes the turn\n'
reset
prompt_submit
permit "$REQ_X"
expect waiting "a permission request raises waiting"
expect_keys 1 "and records one key file"
tool_done "$DONE_X"
expect working "the grant's PostToolUse resumes the turn"
expect_keys 0 "and removes the key file"

reset
prompt_submit
permit "$REQ_X"; permit "$REQ_Y"
expect_keys 2 "two requests make two key files"
tool_done "$DONE_X"
expect waiting "one grant leaves the other request open"
expect_keys 1 "and removes only its own key"
tool_done "$DONE_Y"
expect working "the last grant resumes the turn"

reset
prompt_submit
permit "$REQ_X"
tool_done "$DONE_Z"
expect waiting "a tool that never asked leaves the request open"
expect_keys 1 "and keeps its key"

reset
prompt_submit
permit "$REQ_X"
tool_failed "$DONE_X"
expect working "PostToolUseFailure also resumes the turn"
expect_keys 0 "and removes the key file"

reset
prompt_submit
permit "$REQ_X"; seed_key
expect_keys 2 "a request is open before the turn ends"
claude_stop
expect review "a denied tool ends the turn at review"
expect_keys 0 "and Stop clears the request"
if [ ! -e "$PERM_DIR" ]; then
    printf '  ok   and the pane directory\n'
else
    printf '  FAIL and the pane directory\n'
    failures=$((failures + 1))
fi
reset
prompt_submit
permit "$REQ_X"; seed_key
FAKE_TMUX_STATE=$VISIBLE claude_stop
expect idle "a denied tool in the visible pane ends the turn read"
expect_keys 0 "and Stop clears the request"
reset
prompt_submit
permit "$REQ_X"; seed_key
fire review "" stop
expect_keys 0 "StopFailure clears the request"
reset
prompt_submit
permit "$REQ_X"; seed_key
claude_session_start
expect_keys 0 "SessionStart clears the request"
reset
prompt_submit
permit "$REQ_X"; seed_key
session_end
expect_keys 0 "SessionEnd clears the request"

reset
prompt_submit; turn_end
permit "$REQ_X"
expect review "a permission request never downgrades review"
reset
permit "$REQ_X"
expect waiting "a permission request over an empty file raises waiting"
reset
prompt_submit; question_shown
tool_done "$DONE_X"
expect waiting "a tool with no open request leaves a question alone"
reset
prompt_submit; turn_end
permit "$REQ_X"
tool_done "$DONE_X"
expect review "a grant never overwrites review"
reset
prompt_submit
permit "$REQ_X"
notification
tool_done "$DONE_X"
expect working "the Notification backstop does not stop the grant resuming"
reset
prompt_submit
permit "$REQ_X"; rm -f "$TRIGGER"
tool_done "$DONE_X"
if [ -e "$TRIGGER" ]; then
    printf '  ok   a grant that resumes the turn rings the trigger\n'
else
    printf '  FAIL a grant that resumes the turn rings the trigger\n'
    failures=$((failures + 1))
fi

printf '\na tool with no open request runs no jq\n'
reset
prompt_submit
rm -f "$JQ_CALLS"
tool_done "$DONE_X"
tool_failed "$DONE_X"
if [ ! -e "$JQ_CALLS" ]; then
    printf '  ok   no permissions directory: no jq call\n'
else
    printf '  FAIL no permissions directory: %s jq call(s)\n' "$(wc -l < "$JQ_CALLS" | tr -d ' ')"
    failures=$((failures + 1))
fi
expect working "and the status stays as it was"
mkdir -p "$PERM_DIR"
rm -f "$JQ_CALLS"
tool_done "$DONE_X"
if [ ! -e "$JQ_CALLS" ]; then
    printf '  ok   an empty permissions directory: no jq call\n'
else
    printf '  FAIL an empty permissions directory: %s jq call(s)\n' "$(wc -l < "$JQ_CALLS" | tr -d ' ')"
    failures=$((failures + 1))
fi

printf '\na pane with no TMUX_PANE keeps out of the permission files\n'
reset
prompt_submit
permit "$REQ_X"
before="$(find "$HOME/.config/tws" -type f | sort | tr '\n' ' ')"
PANE_LESS=1 permit "$REQ_Y"
PANE_LESS=1 tool_done "$DONE_X"
PANE_LESS=1 tool_failed "$DONE_X"
after="$(find "$HOME/.config/tws" -type f | sort | tr '\n' ' ')"
if [ "$before" = "$after" ]; then
    printf '  ok   the permission commands with no TMUX_PANE write and remove nothing\n'
else
    printf '  FAIL the permission commands with no TMUX_PANE changed the files\n'
    failures=$((failures + 1))
fi
expect waiting "and the status is unchanged"
reset
PANE_LESS=1 permit "$REQ_X"
if [ ! -e "$HOME/.config/tws/permissions" ]; then
    printf '  ok   a pane-less request makes no directory\n'
else
    printf '  FAIL a pane-less request makes no directory\n'
    failures=$((failures + 1))
fi

printf '\nidle_prompt yields to a live subagent\n'
reset
prompt_submit; sub_start
idle_prompt;   expect working "idle_prompt with a fresh marker changes nothing"
permission_prompt; expect waiting "a permission prompt still raises waiting"
reset
prompt_submit
idle_prompt;   expect waiting "idle_prompt without a marker raises waiting"
reset
prompt_submit; sub_start; backdate "$MARKER"
idle_prompt;   expect waiting "a stale marker does not block idle_prompt"

printf '\nan unmatched SubagentStop is harmless\n'
reset
prompt_submit; sub_start
if subagent_event stop '{"agent_id":"nope","agent_type":""}'; then
    printf '  ok   SubagentStop with no marker exits cleanly\n'
else
    printf '  FAIL SubagentStop with no marker exits cleanly\n'
    failures=$((failures + 1))
fi
expect working "and changes no status"
expect_marker present "and leaves the other markers alone"

printf '\na turn that ends in the visible pane is read\n'
reset
prompt_submit
FAKE_TMUX_STATE=$VISIBLE claude_stop
expect idle "Stop in the visible pane marks it read"
reset
prompt_submit
FAKE_TMUX_STATE=112 claude_stop
expect idle "two attached clients still count as in view"
reset
prompt_submit
FAKE_TMUX_STATE=$VISIBLE fire review "" stop
expect idle "StopFailure shares the visibility check"
reset
prompt_submit
FAKE_TMUX_STATE=$VISIBLE compact_done
expect idle "a manual compaction that ends in the visible pane is read"
reset
prompt_submit
compact_done
expect review "a compaction in a pane out of sight stays review"
reset
prompt_submit
FAKE_TMUX_STATE=$VISIBLE turn_end
expect review "a plain set-mode write never asks tmux"

for state in 101 011 110 001 000 fail; do
    case "$state" in
        101) why="the window is not the active window" ;;
        011) why="the pane is not the active pane" ;;
        110) why="no client is attached" ;;
        001) why="only a client is attached" ;;
        000) why="nothing is active" ;;
        fail) why="the tmux query fails" ;;
    esac
    reset
    prompt_submit
    FAKE_TMUX_STATE=$state claude_stop
    expect review "Stop stays review when $why"
done
reset
prompt_submit
FAKE_TMUX_STATE=garbage claude_stop
expect review "and when tmux answers with something else"

reset
prompt_submit; sub_start
FAKE_TMUX_STATE=$VISIBLE claude_stop
expect working "a live subagent keeps a visible pane working"
reset
prompt_submit; sub_start; backdate "$MARKER"
FAKE_TMUX_STATE=$VISIBLE claude_stop
expect idle "a stale marker does not hold a visible pane"
reset
prompt_submit; sub_start
sub_stop
FAKE_TMUX_STATE=$VISIBLE claude_stop
expect idle "a visible pane is read once the subagents are done"

printf '\na new Claude session starts idle\n'
for stale in review working waiting idle; do
    reset
    prompt_submit
    printf '%s' "$stale" > "$STATUS_FILE"
    claude_session_start
    expect idle "a stale $stale file becomes idle"
done
reset
claude_session_start
expect idle "a missing file becomes idle"
reset
mkdir -p "$(dirname "$STATUS_FILE")"; : > "$STATUS_FILE"
claude_session_start
expect idle "an empty file becomes idle"
reset
prompt_submit; sub_start
expect_marker present "a marker is in place before the session starts"
claude_session_start
expect_marker absent "SessionStart removes the pane's subagent markers"
if [ ! -e "$(dirname "$MARKER")" ]; then
    printf '  ok   and the marker directory\n'
else
    printf '  FAIL and the marker directory\n'
    failures=$((failures + 1))
fi
TRIGGER="$HOME/.config/tws/agent.trigger"
reset
prompt_submit; rm -f "$TRIGGER"
claude_session_start
if [ -e "$TRIGGER" ]; then
    printf '  ok   a changed word rings the trigger\n'
else
    printf '  FAIL a changed word rings the trigger\n'
    failures=$((failures + 1))
fi
reset
prompt_submit; claude_session_start; rm -f "$TRIGGER"
claude_session_start
expect idle "a second start keeps idle"
if [ ! -e "$TRIGGER" ]; then
    printf '  ok   an unchanged word leaves the trigger alone\n'
else
    printf '  FAIL an unchanged word leaves the trigger alone\n'
    failures=$((failures + 1))
fi
reset
prompt_submit; sub_start; backdate "$MARKER"; claude_session_start
prompt_submit; claude_stop
expect review "a turn after the reset ends normally"

printf '\na Codex session start never overwrites a live state\n'
reset
prompt_submit
codex_session_start;  expect working "working stays working"
reset
prompt_submit; permission_prompt
codex_session_start;  expect waiting "waiting stays waiting"
reset
prompt_submit; claude_session_start
codex_session_start;  expect idle "idle stays idle"
reset
prompt_submit; turn_end
codex_session_start;  expect idle "review becomes idle"
reset
codex_session_start;  expect idle "a missing file becomes idle"
reset
mkdir -p "$(dirname "$STATUS_FILE")"; : > "$STATUS_FILE"
codex_session_start;  expect idle "an empty file becomes idle"
reset
prompt_submit; sub_start
codex_session_start
expect_marker present "a Codex session start leaves the subagent markers"
reset
prompt_submit; rm -f "$TRIGGER"
codex_session_start
if [ ! -e "$TRIGGER" ]; then
    printf '  ok   a Codex start that changes nothing leaves the trigger alone\n'
else
    printf '  FAIL a Codex start that changes nothing leaves the trigger alone\n'
    failures=$((failures + 1))
fi
reset
prompt_submit; turn_end; rm -f "$TRIGGER"
codex_session_start
if [ -e "$TRIGGER" ]; then
    printf '  ok   a Codex start that changes the word rings the trigger\n'
else
    printf '  FAIL a Codex start that changes the word rings the trigger\n'
    failures=$((failures + 1))
fi

printf '\nthe SessionStart wiring\n'
# Runs the real configure_* functions against a throwaway HOME, twice. The fork
# pointer entry stays; the reset entry is a second one; a user's own hook and a
# second run change nothing.
wire_home="$HOME/wiring"
rm -rf "$wire_home"; mkdir -p "$wire_home/.claude" "$wire_home/.codex"
printf '%s' '{"hooks":{"SessionStart":[{"matcher":"","hooks":[{"type":"command","command":"echo mine"}]}]}}' \
    > "$wire_home/.claude/settings.json"
printf '%s' '{"hooks":{"SessionStart":[{"matcher":"","hooks":[{"type":"command","command":"echo mine"}]}]}}' \
    > "$wire_home/.codex/hooks.json"
wire() (
    HOME="$wire_home"
    read() { answer=y; }
    info() { :; }; ok() { :; }; warn() { :; }
    hooks_configured=0
    configure_claude_hooks
    configure_codex_hooks
)
if declare -F configure_claude_hooks >/dev/null; then
    wire >/dev/null; wire >/dev/null
    settings="$wire_home/.claude/settings.json"
    hooks="$wire_home/.codex/hooks.json"
    # count_where FILE FILTER: how many SessionStart entries pass FILTER.
    count_where() { jq "[.hooks.SessionStart[] | select($2)] | length" "$1"; }
    check_count() {
        local file="$1" filter="$2" want="$3" name="$4" got
        got="$(count_where "$file" "$filter")"
        if [ "$got" = "$want" ]; then
            printf '  ok   %s\n' "$name"
        else
            printf '  FAIL %s — want %s, got %s\n' "$name" "$want" "$got"
            failures=$((failures + 1))
        fi
    }
    is_tws='(.hooks[0].command | test("config/tws/"))'
    check_count "$settings" "$is_tws" 2 "Claude SessionStart holds the fork pointer plus the reset entry"
    check_count "$settings" "$is_tws and .matcher == \"\"" 1 "and the fork pointer entry keeps the empty matcher"
    check_count "$settings" "$is_tws and .matcher == \"startup|resume|clear\"" 1 \
        "and the reset entry matches exactly startup|resume|clear"
    check_count "$settings" "(.hooks[0].command == \"echo mine\")" 1 "and the user's own hook survives two runs"
    check_count "$hooks" "$is_tws" 1 "Codex SessionStart holds one tws entry"
    check_count "$hooks" "$is_tws and .matcher == \"startup|resume|clear\"" 1 \
        "and it matches exactly startup|resume|clear"
    check_count "$hooks" "(.hooks[0].command == \"echo mine\")" 1 "and the user's own hook survives two runs"
    # count_event FILE EVENT FILTER: how many entries of EVENT pass FILTER.
    count_event() { jq "[(.hooks.$2 // [])[] | select($3)] | length" "$1"; }
    check_event() {
        local file="$1" event="$2" filter="$3" want="$4" name="$5" got
        got="$(count_event "$file" "$event" "$filter")"
        if [ "$got" = "$want" ]; then
            printf '  ok   %s\n' "$name"
        else
            printf '  FAIL %s — want %s, got %s\n' "$name" "$want" "$got"
            failures=$((failures + 1))
        fi
    }
    printf '\nthe permission wiring\n'
    check_event "$settings" PostToolUse "$is_tws" 2 "Claude PostToolUse holds exactly the two tws entries after two runs"
    check_event "$settings" PostToolUse "$is_tws and .matcher == \"^AskUserQuestion\$\"" 1 \
        "and one is the question entry"
    check_event "$settings" PostToolUse "$is_tws and .matcher == \"^(?!AskUserQuestion\$).*\"" 1 \
        "and the other is the permission entry"
    for tool in AskUserQuestion Bash Agent Write; do
        n="$(jq --arg t "$tool" '[.hooks.PostToolUse[] | select(.hooks[0].command | test("config/tws/")) | .matcher as $m | select($t | test($m))] | length' "$settings")"
        want=1
        if [ "$n" = "$want" ]; then
            printf '  ok   PostToolUse matchers match %s exactly once\n' "$tool"
        else
            printf '  FAIL PostToolUse matchers match %s %s time(s)\n' "$tool" "$n"
            failures=$((failures + 1))
        fi
    done
    check_event "$settings" PermissionRequest "$is_tws" 1 "Claude PermissionRequest holds one tws entry"
    check_event "$settings" PostToolUseFailure "$is_tws" 1 "Claude PostToolUseFailure holds one tws entry"
    check_event "$settings" PostToolUseFailure "$is_tws and .matcher == \"\"" 1 "and it matches every tool"
    check_event "$hooks" PostToolUse "$is_tws" 1 "Codex PostToolUse keeps its one tws entry"
    check_event "$hooks" PermissionRequest "$is_tws" 1 "Codex PermissionRequest keeps its one tws entry"
    for source in compact fork; do
        if [[ "$source" =~ ^(startup|resume|clear)$ ]]; then
            printf '  FAIL the SessionStart matcher must not match %s\n' "$source"
            failures=$((failures + 1))
        else
            printf '  ok   the SessionStart matcher does not match %s\n' "$source"
        fi
    done
else
    printf '  FAIL %s has no configure_claude_hooks to check\n' "$INSTALL_SH"
    failures=$((failures + 1))
fi

printf '\neach hook runs jq at most once\n'
reset
permit_x()    { permit "$REQ_X" ; }
done_x()      { tool_done "$DONE_X" ; }
failed_x()    { tool_failed "$DONE_X" ; }
for call in main_tool_call sub_tool_call sub_start sub_stop permit_x done_x failed_x; do
    prompt_submit
    case "$call" in done_x|failed_x) permit "$REQ_X" ;; esac
    rm -f "$JQ_CALLS"
    "$call"
    calls="$(wc -l < "$JQ_CALLS" 2>/dev/null | tr -d ' ' || true)"
    if [ "${calls:-0}" -le 1 ]; then
        printf '  ok   %s runs jq %s time(s)\n' "$call" "${calls:-0}"
    else
        printf '  FAIL %s runs jq %s times\n' "$call" "$calls"
        failures=$((failures + 1))
    fi
done

printf '\npane identity\n'
# $TMUX_PANE is the only pane identity a hook has, and the file name it picks is
# the only sender identity tws sees — so a wrong name is a forged write nothing
# downstream can detect. A pane-id query cannot answer "who is calling": tmux
# replies for the current client's active pane, which is the pane the user is
# looking at. Every check below therefore asserts on *which* file moved.
reset
fire_pane_less working ""
expect_panes "" "a hook with no TMUX_PANE writes nothing"
reset
fire_pane_less working "^(?!AskUserQuestion$).*" live
expect_panes "" "nor does the liveness variant"
reset
prompt_submit
expect_panes "%7" "a hook with TMUX_PANE writes only its own pane"

# A pane-less agent writes nothing anywhere, markers included.
reset
sub_start
expect_marker present "a marker exists for the pane-less stop to leave alone"
PANE_LESS=1 sub_start
PANE_LESS=1 sub_stop
PANE_LESS=1 fire_in "$MAIN_JSON" working "$TOOL_MATCHER" tool
PANE_LESS=1 fire_in "$SUB_JSON" working "$TOOL_MATCHER" tool
fire_pane_less review "" stop
fire_pane_less waiting "idle_prompt" idle_alert
fire_pane_less idle "startup|resume|clear" reset
fire_pane_less idle "startup|resume|clear" rest
FAKE_TMUX_STATE=$VISIBLE fire_pane_less review "" stop
FAKE_TMUX_STATE=$VISIBLE fire_pane_less review "manual|auto" settle
files="$(find "$HOME/.config/tws" -type f 2>/dev/null | wc -l | tr -d ' ' || true)"
if [ "${files:-0}" = 1 ] && [ -e "$MARKER" ]; then
    printf '  ok   the subagent commands with no TMUX_PANE write nothing\n'
else
    printf '  FAIL the subagent commands with no TMUX_PANE left %s file(s), want only the marker\n' "$files"
    failures=$((failures + 1))
fi

# Revisions older than this check have no such helper. Report that as a failure
# rather than dying mid-run, so pointing the harness at one still tells you
# which checks the revision fails.
if declare -F session_end_hook_entry >/dev/null; then
    reset
    prompt_submit
    session_end_pane_less
    expect_panes "%7" "a session end with no TMUX_PANE deletes nobody's status"
    session_end
    expect_panes "" "a session end with TMUX_PANE drops its own"
    reset
    prompt_submit
    sub_start
    expect_marker present "a marker is in place before the session ends"
    session_end
    expect_marker absent "a session end drops the pane markers"
else
    printf '  FAIL %s has no session_end_hook_entry to check\n' "$INSTALL_SH"
    failures=$((failures + 1))
fi

# A query scoped with -t to $TMUX_PANE reads facts about the caller's own pane,
# and it is allowed. Any other display-message call is a pane-identity guess.
unscoped_queries() {
    grep 'display-message' | grep -v -e '-t "\$TMUX_PANE"' -e '"-t", pane,' || true
}

reset
sample='tmux display-message -p -t "$TMUX_PANE" "#{pane_active}"
execFileSync("tmux", ["display-message", "-p", "-t", pane, "#{pane_active}"])'
if [ -z "$(printf '%s\n' "$sample" | unscoped_queries)" ]; then
    printf '  ok   the pane-identity check allows a query scoped to the caller'"'"'s pane\n'
else
    printf '  FAIL the pane-identity check allows a query scoped to the caller'"'"'s pane\n'
    failures=$((failures + 1))
fi
for sample in \
    'tmux display-message -p "#{pane_id}"' \
    'tmux display-message -p -t "$OTHER" "#{pane_id}"' \
    'execFileSync("tmux", ["display-message", "-p", "#{pane_id}"])'; do
    if [ -n "$(printf '%s\n' "$sample" | unscoped_queries)" ]; then
        printf '  ok   the pane-identity check rejects: %s\n' "$sample"
    else
        printf '  FAIL the pane-identity check rejects: %s\n' "$sample"
        failures=$((failures + 1))
    fi
done

guesses="$(unscoped_queries < "$INSTALL_SH" | wc -l | tr -d ' ')"
if [ "$guesses" = "0" ]; then
    printf '  ok   no hook resolves its pane by asking tmux\n'
else
    printf '  FAIL no hook resolves its pane by asking tmux — %s occurrence(s) in %s\n' \
        "$guesses" "$INSTALL_SH"
    failures=$((failures + 1))
fi

printf '\nupgrade cleanup\n'
cleanup="$(sed -n '/^configure_agent_hooks()/,/^}/p' "$INSTALL_SH")"
if printf '%s' "$cleanup" | grep -q 'rm -rf "\$HOME/.config/tws/permissions"'; then
    printf '  ok   the upgrade cleanup clears the permissions directory\n'
else
    printf '  FAIL the upgrade cleanup clears the permissions directory\n'
    failures=$((failures + 1))
fi

printf '\nstatus writes are atomic\n'
# `printf word > "$f"` truncates first. A hook that reads in that gap sees an
# empty file, and `live` mode claims an empty file as `working`. Every write goes
# to a dot file in the same directory and renames into place.
has_direct_write() { grep -Eq '>[[:space:]]*"\$f"'; }

if printf 'printf x > "$f"' | has_direct_write \
    && ! printf 'printf x > "$t" && mv -f "$t" "$f"' | has_direct_write; then
    printf '  ok   the direct-write check tells the two shapes apart\n'
else
    printf '  FAIL the direct-write check tells the two shapes apart\n'
    failures=$((failures + 1))
fi

# Claude and Codex both use status_hook_entry, so this covers both.
for mode in set live alert tool stop idle_alert settle reset rest permit granted; do
    cmd="$(entry_command "$(status_hook_entry working "" "$mode")")"
    if printf '%s' "$cmd" | has_direct_write; then
        printf '  FAIL %s mode redirects straight into "$f"\n' "$mode"
        failures=$((failures + 1))
    else
        printf '  ok   %s mode never redirects straight into "$f"\n' "$mode"
    fi
done

# expect_rename NAME WORD: the last mv moved a dot file holding WORD onto the status file.
expect_rename() {
    local name="$1" word="$2" line src="" dst="" content="" dot=0
    line="$(tail -n 1 "$MV_CALLS" 2>/dev/null || true)"
    IFS=$'\t' read -r src dst content <<< "$line" || true
    case "$(basename "$src")" in .*) dot=1 ;; esac
    if [ "$dst" = "$STATUS_FILE" ] && [ "$content" = "$word" ] \
        && [ "$(dirname "$src")" = "$(dirname "$STATUS_FILE")" ] && [ "$dot" = 1 ]; then
        printf '  ok   %s\n' "$name"
    else
        printf '  FAIL %s — last mv was [%s]\n' "$name" "$line"
        failures=$((failures + 1))
    fi
}

expect_no_temp() {
    local name="$1" left
    left="$(find "$HOME/.config/tws/agents" -name '.*' 2>/dev/null | tr '\n' ' ' || true)"
    if [ -z "$left" ]; then
        printf '  ok   %s\n' "$name"
    else
        printf '  FAIL %s — left [%s]\n' "$name" "$left"
        failures=$((failures + 1))
    fi
}

reset; : > "$MV_CALLS"
prompt_submit;  expect_rename "set mode writes through a rename" working
expect_no_temp "and leaves no temp file"
claude_stop;    expect_rename "stop mode writes through a rename" review
expect_no_temp "and leaves no temp file"
main_tool_call; expect_rename "tool mode resumes a turn through a rename" working
expect_no_temp "and leaves no temp file"
notification;   expect_rename "alert mode writes through a rename" waiting
expect_no_temp "and leaves no temp file"
reset
tool_call;      expect_rename "live mode claims an empty file through a rename" working
expect_no_temp "and leaves no temp file"
reset
prompt_submit
idle_prompt;    expect_rename "idle_alert mode writes through a rename" waiting
expect_no_temp "and leaves no temp file"
reset
prompt_submit
compact_done;   expect_rename "settle mode writes through a rename" review
expect_no_temp "and leaves no temp file"
reset
prompt_submit
claude_session_start; expect_rename "reset mode writes through a rename" idle
expect_no_temp "and leaves no temp file"
reset
prompt_submit; turn_end
codex_session_start; expect_rename "rest mode writes through a rename" idle
expect_no_temp "and leaves no temp file"
reset
prompt_submit
permit "$REQ_X" ;  expect_rename "permit mode raises waiting through a rename" waiting
expect_no_temp "and leaves no temp file"
tool_done "$DONE_X"; expect_rename "granted mode resumes the turn through a rename" working
expect_no_temp "and leaves no temp file"
reset
prompt_submit; sub_start; sub_tool_call; sub_stop; session_end
expect_no_temp "the subagent and session-end hooks leave no temp file"
reset
prompt_submit
: > "$MV_CALLS"
tool_call
if [ ! -s "$MV_CALLS" ]; then
    printf '  ok   the heartbeat touches the file and does not rename\n'
else
    printf '  FAIL the heartbeat touches the file and does not rename\n'
    failures=$((failures + 1))
fi

printf '\n'
if [ "$failures" -eq 0 ]; then
    printf 'all checks passed\n'
else
    printf '%s check(s) failed\n' "$failures"
    exit 1
fi
