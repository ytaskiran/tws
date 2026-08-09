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
eval "$(sed -n '/^status_hook_entry()/,/^}/p;/^session_end_hook_entry()/,/^}/p' "$INSTALL_SH")"

export HOME
HOME="$(mktemp -d)"
export TMUX_PANE="%7"
STATUS_FILE="$HOME/.config/tws/agents/%7"
trap 'rm -rf "$HOME"' EXIT

failures=0

# Extracts the shell command out of the JSON entry and runs it.
fire() {
    local entry
    entry="$(status_hook_entry "$@")"
    sh -c "$(printf '%s' "$entry" | jq -r '.[0].hooks[0].command')"
}

# A tmux that answers the pane query with a pane the caller does not own — the
# real one answers for the current client's active pane, which is the same
# thing from the hook's point of view.
FAKE_BIN="$HOME/fake-bin"
mkdir -p "$FAKE_BIN"
printf '#!/bin/sh\necho "%%99"\n' > "$FAKE_BIN/tmux"
chmod +x "$FAKE_BIN/tmux"

# Fires a command the way a pane-less agent would: no TMUX_PANE to inherit, and
# a tmux standing by to answer if the command asks.
fire_pane_less() {
    local entry
    entry="$(status_hook_entry "$@")"
    env -u TMUX_PANE -u TMUX "PATH=$FAKE_BIN:$PATH" \
        sh -c "$(printf '%s' "$entry" | jq -r '.[0].hooks[0].command')"
}

session_end() {
    local entry
    entry="$(session_end_hook_entry)"
    sh -c "$(printf '%s' "$entry" | jq -r '.[0].hooks[0].command')"
}

session_end_pane_less() {
    local entry
    entry="$(session_end_hook_entry)"
    env -u TMUX_PANE -u TMUX "PATH=$FAKE_BIN:$PATH" \
        sh -c "$(printf '%s' "$entry" | jq -r '.[0].hooks[0].command')"
}

# The events, named as the state machine names them.
prompt_submit()   { fire working "" ; }
tool_call()       { fire working "^(?!AskUserQuestion$).*" live ; }
question_shown()  { fire waiting "^AskUserQuestion$" ; }
question_answered() { fire working "^AskUserQuestion$" ; }
notification()    { fire waiting "permission_prompt|idle_prompt" alert ; }
turn_end()        { fire review "" ; }

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
