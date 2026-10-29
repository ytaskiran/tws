#!/usr/bin/env python3
"""Rebuild tws tmux sessions as Rex sessions. Dry run unless --apply.

Each tmux session -> a Rex session with the same label, each window -> a tab,
extra panes in a window -> splits. The names stay, so tws keeps each session in its thread.
Claude panes restart as `claude --resume ID --fork-session`, so tmux is untouched.
"""
import glob, json, os, shutil, subprocess, sys
from collections import OrderedDict

REX = os.environ.get("REX_BIN") or shutil.which("rex") or os.path.expanduser(
    "~/Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex")
APPLY = "--apply" in sys.argv


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


def launch(pane, agents):
    """(cwd, rex args) for one tmux pane: a Claude pane resumes its conversation.
    --keep-open keeps the tab when claude fails, so the Rex session stays and a
    later window new -s still finds it."""
    pane_id, cwd = pane
    if pane_id not in agents:
        return cwd, []
    session_id, cwd = agents[pane_id]
    return cwd, ["--keep-open", "--", "claude", "--resume", session_id, "--fork-session"]


def main():
    fmt = "#{session_name}\t#{window_index}\t#{window_name}\t#{pane_id}\t#{pane_current_path}"
    rows = subprocess.run(["tmux", "list-panes", "-a", "-F", fmt], check=True, capture_output=True, text=True).stdout
    agents = claude_by_pane()
    existing = set()
    if APPLY:
        ls = json.loads(subprocess.run([REX, "ls", "--json"], check=True, capture_output=True, text=True).stdout)
        existing = {s.get("label") for s in (ls.get("sessions", ls) if isinstance(ls, dict) else ls)}

    # session -> window index -> (window name, [(pane_id, cwd)]) in tmux order.
    # The key is the index: two windows can have the same name, such as zsh.
    sessions = OrderedDict()
    for line in rows.splitlines():
        name, index, window, pane, cwd = line.split("\t")
        sessions.setdefault(name, OrderedDict()).setdefault(index, (window, []))[1].append((pane, cwd))

    for name, windows in sessions.items():
        print(f"\n# {name}")
        if name in existing:
            print("  skip: a Rex session with this label exists")
            continue
        sid = None
        for window, panes in windows.values():
            (cwd, cmd), rest = launch(panes[0], agents), panes[1:]
            if sid is None:
                r = rex("new", name, "-c", cwd, *cmd)
                sid, anchor = r["session_id"], r["initial_windows"][0]["block_ids"][0]
            else:
                anchor = rex("window", "new", "-s", sid, window, "-c", cwd, *cmd)["block_ids"][0]
            for pane in rest:
                split_cwd, split_cmd = launch(pane, agents)
                rex("split", "-s", sid, "-b", anchor, "--split=right", "--ratio", "35", "--focus=false",
                    "-c", split_cwd, *split_cmd)

    if not APPLY:
        print("\nDry run. Run again with --apply to create the sessions.")


if __name__ == "__main__":
    main()
