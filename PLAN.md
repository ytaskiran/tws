# tws — tmux workspace manager

## Context

The built-in tmux session picker (`prefix + s`) is a flat, unstructured list. Existing tools (sesh, tmuxinator, tmuxp, etc.) either couple organization to the filesystem or just provide fancier switching for live sessions. None offer a persistent, abstract organizational hierarchy independent of running sessions.

**tws** is a standalone Rust TUI that replaces `prefix + s` as your workspace home screen:

```
Collection  (permanent, user-defined grouping)
  └── Project  (permanent entry within a collection)
        └── Session(s)  (ephemeral, launched into tmux on demand)
```

Collections and projects persist in a local config file. Sessions are transient — launched from a project, gone when killed, but the project entry stays.

---

## Architecture

**Pattern:** Hybrid TEA + Component Architecture (Ratatui conventions)
- Top-level: Elm Architecture — single event loop, `Action` enum, centralized dispatch
- UI panels: self-contained components behind a `Component` trait
- Core domain logic (`core/`) strictly separated from TUI (`components/`) — testable without a terminal

**Final module structure (grows incrementally):**

```
tws/
├── Cargo.toml
├── src/
│   ├── main.rs              # entry point, terminal setup/teardown
│   ├── app.rs               # App struct, main loop, action dispatch, mode state machine
│   ├── action.rs            # Action enum (all possible mutations)
│   ├── event.rs             # crossterm event polling → Action mapping
│   ├── tui.rs               # terminal init/restore (RAII)
│   ├── core/
│   │   ├── model.rs         # Collection, Project, Session types
│   │   ├── state.rs         # AppState: in-memory data + CRUD methods
│   │   └── persistence.rs   # load/save ~/.config/tws/state.json
│   ├── tmux/
│   │   └── commands.rs      # wrappers around tmux CLI via std::process::Command
│   ├── components/
│   │   ├── tree_view.rs     # main hierarchical browser
│   │   ├── status_bar.rs    # bottom bar with context-sensitive keybind hints
│   │   ├── input_modal.rs   # popup for naming collections/projects
│   │   ├── confirm_modal.rs # popup for delete confirmation
│   │   └── help_overlay.rs  # full keybinding reference
│   └── theme.rs             # centralized colors/styles
```

---

## Phase 1: Skeleton + Data Model + Basic TUI
**Status:** `done`

**Goal:** `cargo run` shows a navigable tree of hardcoded sample data. Proves the data model, rendering pipeline, and navigation.

### Deliverables
- [x] Cargo project initialized with dependencies
- [x] Core domain types: `Collection`, `Project`, `Session` (UUID identifiers, serde derives)
- [x] `AppState` struct with in-memory `Vec<Collection>`
- [x] Tree rendering via `tui-tree-widget` — collections as parents, projects as children
- [x] vim-style navigation: `j`/`k` or arrows to move, `Enter` to expand/collapse, `q` to quit
- [x] Clean terminal restore on exit (RAII in `tui.rs`)
- [x] Unit tests for model types (4 passing)

### Key decisions
- UUIDs as entity identifiers (stable across renames)
- `tui-tree-widget` crate for tree display — confirmed working well
- Synchronous event loop (no async)

### Dependencies introduced
`ratatui`, `crossterm`, `tui-tree-widget`, `uuid`, `serde`, `serde_json`

### Files created
`Cargo.toml`, `main.rs`, `app.rs`, `action.rs`, `event.rs`, `tui.rs`, `core/{model,state}.rs`, `components/tree_view.rs`, `theme.rs`

---

## Phase 2: Persistence + CRUD
**Status:** `done`

**Goal:** User can create, rename, delete collections and projects. State persists across restarts.

### Deliverables
- [x] `a` → create collection (at root) or project (inside collection) via input modal
- [x] `r` → rename focused item via input modal
- [x] `d` → delete focused item with confirmation modal
- [x] State auto-saves to `~/.config/tws/state.json` on every mutation
- [x] Loads from file on startup (empty state if no file exists)
- [x] Unit tests for persistence round-trip and CRUD operations (16 total, 12 new)

### Key decisions
- Modal system via `Mode` enum: `Normal`, `Input(InputContext)`, `Confirm(ConfirmContext)`
- Input routing changes based on current mode (tree nav vs text input vs yes/no)
- JSON persistence (serde_json, human-readable)
- Save-on-mutate (data is small, sync write is fine)

### New files
`core/persistence.rs`, `components/{input_modal,confirm_modal}.rs`

### Verification
- Create collections/projects, quit, relaunch — data persists
- Delete with confirmation works
- `cargo test` passes for persistence and CRUD tests

---

## Phase 3: tmux Integration
**Status:** `pending`

**Goal:** Launch, attach, and kill tmux sessions from projects. Core value proposition.

### Deliverables
- [ ] `Enter` on project → launch new session (if none) or attach to existing
- [ ] `x` on session → kill it
- [ ] Sessions shown as children of projects in the tree (visually distinct)
- [ ] On startup, discover existing `tws_*` tmux sessions and associate with projects
- [ ] Periodic refresh (~2s) to catch externally killed sessions

### Key decisions
- Session naming convention: `tws_{collection_slug}_{project_slug}`
- Inside tmux (`$TMUX` set) → `switch-client`; outside → `attach-session`
- Suspend/resume pattern: restore terminal before tmux attach, re-enter alternate screen after
- tmux interaction abstracted behind `tmux/commands.rs`

### New files
`tmux/{mod,commands}.rs`

### Verification
- Launch session from project, verify in `tmux ls`
- Attach, detach, kill — all work correctly
- `cargo test` for slug generation and session reconciliation

---

## Phase 4: Visual Polish + UX
**Status:** `pending`

**Goal:** Production-quality look and feel.

### Deliverables
- [ ] Styled tree: icons/prefixes for collections (◆), projects (▸), sessions (●)
- [ ] Color differentiation between entity types and states
- [ ] Context-sensitive status bar with keybinding hints
- [ ] Help overlay (`?` key) with full keybinding reference
- [ ] Filter mode (`/` key) to narrow the tree to matching items

### New files
`components/{status_bar,help_overlay}.rs`

### Verification
- Visual inspection across terminal emulators
- Snapshot tests using Ratatui `TestBackend`

---

## Phase 5: Config, Error Handling, Distribution
**Status:** `pending`

**Goal:** Production readiness.

### Deliverables
- [ ] `~/.config/tws/config.toml` for optional settings (default shell, theme overrides)
- [ ] CLI flags: `--help`, `--version`, `--import` (import existing tmux sessions)
- [ ] Graceful error handling (tmux not found, corrupt state file, etc.)
- [ ] Signal handling (SIGTERM/SIGINT → save state + restore terminal)
- [ ] README, CI (GitHub Actions)

### New dependencies
`clap`, `toml`

### Verification
- `tws --help` and `tws --version` work
- `tws --import` imports existing tmux sessions
- Corrupt state file recovery works
- CI green on Linux + macOS

---

## Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| Terminal corruption on panic during tmux attach | RAII terminal restore + `catch_unwind` wrapper |
| `tui-tree-widget` doesn't support per-node-type styling | Evaluate in Phase 1; fallback to hand-rolled tree |
| Session name collisions across collections | Include collection slug in session name |
| Rename while session running | Session keeps old tmux name; reconcile by stored name |
