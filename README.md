
# tws

**tmux workspace manager** — organize your tmux sessions into threads.

<img width="960" height="624" alt="tws-demo" src="https://github.com/user-attachments/assets/7b6d8503-5b65-4669-8593-e86ed77fa9e0" />

tws is a terminal UI that adds a persistent organizational layer on top of tmux. tmux sessions are flat and ephemeral; tws groups them into **threads** you control, and launches and attaches them for you.

- **Threads** — units of work (e.g. `auth-refactor`, `bug-1234`, `scratch`), each holding one or more sessions
- **Sessions** — live tmux sessions, launched and attached from tws

Threads are saved to `~/.config/tws/`. Sessions are real tmux sessions discovered at runtime. Detach from a tmux session to go back to tws.

<img width="2940" height="1912" alt="tws-welcome" src="https://github.com/user-attachments/assets/023bb887-9f75-4bdd-8a84-43d9c35fb8ef" />

## Features

### Thread tree

The main view. The thread/session tree on the left, a notes panel on the right. Threads expand to show their sessions, and each running session reveals any AI coding agents detected inside it. Press `Enter` on a thread to spawn a new session, or on a session to attach.

<img width="2940" height="1912" alt="tws-sessions" src="https://github.com/user-attachments/assets/cc23cebe-38c1-44f3-9f85-657284c1000e" />

### Thread working directories

A thread can have a default working directory. Sessions launched from it start
there; threads without one start in your home directory.

Press `c` on a thread to open the directory picker. The top row is always the
directory you are browsing — `Enter` there accepts it. Type to filter, `Tab` to
enter the first match, or move down to a subdirectory and press `Enter` to go
into it. `Backspace` deletes a filter character, or goes up a level when the
filter is empty. `Esc` cancels. New threads prompt for a directory right after
you name them.

To point a thread back at your home directory, `Backspace` up to `~` and press
`Enter`.

If a thread's directory is deleted, sessions still launch — they start in your
home directory with a warning in the status bar (tmux's status line, if tws
is running inside tmux).

### Agents view

Toggle with `v`. A flat view of every AI coding agent (Claude Code, Codex, Pi) running across all your sessions, regardless of which thread owns them. Pin frequently-used agents to numbered slots — `p` to pin, `P` to set a slot, `0`–`9` to jump to a pinned agent from anywhere.

<img width="2940" height="1912" alt="tws-agents" src="https://github.com/user-attachments/assets/79a2cd9e-7437-4539-a18e-cded8d3084bd" />

### Session fork (experimental)

Press `prefix+F` in a Claude Code pane to open a **forked** copy of that
session in a new pane on the right. The fork inherits the whole conversation.
It writes to a new session id. The parent transcript stays untouched.

Ask a throwaway question while the parent stays visible. Press `prefix+z` to
zoom the fork, and press it again to restore the split. Type `/exit` to close
the fork pane.

`install.sh` adds the hooks and, if you accept, this binding:

    bind-key F run-shell "tmux split-window -h -l 45% -t #{pane_id} \"tws fork-pane #{pane_id}\""

This feature works with Claude Code only. Codex and Pi panes do not support
forking. `codex resume` appends to the parent session instead of forking it.

### Read a pane by moving into it

An agent pane in the review state means that the agent finished and you did not
look yet. When you attach through tws, tws clears the state. When you move into
the pane with plain tmux, tws cannot see the move. The tmux hooks below close
that gap. They call `tws ack-pane` for the pane you land on. The command clears
the review state of that pane and changes nothing else.

`install.sh` adds the hooks if you accept. The hooks use the absolute path of the
binary, because `run-shell` does not read your shell `PATH`:

    # tws ack hooks
    set-hook -g after-select-pane[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g after-select-window[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g client-session-changed[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g window-pane-changed[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g session-window-changed[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'

Write the full path of your own binary in `~/.tmux.conf`. Do not use `~`. The
path must not contain a space or one of these characters: `'` `"` `\` `$` `#` `;`.
tmux splits the hook line with its own quoting, and a bad path breaks the line.
`install.sh` does not write the hooks for such a path. It prints them instead.

The last two hooks cover `last-pane`, `kill-pane` and `kill-window`. These
commands move focus but fire none of the first three hooks. One move can fire
two hooks. This is safe, because `ack-pane` does the same thing each time.
`kill-session` fires no hook, so a client that falls to another session does
not clear the review state.

The fixed index `89` makes a reload replace the tws entry, and it leaves your own
hooks at other indexes alone. Reload with `tmux source-file ~/.tmux.conf`.

### Notes

Each thread and session has its own markdown note, stored as a plain `.md` file under `~/.config/tws/notes/`. Press `Tab` to focus the notes panel, `Enter` to open the current note in `$EDITOR`. Renders with [glow](https://github.com/charmbracelet/glow) if installed, falls back to basic markdown otherwise. Handy for per-workstream scratch notes, todo lists, and command snippets.

### Fuzzy finder

Press `/` to search across all active sessions by name or path, sorted by most-recently-attached. Type to filter, `Enter` to attach. Works from any view.

### Recent sessions

Keys `1`–`5` attach to the five most-recently-attached sessions instantly — useful for hopping between two or three active workstreams without leaving the keyboard home row.

### Agent detection

tws scans tmux panes periodically and identifies running AI coding agents by their child process names — no manual registration. Agents appear automatically under their session in the tree. The optional install-time hooks make tws refresh immediately when an agent starts or stops, instead of waiting for the next scan. When the optional hooks are installed, tws also shows each agent's live status — working, waiting for you, finished and awaiting review, or idle — as a colored dot in the agents view (`v`), with a count of working/waiting agents in the status bar. A pane that goes silent for 15 minutes while claiming to work is treated as no longer working and drops to idle, so an interrupted or crashed agent does not stay marked active forever.

### Importing existing sessions

Already have tmux sessions running? `tws import` walks you through assigning them to threads instead of leaving them orphaned outside the hierarchy.

## Requirements

- **[tmux](https://github.com/tmux/tmux)** — required. tws manages tmux sessions; it does nothing without it.
- **[glow](https://github.com/charmbracelet/glow)** — optional, for rich markdown rendering in the notes panel. Falls back to basic rendering if absent.

## Install

### Install script (macOS / Linux, x86_64 / ARM)

```sh
curl -fsSL https://raw.githubusercontent.com/ytaskiran/tws/main/install.sh | bash
```

Downloads the latest release binary to `~/.local/bin`. The script will, **with your confirmation at each step**, also offer to:

- add `~/.local/bin` to your `PATH`,
- install `glow`,
- configure agent-detection hooks in `~/.claude/settings.json`, `~/.codex/config.toml`, and/or `~/.pi/agent/extensions/` (so tws can refresh its agent view when an agent starts or stops).

Re-run the same command any time to upgrade.

### Build from source

Requires a [Rust toolchain](https://rustup.rs/).

```sh
git clone https://github.com/ytaskiran/tws
cd tws
cargo install --path .
```

### macOS Gatekeeper

Release binaries are ad-hoc signed, not notarized. If macOS blocks the binary, clear the quarantine attribute:

```sh
xattr -dr com.apple.quarantine ~/.local/bin/tws
```

## Usage

```sh
tws          # launch the TUI
tws import   # interactively import existing unmanaged tmux sessions
tws ack-pane [PANE_ID]   # mark a pane as read (used by the tmux hooks)
```

The status bar shows context-aware key hints for whatever is selected. The essentials:

### Navigate

| Key | Action |
|---|---|
| `j` / `k` (or `↓` / `↑`) | Move down / up |
| `h` / `l` (or `←` / `→`) | Collapse / expand |
| `Space` | Toggle expand |
| `e` | Toggle expand all |
| `1`–`5` | Attach to a recent session |
| `/` | Fuzzy-find and attach to any session |
| `v` | Toggle agents view |
| `Tab` | Focus the notes panel |
| `q` | Quit |

### Organize

| Key | Action |
|---|---|
| `a` | Add a thread |
| `r` | Rename selected item |
| `d` | Delete selected thread |
| `m` | Move a session to another thread |
| `c` | Set the selected thread's working directory |

### Sessions

| Key | Action |
|---|---|
| `Enter` | Attach to a session, or create a new one on a thread |
| `x` | Kill the selected session (or a thread's sessions) |

Inside a session, detach with `prefix + d` to return to tws.

## Configuration

Optional. Drop a TOML file at `~/.config/tws/config.toml` to customize the theme, palette, and keybindings. A missing or empty file keeps tws's defaults — everything below is opt-in.

### Theme

Pick a built-in preset:

```toml
theme = "catppuccin-mocha"
```

Available presets: `default`, `catppuccin-mocha`, `catppuccin-macchiato`, `catppuccin-frappe`, `catppuccin-latte`, `gruvbox-dark`, `gruvbox-light`, `nord`, `tokyo-night`.

You can also drop a custom theme file at `~/.config/tws/themes/<name>.toml` and reference it by name. Custom themes use the same `[palette]` schema as below.

### Palette overrides

Override any subset of the 7 palette colors on top of the chosen theme:

```toml
[palette]
accent = "#ff9e64"   # primary accent (threads, highlights)
green  = "#a6e3a1"   # sessions, success states
fg     = "#d4d4d4"   # foreground text
dim    = "#a0a0a0"   # secondary text
muted  = "#646464"   # tertiary / disabled
border = "#3c3c3c"   # borders, separators
bg     = "#1e1e1e"   # background
```

### Keybindings

Rebind any action by mode. Only specify what you want to change; unspecified actions keep their defaults.

```toml
[keys.normal]
quit       = "q"
add        = "a"
move_down  = "ctrl+j"
move_up    = "ctrl+k"
finder     = "/"

[keys.agents]
toggle_view = "v"

[keys.notes]
scroll_down = "j"
scroll_up   = "k"

[keys.dir_picker]
complete = "tab"
```

**Modes:** `normal`, `agents`, `notes`, `finder`, `input`, `confirm`, `dir_picker`.

**Key syntax:** single chars (`"q"`, `"A"`), modifier prefixes (`"ctrl+j"`, `"alt+x"`), named keys (`"enter"`, `"esc"`, `"space"`, `"tab"`, `"backspace"`, `"up"`, `"down"`, `"left"`, `"right"`).

If your config has malformed TOML or unknown action names, tws prints an error and exits — fix the file and re-launch.

## License

[MIT](LICENSE)
