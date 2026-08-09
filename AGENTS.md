# AGENTS.md

This file provides guidance to agents working in this repository.

## Build & Test

```bash
cargo build                    # compile
cargo test                     # run the full test suite
cargo test state::tests        # run tests in a specific module
cargo test resolve_selection   # run tests matching a name pattern
```

## CI

`.github/workflows/ci.yml` runs on every PR and every push to `main`. Four jobs, all required to pass:

```bash
cargo test   --all-targets --locked                  # Test
cargo fmt    --all --check                            # Rustfmt — formatting
cargo clippy --all-targets --locked -- -D warnings    # Clippy — lints as errors
cargo audit                                           # Security audit — RustSec advisories
```

Run `cargo fmt --all` and `cargo clippy --all-targets -- -D warnings` before pushing. CI pins the toolchain to **Rust 1.96.1** (see `RUST_VERSION` in `ci.yml`); rustfmt reflows and clippy lints shift between releases, so format/lint with a matching toolchain to avoid CI turning red on formatting alone.

## Workflow

Every change — even a tiny one — happens in a **fresh git worktree on a new branch**, then goes out as a PR. Never edit and commit directly on `main` in the primary checkout.

```bash
git worktree add ../tws-<slug> -b <branch> origin/main
cd ../tws-<slug>
# ...edit, commit, push, gh pr create
```

This keeps in-flight work isolated from `main`, and ensures every change is reviewable on GitHub before it lands.

## Release

Patch/minor/major bumps follow semver.

1. Land fix/feature commits on `main` (merge any feature worktrees in first — do **not** bump versions inside a feature branch/worktree, it causes `Cargo.lock` conflicts on merge).
2. In the primary `main` checkout, bump `version` in both `Cargo.toml` and `Cargo.lock` (the `[[package]] name = "tws"` entry) in a separate commit titled `version bump to vX.Y.Z`.
3. Lightweight tag at the bump commit: `git tag vX.Y.Z`. Push both: `git push origin main && git push origin vX.Y.Z`.

Don't use `--follow-tags` (only pushes annotated tags) or `--tags` (pushes all local tags, including any forgotten experimental ones).

## What This Is

tws is a standalone Rust TUI that replaces tmux's `prefix+s` session picker. It adds a persistent organizational hierarchy on top of ephemeral tmux sessions:

```
Collection → Thread → Session(s)
```

Collections and threads are user-created, persisted to `~/.config/tws/state.json`. Sessions are live tmux sessions discovered at runtime. Agent sessions (Claude Code, Codex) are detected by scanning tmux panes and matching child process names.

## Architecture

**Single-threaded event loop** in `app.rs` — the brain of the app. It owns the `Mode` state machine, key routing, rendering, and all side effects. The loop polls keys every 250ms and refreshes tmux sessions on a 30s floor, plus immediately whenever an agent hook fires (see [Agent status protocol](#agent-status-protocol)).

### Mode state machine

```
Mode::Normal → Mode::Input { purpose, buffer } → confirm → back to Normal
             → Mode::Confirm { purpose }       → confirm → back to Normal
             → Mode::Finder { ... }            → select  → back to Normal
```

`InputPurpose` and `ConfirmPurpose` enums capture *what* the modal is for (add collection, rename thread, kill session, etc.) at open time. On confirm, the purpose is consumed via `std::mem::replace` to avoid borrow conflicts on `self.mode`.

### Selection resolution

The tree widget (`tui-tree-widget`) uses UUID strings as node identifiers. `state.rs::resolve_selection()` maps a `&[String]` tree path into `SelectedItem` — an enum with variants `None | Collection(idx) | Thread(col, thread) | Session(col, thread, sess) | Agent(col, thread, sess, agent)`. This is the bridge between the tree widget and the domain model.

### Key modules

| Module | Role |
|---|---|
| `app.rs` | Main loop, mode state machine, key routing, rendering |
| `core/model.rs` | Data structs: Collection, Thread, Session, AgentSession, AgentType |
| `core/state.rs` | AppState, CRUD methods, `resolve_selection()`, session/agent lookups |
| `core/persistence.rs` | JSON save/load to `~/.config/tws/` (state + UI state) |
| `core/notes.rs` | File-based notes stored as `.md` in `~/.config/tws/notes/` |
| `tmux/commands.rs` | Thin wrappers around `tmux` CLI subcommands via `std::process::Command` |
| `tmux/agent_scan.rs` | Detect AI agents by `tmux list-panes` + `ps -e`, match child process names |
| `components/` | Stateless render functions: tree_view, input_modal, confirm_modal, finder_modal, notes_sidebar, status_bar, recent_bar |
| `theme.rs` | All `Style` constants — warm palette (orange collections, tan threads, sage green sessions) |

### Rendering

Immediate-mode: all widgets are rebuilt from `AppState` each frame. Components are stateless functions (`fn render(frame, state, area)`), not structs. `ratatui` diffs the output for efficiency.

### tmux integration

- Sessions are launched detached (`tmux new-session -d`), then attached via `switch-client` (inside tmux) or `attach-session` (outside tmux)
- Agent detection: `tmux list-panes -a` gets pane PIDs → `ps -e` finds child processes → match against known agent binaries (`claude`, `codex`)
- Agent renames are in-memory only (not persisted), preserved across scan refreshes via a `renamed` flag and HashMap snapshot/restore in `do_agent_scan()`

### Agent status protocol

Agents report state through the filesystem. A hook writes one word (`working` / `waiting` / `review`) to `~/.config/tws/agents/$TMUX_PANE`, then touches `~/.config/tws/agent.trigger`. tws polls that trigger's mtime every 250ms (`AgentTrigger` in `core/status.rs`) and rescans when it moves.

A hook writes only when the word *changes*, so mtime is the state-entry time that `status_since` displays. `PreToolUse` is the exception: it refreshes mtime on every tool call even when the word is unchanged, giving `expire_stale_working()` a liveness heartbeat. For `working` panes, then, mtime means last-activity rather than state-entry.

Two further properties keep this responsive, and both are easy to break:

**Every state needs an exit event.** tws can only be as fresh as the hooks that fire. `working` is asserted by `UserPromptSubmit`, `PreToolUse` *and* `PostToolUse` (Claude and Codex alike); Pi's extension gets the same signal from `turn_start`. `PostToolUse` is the "turn resumed" event, and it is the one that is easy to forget. Without it, leaving `waiting` — a permission granted, an `AskUserQuestion` answered — waits on the model reaching its *next* tool call, which is unbounded: measured at 8s in a busy session and 18 hours against an idle one. When adding a state, ask what event returns the agent *out* of it, and whether that event is bounded by something other than the model's own choice to act.

**Scans snapshot the trigger before reading statuses.** `do_agent_scan()` reads the trigger mtime up front and acknowledges *that* value at the end. Reading it fresh at the end instead would mark a hook that fired mid-scan as seen while its status went unread, stranding the agent until its next hook. `prune_stale_files()` has the mirror-image guard: it keeps files written since the scan began, since an agent that spawned mid-scan is missing from the pane snapshot but is very much running.

Hook wiring lives in `install.sh` (`status_hook_entry`). Editing it does **not** reach existing installs — the mappings are copied into `~/.claude/settings.json` at install time, so protocol changes require re-running `install.sh`.

## Tests

All tests are in-file `#[cfg(test)]` modules, not in a separate `tests/` directory. Coverage focuses on model construction, persistence round-trips, CRUD operations, selection resolution, and agent scan parsing. tmux command wrappers are not unit-tested (side-effectful).

## CLI

```
tws              # launch TUI (default)
tws import       # interactive import of unmanaged tmux sessions
```

Detach from a session with `prefix + d` to return to the shell.

## Comments

Keep code self-explanatory through clear names, structure, and small functions. Do not add comments that merely restate what the code does or provide broad project context.

Add a comment only when it captures useful information that cannot be understood easily from the code. Comments may explain non-obvious implementation choices, necessary workarounds, invariants, subtle constraints, ownership or lifecycle details, externally imposed behavior, important edge cases, or non-obvious public API behavior.

Prefer explaining “why” over “what.” Keep comments short, specific, and next to the code they describe. Update or remove comments when the associated code changes.
