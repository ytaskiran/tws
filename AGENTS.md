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

`.github/workflows/ci.yml` runs on every PR and every push to `main`. Five jobs, all required to pass:

```bash
cargo test   --all-targets --locked                   # Test
cargo fmt    --all --check                            # Rustfmt — formatting
cargo clippy --all-targets --locked -- -D warnings    # Clippy — lints as errors
cargo audit                                           # Security audit — RustSec advisories
bash scripts/verify-agent-hooks.sh                    # Agent hook protocol — see below
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

Selection is a `&[String]` path of identifiers (collection/thread UUIDs, tmux session names, pane IDs), stored in a `tui_tree_widget::TreeState`. `state.rs::resolve_selection()` maps that path into `SelectedItem` — an enum with variants `None | Collection(idx) | Thread(col, thread) | Session(col, thread, sess) | Agent(col, thread, sess, agent)`. This is the bridge between the UI and the domain model.

The sessions view (`components/sessions_view.rs`) draws its own rows and does not render the `Tree` widget. `TreeState` only learns the row order from a `Tree` render, so its `key_down`/`key_up` do not work here. Navigation uses `sessions_view::row_paths()` and `sessions_view::step()` instead. Keep the row order in `rows()` only, so the screen and the cursor cannot disagree.

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
| `components/` | Stateless render functions: sessions_view, agents_view, input_modal, confirm_modal, finder_modal, notes_sidebar, agent_preview, status_bar, recent_bar |
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

`status_hook_entry` emits one of eleven command shapes, and picking the wrong one is how this protocol breaks:

| mode | writes | used by |
|---|---|---|
| `set` | unconditionally, when the word differs | Codex `UserPromptSubmit`, `PreToolUse ^AskUserQuestion$`, `PostToolUse ^AskUserQuestion$`, `PreCompact manual` (writes `working`) |
| `prompt` | the same as `set`, and it removes the pane's permission key files, because a denied request fires no `Stop` | Claude `UserPromptSubmit` |
| `stop` | while a fresh subagent marker exists, `working` (an open `waiting` stays); else `idle` if the pane is in view, else `review`. It also deletes the pane's permission key files. | `Stop`, `StopFailure`, `PostCompact manual` |
| `tool` | reads the payload. A main-thread call resumes `working` over `review`, `idle`, or an empty file. A subagent call acts as `live`. | Claude `PreToolUse` (every tool but the question) |
| `live` | refreshes `working`, or claims an empty file — never overwrites a resting state | Codex `PreToolUse` |
| `alert` | raises `waiting` over `working` or an empty file only | `Notification permission_prompt`, Codex `PermissionRequest` |
| `idle_alert` | the same as `alert`, but skipped while a fresh subagent marker exists | `Notification idle_prompt` |
| `reset` | writes `idle` over any state, deletes the pane's subagent markers and permission key files, rings the trigger only if the word changed. It does nothing while the word is `working` and a fresh marker exists, because a nested agent in the same pane (`claude -p` from a Bash tool) also fires `SessionStart`. | Claude `SessionStart` (`startup\|resume\|clear`) |
| `permit` | writes a key file for the request, then raises `waiting` with the `alert` rules | Claude `PermissionRequest ^(?!AskUserQuestion$).*` |
| `granted` | removes the key file of the finished call. If the pane is `waiting` and no key file is left, writes `working`. If the pane has no key file, it exits before it starts `jq`. | Claude `PostToolUse ^(?!AskUserQuestion$).*`, `PostToolUseFailure` |
| `rest` | changes `review` or an empty file to `idle`; leaves `working`, `waiting` and `idle`; keeps the markers | Codex `SessionStart` (`startup\|resume\|clear`) |

A new agent session starts `idle`. Without this, a new agent in a pane inherits the status file of the last agent there, for example a stale `review`, until its first hook. `/clear` in a pane in `review` would keep `review`. Claude `SessionStart` uses `reset` mode with the matcher `startup|resume|clear`. Do not add `compact`, which fires in the middle of a session, or `fork`. Codex also fires `SessionStart` when a subagent starts, so its `rest` mode cannot overwrite `working` or `waiting`, and it cannot clear markers. The Pi extension writes `idle` on `session_start`, and it keeps the pane's word on `reload`: `session_start` skips that reason, and `session_shutdown` does not delete the pane file for it. The fork pointer keeps its own `SessionStart` entry, next to the `reset` entry.

A manual `/compact` is a turn, and an auto compaction is not one. Manual `/compact` fires `PreCompact` (trigger `manual`), `SubagentStop`, `SessionStart` (source `compact`) and `PostCompact` (trigger `manual`). It fires no `UserPromptSubmit` and no `Stop`, so `PreCompact manual` writes `working` and `PostCompact manual` uses `stop` mode. An auto compaction happens in the middle of a turn, and the `Stop` of that turn ends it. Auto compaction has no hook: a `PostCompact` write would show `review` while the agent still works. Every compaction matcher is exactly `manual`. In Pi, `session_compact` carries a `reason` (`manual`, `threshold` or `overflow`). The extension writes the turn-end word only for `manual`, because `agent_settled` waits for the other two.

Seven further properties keep this correct, and all seven are easy to break:

**A pane has more than one writer.** Hooks are keyed on `$TMUX_PANE`, but a background subagent runs in the same pane as the main loop and fires the same tool hooks. A Claude tool hook payload carries `agent_id` only inside a subagent. So `tool` mode tells the two apart with one `jq` call. A subagent call cannot start a turn, so it behaves as `live`. A main-thread call can, because the main loop only calls a tool when its turn is live. Only `waiting` survives a main-thread call, because a background subagent can hold the pane there for its permission prompt. Before this rule, `Stop` set `review` and the subagent's next tool call repainted the pane `working` three seconds later, hiding exactly the pane that needed you. Run `bash scripts/verify-agent-hooks.sh` after touching any of this; it drives the generated commands and asserts the words.

**A running subagent keeps the pane working.** `SubagentStart` creates `~/.config/tws/subagents/$TMUX_PANE/<agent_id>`, `SubagentStop` removes it, and each subagent tool call touches it. `Stop` and `StopFailure` write `working`, not `review`, while any marker is newer than `SUBAGENT_FRESH_MINS` (15). An open `waiting` survives them, as it survives `tool` mode, because the subagent can still be blocked on its permission prompt. `Stop` also deletes older markers, since a subagent that dies leaves no `SubagentStop`. `idle_prompt` does not raise `waiting` while a fresh marker exists. The window equals `STALE_WORKING_SECS`, and a Rust test checks that the two agree. A `SubagentStop` with no marker is normal (compaction sends one) and does nothing. Background Bash tasks and monitors are not subagents, and they must not count: a dev server can run for hours. tws prunes the marker directory of a pane that is not live. The `find -mmin`, `-delete`, `stat`, and `touch` calls must work on both BSD and GNU.

**Every state needs an exit event.** tws can only be as fresh as the hooks that fire. `working` is asserted by `UserPromptSubmit` and — only to leave a wait — `PostToolUse` after a question or a permission grant; Pi's extension gets the same signal from `turn_start`. `PostToolUse ^AskUserQuestion$` is the "turn resumed" event, and it is the one that is easy to forget. Without it, leaving `waiting` waits on the model reaching its *next* tool call, which is unbounded: measured at 8s in a busy session and 18 hours against an idle one. A permission grant needs a pairing, because `PermissionRequest` carries no `tool_use_id`. `PermissionRequest` writes a key file, `~/.config/tws/permissions/$TMUX_PANE/<key>`. The key is a `cksum` of `jq -cS '{tool_name, tool_input}'`. Claude gives the same `tool_name` and `tool_input` to `PostToolUse` when the tool runs, and this was checked with a logger hook, in the main loop and in a subagent. `PostToolUse` and `PostToolUseFailure` remove the key of their own call. When the pane is `waiting` and no key file is left, they write `working`. A call with no key file changes nothing, so a tool that never asked cannot end a wait. A pane with no key file exits before it starts `jq`, which is the case for almost every tool call. A denied tool fires neither event. An interrupt (Esc at the dialog) or a "No" with no feedback aborts the turn without a `Stop`, so its key file stays and the pane stays `waiting`. The next prompt clears it: `UserPromptSubmit` removes the directory, because a new prompt means the user answered every request of the last turn. `Stop`, `StopFailure`, `SessionStart` and `SessionEnd` clear the directory too. `PermissionRequest` also fires for `AskUserQuestion`, and no hook removes that key. So `permit` has the same matcher as `granted`, and it skips the question. `Notification permission_prompt` stays as the backstop. `Notification` `idle_prompt` is the backstop: Claude sends it 60s after the main loop goes quiet, and it heals any pane still claiming `working` — unless a fresh subagent marker shows that work goes on. When adding a state, ask what event returns the agent *out* of it, and whether that event is bounded by something other than the model's own choice to act.

**A turn that ends in the visible pane is read.** Every turn-end `review` write asks tmux about the caller's own pane first: `tmux display-message -p -t "$TMUX_PANE" '#{pane_active}#{window_active}#{session_attached}'`. The pane is in view when the answer starts with `11` and the last flag is 1 or more. The hook then writes `idle` and not `review`, so the user does not need to leave and attach again to clear it. This covers Claude `Stop`, `StopFailure`, and `PostCompact manual`, Codex `Stop` and `PostCompact manual`, and Pi `agent_settled` and a manual `session_compact`. Any tmux failure (no server, no binary, an odd answer) means `review`. A live subagent marker is checked first, so a pane with live subagents stays `working` (or keeps an open `waiting`) even when it is in view. Known limit: tmux does not know if the terminal window has focus. A pane that is in view in a background terminal counts as read.

**`$TMUX_PANE` is the only pane identity, and a hook without one must stay silent.** The file name *is* the sender's identity, so a wrong name is an undetectable forged write. A query scoped with `-t "$TMUX_PANE"` reads facts *about* the caller's own pane, and it is allowed. Never fall back to an unscoped `tmux display-message -p "#{pane_id}"`: that answers for the current client's *active* pane, not the caller's, so an agent outside a pane stamps whichever pane the user is watching — and that pane, being idle, fires no hook of its own to correct it. Agents do run without `TMUX_PANE`: Claude Code's background sessions (`claude daemon run` → `bg-pty-host` → `bg-spare`) carry neither `TMUX` nor `TMUX_PANE`. tws tracks only agents inside panes, so a pane-less agent has nothing to report and must write nothing.

**Scans snapshot the trigger before reading statuses.** `do_agent_scan()` reads the trigger mtime up front and acknowledges *that* value at the end. Reading it fresh at the end instead would mark a hook that fired mid-scan as seen while its status went unread, stranding the agent until its next hook. `prune_stale_files()` has the mirror-image guard: it keeps files written since the scan began, since an agent that spawned mid-scan is missing from the pane snapshot but is very much running.

**Every status write is atomic.** `printf word > "$f"` truncates the file before it writes. A hook that reads in that gap sees an empty file, and `live` mode claims an empty file as `working`. A subagent tool call can then replace a `review` that `Stop` is still writing. So every mode writes through the `put` helper in `status_hook_entry`: it writes a dot temp file in the same directory, then runs `mv -f`. The Pi extension uses `writeFileSync` and `renameSync`. `write_status_to` and `expire_stale_working` use `std::fs::rename`. tws skips names that start with `.` when it reads statuses, and `prune_stale_files` deletes dot files older than 60 s. The heartbeat stays `touch -c`, because it does not truncate. Never add a bare `> "$f"`: `scripts/verify-agent-hooks.sh` fails on it.

Hook wiring lives in `install.sh` (`status_hook_entry`, `subagent_hook_entry`, `session_end_hook_entry`). A tws hook entry is any entry whose command contains `config/tws/`, so re-runs replace old entries; keep new commands under that path. Editing it does **not** reach existing installs — the mappings are copied into `~/.claude/settings.json` at install time, so protocol changes require re-running `install.sh`. Re-running it is not enough on its own: an agent that is already running holds the hook config it read earlier, so a session started before the upgrade keeps reporting the old protocol until you restart it. Neither gap is visible from the tree, so when a single pane misreports state and the rest look right, check that pane's agent age before suspecting the protocol.

## Tests

All Rust tests are in-file `#[cfg(test)]` modules. Coverage focuses on model construction, persistence round-trips, CRUD operations, selection resolution, and agent scan parsing. tmux command wrappers are not unit-tested (side-effectful).

The hook commands are shell, not Rust, so `cargo test` cannot reach them. `scripts/verify-agent-hooks.sh` runs each generated command against a throwaway `HOME`, with a JSON payload on stdin as Claude sends it, and checks three things: the status words a sequence of events produces, which pane file each command touches, and that a hook runs `jq` at most once. The pane checks supply a fake `tmux` that answers a bare pane query with a pane the caller does not own, so a re-introduced fallback writes somewhere visible instead of failing silently. The same fake answers a query scoped to `%7` with the flags in `FAKE_TMUX_STATE` (`111` in view, `101` window not active, `011` pane not active, `110` no client, `fail` for a failing tmux). The script also fails on any `display-message` call that lacks `-t "$TMUX_PANE"`. CI runs the script as the `hooks` job.

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
