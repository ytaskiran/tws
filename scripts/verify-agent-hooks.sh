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
eval "$(sed -n '/^SUBAGENT_FRESH_MINS=/p;/^status_hook_entry()/,/^}/p;/^session_end_hook_entry()/,/^}/p;/^subagent_hook_entry()/,/^}/p' "$INSTALL_SH")"

export HOME
HOME="$(mktemp -d)"
export TMUX_PANE="%7"
STATUS_FILE="$HOME/.config/tws/agents/%7"
trap 'rm -rf "$HOME"' EXIT

failures=0

# A tmux that answers the pane query with a pane the caller does not own — the
# real one answers for the current client's active pane, which is the same
# thing from the hook's point of view.
FAKE_BIN="$HOME/fake-bin"
mkdir -p "$FAKE_BIN"
printf '#!/bin/sh\necho "%%99"\n' > "$FAKE_BIN/tmux"
chmod +x "$FAKE_BIN/tmux"

# A jq that counts its calls, so a check can assert a hook runs it once at most.
REAL_JQ="$(command -v jq)"
JQ_CALLS="$HOME/jq-calls"
SPY_BIN="$HOME/spy-bin"
mkdir -p "$SPY_BIN"
printf '#!/bin/sh\necho x >> "%s"\nexec "%s" "$@"\n' "$JQ_CALLS" "$REAL_JQ" > "$SPY_BIN/jq"
chmod +x "$SPY_BIN/jq"

# A jq that is not there, as sh reports it. Put first on PATH with JQ_BROKEN=1.
BROKEN_BIN="$HOME/broken-bin"
mkdir -p "$BROKEN_BIN"
printf '#!/bin/sh\nexit 127\n' > "$BROKEN_BIN/jq"
chmod +x "$BROKEN_BIN/jq"

# Runs a hook command with a JSON payload on stdin, as Claude Code does. With
# PANE_LESS=1 the command runs the way a pane-less agent would: no TMUX_PANE to
# inherit, and a tmux standing by to answer if the command asks.
run_command() {
    local json="$1" command="$2" broken=""
    [ "${JQ_BROKEN:-0}" = 1 ] && broken="$BROKEN_BIN:"
    if [ "${PANE_LESS:-0}" = 1 ]; then
        printf '%s' "$json" | env -u TMUX_PANE -u TMUX "PATH=$broken$FAKE_BIN:$SPY_BIN:$PATH" sh -c "$command"
    else
        printf '%s' "$json" | env "PATH=$broken$SPY_BIN:$PATH" sh -c "$command"
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

printf '\neach hook runs jq at most once\n'
reset
for call in main_tool_call sub_tool_call sub_start sub_stop; do
    prompt_submit
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

reset
guesses="$(grep -c 'display-message' "$INSTALL_SH" || true)"
if [ "$guesses" = "0" ]; then
    printf '  ok   no hook resolves its pane by asking tmux\n'
else
    printf '  FAIL no hook resolves its pane by asking tmux — %s occurrence(s) in %s\n' \
        "$guesses" "$INSTALL_SH"
    failures=$((failures + 1))
fi

printf '\n'
if [ "$failures" -eq 0 ]; then
    printf 'all checks passed\n'
else
    printf '%s check(s) failed\n' "$failures"
    exit 1
fi
