# tws

**tmux workspace manager** — organize your tmux sessions into collections and threads.

tws is a terminal UI that adds a persistent organizational layer on top of tmux. tmux sessions are flat and ephemeral; tws groups them into a hierarchy you control, and launches/attaches them for you.

```
Collection → Thread → Session(s)
```

- **Collections** — top-level groups (e.g. `work`, `personal`, `infra`)
- **Threads** — units of work inside a collection, each holding one or more sessions
- **Sessions** — live tmux sessions, launched and attached from tws

Collections and threads are saved to `~/.config/tws/`. Sessions are real tmux sessions discovered at runtime. tws also detects running AI coding agents (Claude Code, Codex) inside your sessions and gives them a dedicated view.

<!-- TODO: add a screenshot or demo GIF of the tree view here -->

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
- configure agent-detection hooks in `~/.claude/settings.json` and/or `~/.codex/config.toml` (so tws can refresh its agent view when an agent starts/stops).

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
| `A` | Add a collection |
| `a` | Add a thread |
| `r` | Rename selected item |
| `d` | Delete selected collection / thread |
| `m` | Move a session to another thread |

### Sessions

| Key | Action |
|---|---|
| `Enter` | Attach to a session, or create a new one on a thread |
| `x` | Kill the selected session (or a thread's sessions) |

Inside a session, detach with `prefix + d` to return to tws.

## License

[MIT](LICENSE)
