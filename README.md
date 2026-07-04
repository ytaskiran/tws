
# tws

**tmux workspace manager** — organize your tmux sessions into threads.

<img width="960" height="625" alt="tws-launch" src="https://github.com/user-attachments/assets/365ec572-542f-4478-bb3c-6d25ef0131d8" />

tws is a terminal UI that adds a persistent organizational layer on top of tmux. tmux sessions are flat and ephemeral; tws groups them into **threads** you control, and launches and attaches them for you.

- **Threads** — units of work (e.g. `auth-refactor`, `bug-1234`, `scratch`), each holding one or more sessions
- **Sessions** — live tmux sessions, launched and attached from tws

Threads are saved to `~/.config/tws/`. Sessions are real tmux sessions discovered at runtime. Detach from a tmux session to go back to tws.

<img width="1470" height="956" alt="Screenshot 2026-06-02 at 22 34 36" src="https://github.com/user-attachments/assets/18da8c05-6973-4838-ad81-8b96c8010101" />

## Features

### Thread tree

The main view. The thread/session tree on the left, a notes panel on the right. Threads expand to show their sessions, and each running session reveals any AI coding agents detected inside it. Press `Enter` on a thread to spawn a new session, or on a session to attach.

<img width="1470" height="956" alt="Screenshot 2026-06-06 at 17 23 48" src="https://github.com/user-attachments/assets/0d38ff3d-1caf-4f5a-bd91-67dba3dad98f" />


### Agents view

Toggle with `v`. A flat view of every AI coding agent (Claude Code, Codex) running across all your sessions, regardless of which thread owns them. Pin frequently-used agents to numbered slots — `p` to pin, `P` to set a slot, `0`–`9` to jump to a pinned agent from anywhere.

<img width="1470" height="956" alt="Screenshot 2026-06-06 at 17 24 23" src="https://github.com/user-attachments/assets/d4f09aca-03d9-49df-b391-bcdc6c06fd7e" />


### Notes

Each thread and session has its own markdown note, stored as a plain `.md` file under `~/.config/tws/notes/`. Press `Tab` to focus the notes panel, `Enter` to open the current note in `$EDITOR`. Renders with [glow](https://github.com/charmbracelet/glow) if installed, falls back to basic markdown otherwise. Handy for per-workstream scratch notes, todo lists, and command snippets.

### Fuzzy finder

Press `/` to search across all active sessions by name or path, sorted by most-recently-attached. Type to filter, `Enter` to attach. Works from any view.

### Recent sessions

Keys `1`–`5` attach to the five most-recently-attached sessions instantly — useful for hopping between two or three active workstreams without leaving the keyboard home row.

### Agent detection

tws scans tmux panes periodically and identifies running AI coding agents by their child process names — no manual registration. Agents appear automatically under their session in the tree. The optional install-time hooks make tws refresh immediately when an agent starts or stops, instead of waiting for the next scan.

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
- configure agent-detection hooks in `~/.claude/settings.json` and/or `~/.codex/config.toml` (so tws can refresh its agent view when an agent starts or stops).

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
```

**Modes:** `normal`, `agents`, `notes`, `finder`, `input`, `confirm`.

**Key syntax:** single chars (`"q"`, `"A"`), modifier prefixes (`"ctrl+j"`, `"alt+x"`), named keys (`"enter"`, `"esc"`, `"space"`, `"tab"`, `"backspace"`, `"up"`, `"down"`, `"left"`, `"right"`).

If your config has malformed TOML or unknown action names, tws prints an error and exits — fix the file and re-launch.

## License

[MIT](LICENSE)
