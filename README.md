# tws

tmux workspace manager — organize sessions into collections and projects.

tws replaces tmux's built-in session picker (`prefix + s`) with a TUI that adds an organizational layer on top of tmux sessions. Group your sessions into **projects**, and projects into **collections**.

- **Collections** — top-level groups (e.g. "work", "personal", "infra")
- **Projects** — within a collection, each with one or more tmux sessions
- **Sessions** — ephemeral tmux sessions, launched and managed from tws

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/ytaskiran/tws/main/install.sh | bash
```

This downloads the latest release binary to `~/.local/bin` and optionally configures a tmux keybinding (`prefix + s`).

Supports macOS and Linux, both x86_64 and ARM.

### Upgrade

Run the same command again. It fetches the latest release and replaces the binary.

### Manual keybinding

If you skipped the tmux setup during install, add this to your `tmux.conf`:

```
bind-key s display-popup -E -w 80% -h 80% "tws"
```

## Usage

Run `tws` in a terminal, or press `prefix + s` inside tmux if you configured the keybinding.

## License

[MIT](LICENSE)
