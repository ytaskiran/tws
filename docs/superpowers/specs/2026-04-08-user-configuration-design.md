# User Configuration System — Design Spec

## Overview

Make tws configurable so users can customize keybindings and themes via a TOML config file at `~/.config/tws/config.toml`. The system follows Rust TUI ecosystem conventions (Helix, Zellij, Yazi) with sensible defaults — a missing or empty config file reproduces today's exact behavior.

## Config File Format

**Location:** `~/.config/tws/config.toml`

```toml
# Theme selection — name of a built-in preset or custom theme file
theme = "catppuccin-mocha"

# Optional palette overrides applied on top of the chosen theme
[palette]
accent = "#ff9e64"
green = "#a6e3a1"

# Keybindings — action-to-key mapping, organized by mode
# Only specify overrides; unspecified actions keep their defaults
[keys.normal]
quit = "q"
add = "a"
rename = "r"

[keys.agents]
quit = "q"

[keys.notes]
open_editor = "enter"

[keys.finder]
confirm = "enter"

[keys.input]
confirm = "enter"

[keys.confirm]
confirm = "y"
```

**Principle:** Everything is optional. An empty file or no file at all gives exactly today's behavior.

## Theme System

### Base Palette

Themes are defined by 7 semantic base colors. The app derives all 30+ UI styles from these:

| Palette key | Role | Default value |
|---|---|---|
| `accent` | Primary accent — collections, highlights, modals, cursor | `#cc7832` |
| `green` | Sessions, badges, code blocks | `#82b482` |
| `fg` | Default foreground text | `#d4d4d4` |
| `dim` | Secondary/less important text | `#a0a0a0` |
| `muted` | Hints, placeholders, descriptions | `#646464` |
| `border` | Borders, separators, subtle chrome | `#3c3c3c` |
| `bg` | Background (used for highlight contrast) | `#1e1e1e` |

### Style Derivation

`theme.rs` exposes a `fn build_theme(palette: &Palette) -> Theme` that maps palette colors to all UI styles. Key derivation rules:

- `COLLECTION_STYLE` → bold, brightened `accent` (accent RGB + ~16 per channel)
- `THREAD_STYLE` → `accent`
- `THREAD_DIM_STYLE` → darkened `accent` (~50% toward `border`)
- `SESSION_STYLE` → `green`
- `HIGHLIGHT_STYLE` → `fg: bg`, `bg: accent`, bold
- `HIGHLIGHT_UNFOCUSED_STYLE` → `fg: fg`, `bg: border`
- `SEPARATOR_STYLE` → `border`
- `STATUSBAR_KEY_STYLE` → `dim`
- `STATUSBAR_DESC_STYLE` → midpoint between `muted` and `border`
- `CURSOR_STYLE` → `accent`, slow blink
- `MODAL_BORDER_STYLE` → `accent`
- `MODAL_TITLE_STYLE` → `accent`, bold
- `MODAL_MUTED_STYLE` → `muted`
- `EMPTY_TITLE_STYLE` → `accent`, bold
- `EMPTY_HINT_STYLE` → `muted`
- `AGENT_STYLE` → `dim`
- `AGENT_CONNECTOR_STYLE` → `muted`
- `BADGE_DOT_STYLE` → `green`
- `BADGE_COUNT_STYLE` → `muted`
- `FLASH_STYLE` → `accent`, bold
- `RECENT_NUMBER_STYLE` → `accent`, bold
- `RECENT_NAME_STYLE` → `dim`
- `SCROLLBAR_THUMB_STYLE` → `muted`
- `SCROLLBAR_TRACK_STYLE` → `border`
- `NOTES_BORDER_FOCUSED` → `accent`
- `NOTES_BORDER_UNFOCUSED` → `border`
- `NOTES_TITLE_FOCUSED` → `accent`, bold
- `NOTES_TITLE_UNFOCUSED` → `dim`
- `NOTES_PLACEHOLDER_STYLE` → `muted`
- `PREVIEW_BORDER_STYLE` → `border`
- `PREVIEW_TITLE_STYLE` → `dim`
- `PREVIEW_PLACEHOLDER_STYLE` → `muted`
- `NoteStyleSheet::heading(1)` → `accent`, bold
- `NoteStyleSheet::heading(2)` → `accent`
- `NoteStyleSheet::heading(3+)` → `dim`, italic
- `NoteStyleSheet::code()` → `green`
- `NoteStyleSheet::link()` → `accent`, underlined
- `NoteStyleSheet::blockquote()` → `muted`, italic
- `NoteStyleSheet::heading_meta()` → `muted`
- `NoteStyleSheet::metadata_block()` → `muted`

### Built-in Theme Presets

Embedded via `include_str!()` from `src/themes/`:

| Theme name | File |
|---|---|
| `default` | `default.toml` — current warm palette |
| `catppuccin-mocha` | `catppuccin-mocha.toml` |
| `catppuccin-latte` | `catppuccin-latte.toml` |
| `catppuccin-macchiato` | `catppuccin-macchiato.toml` |
| `catppuccin-frappe` | `catppuccin-frappe.toml` |
| `gruvbox-dark` | `gruvbox-dark.toml` |
| `gruvbox-light` | `gruvbox-light.toml` |
| `tokyo-night` | `tokyo-night.toml` |
| `nord` | `nord.toml` |

Each preset file contains only `[palette]` keys (the same 7 colors).

### Custom Themes

Users can create `~/.config/tws/themes/mytheme.toml` with a `[palette]` section and reference it with `theme = "mytheme"`.

### Theme Resolution Order

1. Check `~/.config/tws/themes/<name>.toml` (user custom themes)
2. Check embedded presets
3. Fall back to `default`
4. Apply `[palette]` overrides from `config.toml` on top

## Keybinding System

### Action Enum

Every bindable action has a named variant:

```rust
enum Action {
    // Navigation (shared across modes)
    MoveDown, MoveUp, MoveLeft, MoveRight,

    // Normal mode tree actions
    Quit, Enter, Add, AddCollection, Rename, Delete,
    KillSession, Finder, ExpandAll, ToggleSelect, Deselect,
    RecentSession1, RecentSession2, RecentSession3,
    RecentSession4, RecentSession5,
    ToggleView,

    // Notes
    OpenEditor, ScrollUp, ScrollDown,

    // Modal shared
    Confirm, Cancel,

    // Input/Finder
    Backspace,
}
```

### Mode Scopes

| Config section | Maps to handler |
|---|---|
| `[keys.normal]` | `handle_normal_key()` + `handle_normal_mode()` focus switching |
| `[keys.agents]` | `handle_agents_view_key()` |
| `[keys.notes]` | `handle_notes_key()` |
| `[keys.finder]` | `handle_finder_key()` |
| `[keys.input]` | `handle_input_key()` |
| `[keys.confirm]` | `handle_confirm_key()` |

### Key Syntax

TOML string values for keys:

| Syntax | Meaning |
|---|---|
| `"q"` | Plain q key |
| `"A"` | Shift+A (uppercase) |
| `"ctrl+j"` | Ctrl+J |
| `"enter"` | Enter key |
| `"esc"` | Escape |
| `"space"` | Space bar |
| `"tab"` | Tab |
| `"up"`, `"down"`, `"left"`, `"right"` | Arrow keys |
| `"backspace"` | Backspace |
| `"/"`, `"?"` | Literal special characters |

### Non-Rebindable Keys

- `Ctrl+C` — always quits (safety exit, hardcoded)
- Character input in Input/Finder modes — typing text is not an action
- Number keys `1-5` for recent sessions — positional, not semantic

### Runtime Keymap

At startup, a `Keymap` struct is built: `HashMap<(Mode, KeyEvent), Action>`. The event loop resolves `(current_mode, key_event) → Action` and matches on the action instead of raw key codes.

## Module Structure

### New Files

| File | Role |
|---|---|
| `src/config.rs` | `Config` struct, TOML loading, merge logic |
| `src/config/keys.rs` | `Action` enum, key string parsing, default keymap builder |
| `src/config/palette.rs` | `Palette` struct, hex color parsing, preset resolution |
| `src/themes/*.toml` | 9 embedded preset theme files |

### Changed Files

| File | Changes |
|---|---|
| `src/theme.rs` | Replace `pub const` styles with `Theme` struct + `build_theme(palette) -> Theme`. Add `NoteStyleSheet` that holds a `Palette` reference. |
| `src/app.rs` | Key dispatch changes from `match code { KeyCode::Char('q') => }` to `match keymap.resolve(mode, key) { Some(Action::Quit) => }`. `App` receives `Config` at construction. |
| `src/components/*.rs` | Render functions take `&Theme` parameter instead of importing `theme::CONST`. |
| `src/main.rs` | Load config before constructing `App`, pass config in. |
| `Cargo.toml` | Add `toml` dependency. |

### Unchanged Files

- `src/core/state.rs`, `src/core/model.rs`, `src/core/persistence.rs`, `src/core/notes.rs`
- `src/tmux/commands.rs`, `src/tmux/agent_scan.rs`
- All existing test modules

## Startup Data Flow

```
main.rs
  ├── load ~/.config/tws/config.toml → Config (defaults if missing)
  ├── resolve theme: user themes dir → embedded presets → default
  ├── apply [palette] overrides → final Palette
  ├── build_theme(palette) → Theme
  ├── build_keymap(config.keys, defaults) → Keymap
  └── App::new(state, theme, keymap, ...)
```

## Error Handling

- Missing config file → use all defaults silently
- Malformed TOML → print error to stderr, exit with helpful message pointing to the problematic line
- Unknown theme name → print warning, fall back to `default`
- Unknown action name in `[keys.*]` → print warning, skip that binding
- Invalid key syntax → print warning, skip that binding
- Duplicate action in same mode → last one wins (TOML standard)

## New Dependency

- `toml` crate (serde-compatible TOML parser, widely used in Rust ecosystem)
