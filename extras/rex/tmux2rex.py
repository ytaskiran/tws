#!/usr/bin/env python3
"""Rebuild tws tmux sessions as Rex sessions. Dry run unless --apply.

thread -> Rex session, twsr_<thread>_<label> -> tab, extra tmux windows -> splits.
Claude panes restart as `claude --resume ID --fork-session`, so tmux is untouched.
"""
import glob, json, os, shutil, subprocess, sys
from collections import OrderedDict

REX = os.environ.get("REX_BIN") or shutil.which("rex") or os.path.expanduser(
    "~/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex")
APPLY = "--apply" in sys.argv


def parse_name(name):
    """twsr_<thread>_<label> -> (thread, label); slugs use '-', never '_'."""
    if not name.startswith("twsr_"):
        return None
    thread, _, label = name[5:].partition("_")
    return thread, label or "main"


assert parse_name("twsr_safe-open-weight-protocol_brainstorm") == ("safe-open-weight-protocol", "brainstorm")
assert parse_name("twsr_tws_agent-session-recovery") == ("tws", "agent-session-recovery")
assert parse_name("work") is None


def claude_by_pane():
    """%N -> (sessionId, cwd) for live Claude processes, from ~/.claude/sessions/<pid>.json."""
    out = {}
    for f in glob.glob(os.path.expanduser("~/.claude/sessions/*.json")):
        try:
            d = json.load(open(f))
            os.kill(d["pid"], 0)  # skip stale files
        except (OSError, ValueError, KeyError):
            continue
        if d.get("tmux"):
            out[d["tmux"].rsplit(".", 1)[-1]] = (d["sessionId"], d["cwd"])
    return out


def rex(*args):
    print("  rex", " ".join(args))
    if not APPLY:
        return {"session_id": "<new>", "block_ids": ["<new>"], "initial_windows": [{"block_ids": ["<new>"]}]}
    # --json goes after the subcommand words but before any `--`, or the child program receives it
    words = 2 if args[0] == "window" else 1
    argv = [REX, *args[:words], "--json", *args[words:]]
    return json.loads(subprocess.run(argv, check=True, capture_output=True, text=True).stdout)


def main():
    fmt = "#{session_name}\t#{window_index}\t#{pane_id}\t#{pane_current_path}"
    rows = subprocess.run(["tmux", "list-panes", "-a", "-F", fmt], check=True, capture_output=True, text=True).stdout
    agents = claude_by_pane()
    existing = set()
    if APPLY:
        ls = json.loads(subprocess.run([REX, "ls", "--json"], check=True, capture_output=True, text=True).stdout)
        existing = {s.get("label") for s in (ls.get("sessions", ls) if isinstance(ls, dict) else ls)}

    # thread -> label -> [(pane_id, cwd)] in tmux order
    threads = OrderedDict()
    for line in rows.splitlines():
        name, _, pane, cwd = line.split("\t")
        parsed = parse_name(name)
        if parsed:
            threads.setdefault(parsed[0], OrderedDict()).setdefault(parsed[1], []).append((pane, cwd))

    for thread, tabs in threads.items():
        print(f"\n# thread {thread}")
        if thread in existing:
            print("  skip: a Rex session with this label exists")
            continue
        sid = None
        for label, panes in tabs.items():
            (pane, cwd), rest = panes[0], panes[1:]
            cmd = []
            if pane in agents:
                session_id, cwd = agents[pane]
                cmd = ["--", "claude", "--resume", session_id, "--fork-session"]
            if sid is None:
                r = rex("new", thread, "--window", label, "-c", cwd, *cmd)
                sid, anchor = r["session_id"], r["initial_windows"][0]["block_ids"][0]
            else:
                anchor = rex("window", "new", "-s", sid, label, "-c", cwd, *cmd)["block_ids"][0]
            for _, shell_cwd in rest:
                rex("split", "-s", sid, "-b", anchor, "--split=right", "--ratio", "35", "--focus=false", "-c", shell_cwd)

    if not APPLY:
        print("\nDry run. Run again with --apply to create the sessions.")


if __name__ == "__main__":
    main()
