#!/usr/bin/env bash
# Behaviour tests for the agent status hooks that install.sh writes into
# ~/.claude/settings.json and ~/.codex/hooks.json.
#
# Each test runs the emitted hook command for real, against a sandbox HOME and
# a fake tmux, then asserts which status file it touched.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/install.sh"
set +e  # install.sh sets -e; a failed assertion must not abort the run

fails=0
runs=0

assert_equals() {
    local want="$1" got="$2" what="$3"
    runs=$((runs + 1))
    if [ "$want" = "$got" ]; then
        printf '  ok   %s\n' "$what"
    else
        printf '  FAIL %s\n       want: [%s]\n       got:  [%s]\n' "$what" "$want" "$got"
        fails=$((fails + 1))
    fi
}

# A tmux that answers the pane query with a pane the caller does not own.
# The real one answers with the active pane of whichever client is current.
fake_tmux_dir() {
    local dir="$1"
    mkdir -p "$dir/bin"
    cat > "$dir/bin/tmux" <<'EOF'
#!/bin/sh
echo "%99"
EOF
    chmod +x "$dir/bin/tmux"
}

sandbox() {
    local dir
    dir="$(mktemp -d)"
    fake_tmux_dir "$dir"
    printf '%s' "$dir"
}

# Extract the shell command out of the hook JSON that status_hook_entry emits.
hook_command() {
    status_hook_entry "$@" | jq -r '.[0].hooks[0].command'
}

# Run a hook command as an agent would: sandbox HOME, fake tmux, chosen pane.
run_hook() {
    local box="$1" pane="$2" cmd="$3"
    if [ -z "$pane" ]; then
        env -u TMUX_PANE -u TMUX "HOME=$box" "PATH=$box/bin:$PATH" sh -c "$cmd"
    else
        env -u TMUX "HOME=$box" "PATH=$box/bin:$PATH" "TMUX_PANE=$pane" sh -c "$cmd"
    fi
}

statuses_in() {
    ls -A "$1/.config/tws/agents" 2>/dev/null | tr '\n' ' ' | sed 's/ $//'
}

test_hook_without_tmux_pane_writes_no_status() {
    local box
    box="$(sandbox)"

    run_hook "$box" "" "$(hook_command working "")"

    assert_equals "" "$(statuses_in "$box")" \
        "a hook with no TMUX_PANE writes no status file"
    rm -rf "$box"
}

test_hook_with_tmux_pane_writes_that_pane_only() {
    local box
    box="$(sandbox)"

    run_hook "$box" "%7" "$(hook_command working "")"

    assert_equals "%7" "$(statuses_in "$box")" \
        "a hook with TMUX_PANE writes exactly its own pane"
    assert_equals "working" "$(cat "$box/.config/tws/agents/%7")" \
        "the status word reaches the file"
    rm -rf "$box"
}

test_session_end_without_tmux_pane_removes_nothing() {
    local box
    box="$(sandbox)"
    mkdir -p "$box/.config/tws/agents"
    printf 'review' > "$box/.config/tws/agents/%99"

    run_hook "$box" "" "$(session_end_hook_entry | jq -r '.[0].hooks[0].command')"

    assert_equals "%99" "$(statuses_in "$box")" \
        "a session end with no TMUX_PANE deletes no other pane's status"
    rm -rf "$box"
}

test_session_end_with_tmux_pane_removes_its_own_status() {
    local box
    box="$(sandbox)"
    mkdir -p "$box/.config/tws/agents"
    printf 'review' > "$box/.config/tws/agents/%7"
    printf 'review' > "$box/.config/tws/agents/%8"

    run_hook "$box" "%7" "$(session_end_hook_entry | jq -r '.[0].hooks[0].command')"

    assert_equals "%8" "$(statuses_in "$box")" \
        "a session end removes its own status and leaves the rest"
    rm -rf "$box"
}

test_heartbeat_hook_without_tmux_pane_writes_no_status() {
    local box
    box="$(sandbox)"

    run_hook "$box" "" "$(hook_command working "" heartbeat)"

    assert_equals "" "$(statuses_in "$box")" \
        "the heartbeat variant also stays silent with no TMUX_PANE"
    rm -rf "$box"
}

# tmux answers this query for the *current client's* active pane, not the
# caller's, so any hook asking it stamps whichever pane the user is watching.
test_no_hook_guesses_the_pane_id() {
    local hits
    hits="$(grep -c 'display-message' "$ROOT/install.sh")"

    assert_equals "0" "$hits" \
        "no hook resolves its pane with tmux display-message"
}

for t in $(declare -F | awk '{print $3}' | grep '^test_'); do
    printf '%s\n' "$t"
    "$t"
done

printf '\n%d assertions, %d failed\n' "$runs" "$fails"
[ "$fails" -eq 0 ]
