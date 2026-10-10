
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

`install.sh` adds the hooks and this binding. It does not overwrite a `prefix+F`
that you already use:

    bind-key F run-shell "tmux split-window -h -l 45% -t #{pane_id} \"tws fork-pane #{pane_id}\""

This feature works with Claude Code only. Codex and Pi panes do not support
forking. `codex resume` appends to the parent session instead of forking it.

### Read a pane by moving into it

An agent pane in the review state means that the agent finished and you did not
look yet. When you attach through tws, tws clears the state. When you move into
the pane with plain tmux, tws cannot see the move. The tmux hooks below close
that gap. They call `tws ack-pane` for the pane you land on. The command clears
the review state of that pane and changes nothing else.

`install.sh` adds the hooks together with the agent status hooks. It writes them
to your tmux config (`~/.tmux.conf`, or `~/.config/tmux/tmux.conf` when that is
the file you use). If you have no tmux config, it creates `~/.tmux.conf`. It also
loads the hooks into a tmux server that is running, so you do not need to reload.
The hooks use the absolute path of the binary, because `run-shell` does not read
your shell `PATH`:

    # tws ack hooks
    set-hook -g after-select-pane[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g after-select-window[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g client-session-changed[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g window-pane-changed[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'
    set-hook -g session-window-changed[89] 'run-shell -b "/home/you/.local/bin/tws ack-pane #{pane_id}"'

To add them by hand, write the full path of your own binary. Do not use `~`. The
path must not contain a space or one of these characters: `'` `"` `\` `$` `#` `;`.
tmux splits the hook line with its own quoting, and a bad path breaks the line.
`install.sh` does not write the hooks for such a path. It prints them instead.

The last two hooks cover `last-pane`, `kill-pane` and `kill-window`. These
commands move focus but fire none of the first three hooks. One move can fire
two hooks. This is safe, because `ack-pane` does the same thing each time.
`kill-session` fires no hook, so a client that falls to another session does
not clear the review state.

The fixed index `89` makes a reload replace the tws entry, and it leaves your own
hooks at other indexes alone.

### Status bar

The tmux status bar can show your agents, so you do not need to open tws to see
them. Each window tab shows one glyph for each agent in that window: a green `●`
working, an orange `●` waiting or review, a grey `○` idle. The right end shows the thread and the session.

     0 claude ●   1 zsh   2 review ●○                         tws › status-bar

`install.sh` adds the block below together with the agent status hooks. A config
that sets a status option, or a theme plugin that sets one, has a bar of its own.
For such a config, the installer asks you first. If you say yes, the block goes
after your lines and replaces your bar. If you say no, the installer changes
nothing and shows a note. The installer leaves out the
`status-interval` and `status-right-length` lines when your config sets them.

With [tmux-nova](https://github.com/o0th/tmux-nova), the installer keeps your
nova bar and adds the agents on top of it, with no question. It appends the
glyph command to your `@nova-pane`, adds a `tws` segment at the right end, and
runs nova again:

    # tws status bar
    set -ga @nova-pane '#(/home/you/.local/bin/tws bar window --since #{start_time} #{P:#{pane_id} })'  # tws status bar
    set -ga @nova-segments-0-right ' tws'  # tws status bar
    set -g @nova-segment-tws '#(/home/you/.local/bin/tws bar where -- #{q:session_name})'  # tws status bar
    set -g @nova-segment-tws-colors '#c88e68 #121212'  # tws status bar
    run-shell '/home/you/.tmux/plugins/tmux-nova/nova.tmux'  # tws status bar

When your config does not set `@nova-pane` or `@nova-segments-0-right`, the
block sets them to the nova default plus the tws part. For any other bar, the
installer uses the block below.

    # tws status bar
    set -g status-interval 5  # tws status bar
    set -g status-right-length 80  # tws status bar
    set -g status-style 'bg=#1e1e1e,fg=#d4d4d4'  # tws status bar
    set -g status-left ' '  # tws status bar
    set -g window-status-format ' #I #W#{s/[*-]//:window_flags}#(/home/you/.local/bin/tws bar window --since #{start_time} #{P:#{pane_id} }) '  # tws status bar
    set -g window-status-current-format '#[bg=#c88e68,fg=#121212] #I #W#{s/[*-]//:window_flags}#(/home/you/.local/bin/tws bar window --since #{start_time} #{P:#{pane_id} }) #[default]'  # tws status bar
    set -g status-right '#[bg=#c88e68,fg=#121212] #(/home/you/.local/bin/tws bar where -- #{q:session_name}) '  # tws status bar

To use your own bar, add the two commands to it. tmux replaces
`#{P:#{pane_id} }` with the panes of the window, and `tws bar window` prints
their glyphs in the tws theme colors. The current tab gets slightly darker
tones, because it most often has a bright background. `--since #{start_time}`
skips the status files from before the tmux server started. Put the command at
the end of the tab label: the glyph color stays on for the text after it.
`tws bar where` prints `thread › session`. For example, with
[tmux-nova](https://github.com/o0th/tmux-nova):

    set -g @nova-pane '#I#{?pane_in_mode,  #{pane_mode},}  #W#(/home/you/.local/bin/tws bar window --since #{start_time} #{P:#{pane_id} })'
    set -g @nova-segment-tws '#(/home/you/.local/bin/tws bar where -- #{q:session_name})'
    set -g @nova-segment-tws-colors '#c88e68 #121212'
    set -g @nova-segments-0-right 'tws'
    set -g @nova-status-style-active-bg '#c88e68'

A `#()` command does not read your shell `PATH`, so write the full path of your
own binary. tmux runs the commands again once in each `status-interval`, so a new
agent state can take up to 5 seconds to show.

To remove the bar, delete the lines that end with `# tws status bar` from your
tmux config, then restart tmux. To keep `install.sh` from adding the bar again,
add this line to your tmux config:

    # tws status bar off

### Notes

Each thread and session has its own markdown note, stored as a plain `.md` file under `~/.config/tws/notes/`. Press `Tab` to focus the notes panel, `Enter` to open the current note in `$EDITOR`. Renders with [glow](https://github.com/charmbracelet/glow) if installed, falls back to basic markdown otherwise. Handy for per-workstream scratch notes, todo lists, and command snippets.

### Fuzzy finder

Press `/` to search across all active sessions by name or path, sorted by most-recently-attached. Type to filter, `Enter` to attach. Works from any view.

### Recent sessions

Keys `1`–`5` attach to the five most-recently-attached sessions instantly — useful for hopping between two or three active workstreams without leaving the keyboard home row.

### Agent detection

tws scans tmux panes periodically and identifies running AI coding agents by their child process names — no manual registration. Agents appear automatically under their session in the tree. The optional install-time hooks make tws refresh immediately when an agent starts or stops, instead of waiting for the next scan. When the optional hooks are installed, tws also shows each agent's live status — working, waiting for you, finished and awaiting review, or idle — as a colored dot in the agents view (`v`), with a count of working/waiting agents in the status bar. A pane that goes silent for 15 minutes while claiming to work is treated as no longer working and drops to idle, so an interrupted or crashed agent does not stay marked active forever.

### Importing existing sessions

Already have tmux sessions running? `tws import` walks you through assigning them to threads, so that they show in the tree.

## Requirements

- **[tmux](https://github.com/tmux/tmux)** — required. tws manages tmux sessions; it does nothing without it.
- **[glow](https://github.com/charmbracelet/glow)** — optional, for rich markdown rendering in the notes panel. Falls back to basic rendering if absent.

## Install

### Install script (macOS / Linux, x86_64 / ARM)

```sh
curl -fsSL https://raw.githubusercontent.com/ytaskiran/tws/main/install.sh | bash
```

Downloads the latest release binary to `~/.local/bin`. Then the script scans your
setup and shows all the changes it can make in one list. It asks one question,
`Apply these changes? [Y/n]`. The list can hold:

- agent status hooks for each agent it finds: Claude Code (`~/.claude/settings.json`), Codex (`~/.codex/hooks.json` and `config.toml`), and Pi (`~/.pi/agent/extensions/`),
- the tmux ack hooks and the `prefix+F` fork binding in your tmux config (`~/.tmux.conf`, or `~/.config/tmux/tmux.conf`). It creates `~/.tmux.conf` if you have no tmux config, and loads the new lines into a running tmux server. It does not overwrite a `prefix+F` that you already use. Your config can load other files or plugins. If no tmux server runs, the installer cannot see their keys. It then skips the fork binding and asks you to start tmux and run install again,
- a `PATH` line for `~/.local/bin` in your shell rc and profile, if your shell does not find it,
- `glow`, with `brew` or `go`, if it is missing.

Answer `n` to install only the binary. A run with no terminal to answer from
(for example in CI) also changes nothing else.

Re-run the same command any time to upgrade. A re-run replaces the tws lines in place, so it adds no copies.

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
| `t` | Pick a theme with a live preview |
| `v` | Toggle agents view |
| `Tab` | Focus the notes panel |
| `q` | Quit |

The mouse works in the sessions view: point at a row to select it, and click a session or agent to open it. The wheel moves the selection in every list. To select text with the mouse, hold `Shift` (`Option` in iTerm2) while you drag.

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

Press `t` to open the theme picker. Move the cursor to preview each theme on the whole UI. `Enter` saves the theme as the `theme` line in `config.toml`, and `Esc` goes back to the theme you had.

Or set a preset by hand:

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
theme_picker = "t"

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
