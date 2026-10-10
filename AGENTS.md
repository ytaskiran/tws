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

tws is a standalone Rust TUI that organizes tmux sessions into threads. It adds a persistent layer on top of ephemeral tmux sessions:

```
Thread → Session(s)
```

Threads are user-created and persist to `~/.config/tws/state.json` as a plain array. Sessions are live tmux sessions that tws finds at runtime. A session name is `twsr_<thread-slug>_<label>`. tws detects agent sessions (Claude Code, Codex, Pi) when it scans the process tree of each tmux pane.

## Architecture

**Single-threaded event loop** in `app.rs` — the brain of the app. It owns the `Mode` state machine, key and mouse routing, rendering, and all side effects. The loop waits up to 250ms for input, then handles every queued event before the next draw, so a click hit-tests the frame that you saw. It refreshes tmux sessions on a 30s floor, plus immediately whenever an agent hook fires (see [Agent status protocol](#agent-status-protocol)).

### Mode state machine

```
Mode::Normal → Mode::Input { purpose, buffer } → confirm → back to Normal
             → Mode::Confirm { purpose }       → confirm → back to Normal
             → Mode::Finder { ... }            → select  → back to Normal
```

`InputPurpose` and `ConfirmPurpose` enums capture *what* the modal is for (add thread, rename thread, kill session, etc.) at open time. On confirm, the purpose is consumed via `std::mem::replace` to avoid borrow conflicts on `self.mode`.

### Selection resolution

Selection is a `&[String]` path of identifiers (thread UUIDs, tmux session names, pane IDs), stored in a `tui_tree_widget::TreeState`. `state.rs::resolve_selection()` maps that path into `SelectedItem` — an enum with variants `None | Thread(thread) | Session(thread, sess) | Agent(thread, sess, agent)`. This is the bridge between the UI and the domain model.

The sessions view (`components/sessions_view.rs`) draws its own rows and does not render the `Tree` widget. `TreeState` only learns the row order from a `Tree` render, so its `key_down`/`key_up` do not work here. Navigation uses `sessions_view::row_paths()` and `sessions_view::step()` instead, and a mouse hit uses `sessions_view::path_at()`. Keep the row order in `rows()` only, so the screen, the cursor and the mouse cannot disagree.

Both list views keep their scroll offset between frames (`components::sticky_scroll`): `App` passes the last offset in, and `render` returns the new one. A hover selects a row, so the offset must not change while the selection stays on screen. Each draw also stores `Hits`, the areas of the list and the header tabs, for `sessions_view::path_at()` and `agents_view::agent_at()`. A key, a tab click and an attach from outside tmux clear `Hits`, because each one can change the rows under the pointer.

### Key modules

| Module | Role |
|---|---|
| `app.rs` | Main loop, mode state machine, key and mouse routing, rendering |
| `core/model.rs` | Data structs: Thread, Session, AgentSession, AgentType |
| `core/state.rs` | AppState, CRUD methods, `resolve_selection()`, session/agent lookups |
| `core/persistence.rs` | JSON save/load to `~/.config/tws/` (state + UI state) |
| `core/notes.rs` | File-based notes stored as `.md` in `~/.config/tws/notes/` |
| `tmux/commands.rs` | Thin wrappers around `tmux` CLI subcommands via `std::process::Command` |
| `tmux/agent_scan.rs` | Detect AI agents with `tmux list-panes` + `ps -e`. Search the process tree of each pane |
| `components/` | Stateless render functions: sessions_view, agents_view, input_modal, confirm_modal, finder_modal, notes_sidebar, agent_preview, status_bar, recent_bar |
| `theme.rs` | All `Style` constants — warm palette with an orange accent |

### Rendering

Immediate-mode: all widgets are rebuilt from `AppState` each frame. Components are stateless functions (`fn render(frame, state, area)`), not structs. `ratatui` diffs the output for efficiency.

### tmux integration

- Sessions are launched detached (`tmux new-session -d`), then attached via `switch-client` (inside tmux) or `attach-session` (outside tmux)
- Agent detection: `tmux list-panes -a` gives the pane PIDs. `ps -e` gives the process tree. tws searches breadth-first from `pane_pid` (depth 0 is the pane process) down to depth 3. The shallowest agent wins. Each pane has one agent at most. Agents: Claude Code (also the native `claude/versions/<v>` path), Codex, and Pi. A `node` or `deno` script matches by npm package name, not by directory name.
- Agent renames are in-memory only (not persisted), preserved across scan refreshes via a `renamed` flag and HashMap snapshot/restore in `do_agent_scan()`

### Agent status protocol

Agents report state through the filesystem. A hook writes one word (`working` / `waiting` / `review`) to `~/.config/tws/agents/$TMUX_PANE`, then touches `~/.config/tws/agent.trigger`. tws polls that trigger's mtime every 250ms (`AgentTrigger` in `core/status.rs`) and rescans when it moves.

A hook writes the status file only when the word *changes*, so its mtime is the state-entry time that `status_since` displays. `UserPromptSubmit` is the one exception: it always writes `working`, so for a `working` pane the mtime is the start of the turn. The reason is the Esc. Claude Code fires no `Stop` and no interrupt hook after an Esc, so the file stays `working` with the old turn's mtime. A tool call touches only the heartbeat, so a prompt that wrote only on a word change would give the new turn the age of the old turn. The prompt entries write through `put` (the rename stamps the mtime) and ring the trigger only if the word changed. `PostToolUse ^AskUserQuestion$` keeps the write-on-change rule, because it resumes the same turn.

A tool call in a `working` pane touches a second file, `~/.config/tws/heartbeat/$TMUX_PANE`, and leaves the status file alone. The heartbeat only proves liveness, so the status mtime stays the state-entry time, or the turn-start time for a `working` pane. `expire_stale_working()` takes the heartbeat directory as a parameter and reads the newer of the status mtime and the heartbeat mtime, so a long turn stays `working` while it makes tool calls. Do not touch the status file for liveness: the row would show `now` for the whole turn. `reset` and `SessionEnd` remove the heartbeat of the pane, the Pi `session_shutdown` handler does the same, and the upgrade cleanup clears the directory. tws prunes the heartbeat file of a pane that is not live, with the same scan-start guard as the other directories.

`status_hook_entry` emits one of fourteen command shapes, and picking the wrong one is how this protocol breaks:

| mode | writes | used by |
|---|---|---|
| `set` | unconditionally, when the word differs | `PreToolUse ^AskUserQuestion$`, `PostToolUse ^AskUserQuestion$` |
| `prompt` | always writes `working` through `put`, so the rename stamps a fresh status mtime. It rings the trigger only if the word changed. It also removes the pane's permission key files, because a denied request fires no `Stop`. | Claude and Codex `UserPromptSubmit` |
| `stop` | while a fresh subagent marker exists, `working` (an open `waiting` stays); else `idle` if the pane is in view, else `review`. It also deletes the pane's permission key files and its `m.*` in-flight markers, and keeps `s.*`. With the `keep` argument it deletes no in-flight marker. | Claude `Stop`, `StopFailure`, `PostCompact manual`. Codex `Stop`, `PostCompact manual` use `keep`. |
| `tool` | reads the payload. A main-thread call resumes `working` over `review`, `idle`, or an empty file. A subagent call acts as `live`. Both touch the heartbeat, not the status file. The same `jq` call reads `tool_use_id`, and the hook makes the in-flight marker `m.<id>` (main loop) or `s.<id>` (subagent). | Claude `PreToolUse` (every tool but the question) |
| `live` | touches the heartbeat of a `working` pane, or claims an empty file (the status file then has the entry time) — never overwrites a resting state | no entry uses it now (kept as the base of `begin` and `tool`) |
| `begin` | acts as `live` (it touches the heartbeat), and makes the in-flight marker `m.<tool_use_id>` with one `jq` call | Codex `PreToolUse` |
| `done` | acts as `set`, and removes the in-flight marker of the call with one `jq` call | Codex `PostToolUse` |
| `alert` | raises `waiting` over `working` or an empty file only | `Notification permission_prompt`, Codex `PermissionRequest` |
| `idle_alert` | the same as `alert`, but skipped while a fresh subagent marker exists | `Notification idle_prompt` |
| `reset` | writes `idle` over any state, deletes the pane's subagent markers, permission key files, in-flight markers and heartbeat, rings the trigger only if the word changed. It does nothing while the word is `working` and a fresh marker exists, because a nested agent in the same pane (`claude -p` from a Bash tool) also fires `SessionStart`. | Claude `SessionStart` (`startup\|resume\|clear`) |
| `permit` | writes a key file for the request, then raises `waiting` with the `alert` rules | Claude `PermissionRequest ^(?!AskUserQuestion$).*` |
| `granted` | removes the key file of the finished call. If the pane is `waiting` and no key file is left, writes `working`. It also removes the in-flight marker of the call. It always starts one `jq` call, which gives the `tool_use_id` and the key input. If the pane has no key file, it exits before it hashes anything. | Claude `PostToolUse ^(?!AskUserQuestion$).*`, `PostToolUseFailure` |
| `interrupt` | writes `idle` over any state, and rings the trigger only if the word changed. It deletes the pane's subagent markers and in-flight markers. It starts no `jq` and no `tmux`. | Codex `Interrupt` |
| `rest` | changes `review` or an empty file to `idle`; leaves `working`, `waiting` and `idle`; keeps the markers | Codex `SessionStart` (`startup\|resume\|clear`) |

A new agent session starts `idle`. Without this, a new agent in a pane inherits the status file of the last agent there, for example a stale `review`, until its first hook. `/clear` in a pane in `review` would keep `review`. Claude `SessionStart` uses `reset` mode with the matcher `startup|resume|clear`. Do not add `compact`, which fires in the middle of a session, or `fork`. Codex also fires `SessionStart` when a subagent starts, so its `rest` mode cannot overwrite `working` or `waiting`, and it cannot clear markers. The Pi extension writes `idle` on `session_start`, and it keeps the pane's word on `reload`: `session_start` skips that reason, and `session_shutdown` does not delete the pane file for it. The fork pointer keeps its own `SessionStart` entry, next to the `reset` entry.

A manual `/compact` runs outside a turn, and an auto compaction runs inside one. Manual `/compact` fires `PreCompact` (trigger `manual`), `SubagentStop`, `SessionStart` (source `compact`) and `PostCompact` (trigger `manual`). It fires no `UserPromptSubmit` and no `Stop`, so `PostCompact manual` uses `stop` mode to end it. There is no `PreCompact` write. A `/compact` that is cancelled (Esc), fails (prompt too long, no summary, API error) or is blocked by another `PreCompact` hook fires no `PostCompact` and no `Stop`. A `working` from `PreCompact` would have no exit event, and `idle_prompt` does not reliably heal it, so the pane would show `working` for up to 15 minutes while the agent waits at its prompt. The protocol rule is that every state needs a bounded exit. A manual `/compact` therefore shows no `working` state, and `install.sh` removes an old tws `PreCompact` entry on re-install. An auto compaction happens in the middle of a turn, and the `Stop` of that turn ends it. Auto compaction has no hook: a `PostCompact` write would show `review` while the agent still works. Every compaction matcher is exactly `manual`. In Pi, `session_compact` carries a `reason` (`manual`, `threshold` or `overflow`). The extension writes the turn-end word only for `manual`, because `agent_settled` waits for the other two.

Eight further properties keep this correct, and all eight are easy to break:

**A pane has more than one writer.** Hooks are keyed on `$TMUX_PANE`, but a background subagent runs in the same pane as the main loop and fires the same tool hooks. A Claude tool hook payload carries `agent_id` only inside a subagent. So `tool` mode tells the two apart with one `jq` call. A subagent call cannot start a turn, so it behaves as `live`. A main-thread call can, because the main loop only calls a tool when its turn is live. Only `waiting` survives a main-thread call, because a background subagent can hold the pane there for its permission prompt. Before this rule, `Stop` set `review` and the subagent's next tool call repainted the pane `working` three seconds later, hiding exactly the pane that needed you. Run `bash scripts/verify-agent-hooks.sh` after touching any of this; it drives the generated commands and asserts the words.

**A running subagent keeps the pane working.** `SubagentStart` creates `~/.config/tws/subagents/$TMUX_PANE/<agent_id>`, `SubagentStop` removes it, and each subagent tool call touches it. `Stop` and `StopFailure` write `working`, not `review`, while any marker is newer than `SUBAGENT_FRESH_MINS` (15). An open `waiting` survives them, as it survives `tool` mode, because the subagent can still be blocked on its permission prompt. `Stop` also deletes older markers, since a subagent that dies leaves no `SubagentStop`. `idle_prompt` does not raise `waiting` while a fresh marker exists. The window equals `STALE_WORKING_SECS`, and a Rust test checks that the two agree. A `SubagentStop` with no marker is normal (compaction sends one) and does nothing. Background Bash tasks and monitors are not subagents, and they must not count: a dev server can run for hours. tws prunes the marker directory of a pane that is not live. The `find -mmin`, `-delete`, `stat`, and `touch` calls must work on both BSD and GNU.

**The views show the count of fresh markers.** tws counts the markers of each agent pane that are newer than `STALE_WORKING_SECS` (`apply_subagent_counts`) on every 250ms tick, with no trigger. A recount is one small `readdir` per pane, and it sees every marker change, whichever hook made it. Do not add a trigger for it: each hook that changes a marker would then have to ring it. A subagent killed by ESC still counts until its marker is 15 minutes old, and a subagent inside one tool call longer than 15 minutes drops out until its next call. The count does not read the `s.*` in-flight markers to fix the second case, because ESC leaves those for up to `MAX_TOOL_SECS`.

**A tool call in flight keeps the pane working.** A tool call that runs longer than `STALE_WORKING_SECS` (a long test run, a build, a poll loop) sends no heartbeat, so `expire_stale_working()` would write `idle` while the tool still runs. `PreToolUse` makes the marker `~/.config/tws/inflight/$TMUX_PANE/<prefix>.<tool_use_id>`. The prefix is `m` for the main loop and `s` for a Claude subagent (`agent_id` present). Codex uses `m`, because its documented payload has a `tool_use_id` and no `agent_id`: Codex cannot tell a subagent call from a main-loop call. `PostToolUse` and `PostToolUseFailure` remove the marker of their call; they try both prefixes, because the id is unique. Both use the one `jq` call that the hook already needs, so no hook starts `jq` twice. Only a `tool_use_id` that matches `[A-Za-z0-9_-]+` makes or removes a marker. The hooks decode the id in two ways (`@tsv` in Claude `tool`, a `-c` JSON string in `granted`), and an id with a quote or a control character would get a marker that `granted` never removes. An empty id or any other id does nothing, and so does a failed `jq`. Claude `Stop`, `StopFailure` and `PostCompact manual` remove `m.*` and keep `s.*`, because background subagents work on after the main loop ends its turn. Codex `Stop` and `PostCompact manual` (`stop` mode with `keep`) remove no marker. A Codex subagent call has the `m` prefix, so a Codex `Stop` that removed `m.*` would delete the marker of a subagent that still runs a long tool, and tws would expire the pane to `idle` after 15 minutes. A Codex marker therefore ends only at its `PostToolUse`, at `Interrupt`, at `SessionEnd`, or at the 4 h cap. `SessionStart` (`reset`, unless it skips a busy pane), `SessionEnd` and the Codex `Interrupt` clear the whole directory. `expire_stale_working()` takes the in-flight directory as a parameter and does not expire a `working` pane that has a marker newer than `MAX_TOOL_SECS` (4 hours). The cap bounds the harm from a marker that outlives its call: an ESC in Claude Code fires no `PostToolUse` and no `Stop`. `idle_prompt` heals such a pane after 60 s, and the cap covers the rest. tws prunes the in-flight directory of a pane that is not live, with the same scan-start guard as the other directories.

**Every state needs an exit event.** tws can only be as fresh as the hooks that fire. `working` is asserted by `UserPromptSubmit` and — only to leave a wait — `PostToolUse` after a question or a permission grant; Pi's extension gets the same signal from `turn_start`. `PostToolUse ^AskUserQuestion$` is the "turn resumed" event, and it is the one that is easy to forget. Without it, leaving `waiting` waits on the model reaching its *next* tool call, which is unbounded: measured at 8s in a busy session and 18 hours against an idle one. A permission grant needs a pairing, because `PermissionRequest` carries no `tool_use_id`. `PermissionRequest` writes a key file, `~/.config/tws/permissions/$TMUX_PANE/<key>`. The key is a `cksum` of `jq -cS '{tool_name, tool_input}'`. Claude gives the same `tool_name` and `tool_input` to `PostToolUse` when the tool runs, and this was checked with a logger hook, in the main loop and in a subagent. `PostToolUse` and `PostToolUseFailure` remove the key of their own call. When the pane is `waiting` and no key file is left, they write `working`. A call with no key file changes nothing, so a tool that never asked cannot end a wait. The hook starts one `jq` call for every finished tool (it also needs the `tool_use_id` for the in-flight marker), but a pane with no key file exits before it runs `cksum`, which is the case for almost every tool call. A denied tool fires neither event. An interrupt (Esc at the dialog) or a "No" with no feedback aborts the turn without a `Stop`, so its key file stays and the pane stays `waiting`. The next prompt clears it: `UserPromptSubmit` removes the directory, because a new prompt means the user answered every request of the last turn. `Stop`, `StopFailure`, `SessionStart` and `SessionEnd` clear the directory too. `PermissionRequest` also fires for `AskUserQuestion`, and no hook removes that key. So `permit` has the same matcher as `granted`, and it skips the question. `Notification permission_prompt` stays as the backstop. `Notification` `idle_prompt` is the backstop: Claude sends it 60s after the main loop goes quiet, and it heals any pane still claiming `working` — unless a fresh subagent marker shows that work goes on. An ESC interrupt fires no `Stop`, so it needs its own exit event. Codex has one: `Interrupt` runs when the user interrupts an active turn on the main thread. It does not run for idle threads or for subagents. It uses `interrupt` mode and writes `idle`, not `review`, because the turn gave no result. It also ends a Codex approval wait: `PermissionRequest` uses `alert` and writes no key file. It removes the pane's in-flight markers, because a running tool dies with the turn. It removes the pane's subagent markers. If it kept them, and Codex had stopped the subagents, the `Stop` of the next turn would see a fresh marker and write `working` instead of `review` for up to 15 minutes. If a subagent survives the interrupt, the next Codex `PostToolUse` repaints `working` within seconds, so the failure is safe. The Codex hook timeout is 1 s, so the command must stay free of `jq` and `tmux`. Claude Code has no interrupt hook, so `idle_prompt` stays the backstop there. Stale expiry still covers hard kills and Codex API errors. When adding a state, ask what event returns the agent *out* of it, and whether that event is bounded by something other than the model's own choice to act.

**A turn that ends in the visible pane is read.** Every turn-end `review` write asks tmux about the caller's own pane first: `tmux display-message -p -t "$TMUX_PANE" '#{pane_active}#{window_active}#{session_attached}'`. The pane is in view when the answer starts with `11` and the last flag is 1 or more. The hook then writes `idle` and not `review`, so the user does not need to leave and attach again to clear it. This covers Claude `Stop`, `StopFailure`, and `PostCompact manual`, Codex `Stop` and `PostCompact manual`, and Pi `agent_settled` and a manual `session_compact`. Any tmux failure (no server, no binary, an odd answer) means `review`. A live subagent marker is checked first, so a pane with live subagents stays `working` (or keeps an open `waiting`) even when it is in view. Known limit: tmux does not know if the terminal window has focus. A pane that is in view in a background terminal counts as read.

**`$TMUX_PANE` is the only pane identity, and a hook without one must stay silent.** The file name *is* the sender's identity, so a wrong name is an undetectable forged write. A query scoped with `-t "$TMUX_PANE"` reads facts *about* the caller's own pane, and it is allowed. Never fall back to an unscoped `tmux display-message -p "#{pane_id}"`: that answers for the current client's *active* pane, not the caller's, so an agent outside a pane stamps whichever pane the user is watching — and that pane, being idle, fires no hook of its own to correct it. Agents do run without `TMUX_PANE`: Claude Code's background sessions (`claude daemon run` → `bg-pty-host` → `bg-spare`) carry neither `TMUX` nor `TMUX_PANE`. tws tracks only agents inside panes, so a pane-less agent has nothing to report and must write nothing.

**Scans snapshot the trigger before reading statuses.** `do_agent_scan()` reads the trigger mtime up front and acknowledges *that* value at the end. Reading it fresh at the end instead would mark a hook that fired mid-scan as seen while its status went unread, stranding the agent until its next hook. `prune_stale_files()` has the mirror-image guard: it keeps files written since the scan began, since an agent that spawned mid-scan is missing from the pane snapshot but is very much running.

**Every status write is atomic.** `printf word > "$f"` truncates the file before it writes. A hook that reads in that gap sees an empty file, and `live` mode claims an empty file as `working`. A subagent tool call can then replace a `review` that `Stop` is still writing. So every mode writes through the `put` helper in `status_hook_entry`: it writes a dot temp file in the same directory, then runs `mv -f`. The Pi extension uses `writeFileSync` and `renameSync`. `write_status_to` and `expire_stale_working` use `std::fs::rename`. tws skips names that start with `.` when it reads statuses, and `prune_stale_files` deletes dot files older than 60 s. The heartbeat touch never truncates and never renames, because it writes no word. Never add a bare `> "$f"`: `scripts/verify-agent-hooks.sh` fails on it.

Hook wiring lives in `install.sh` (`status_hook_entry`, `subagent_hook_entry`, `session_end_hook_entry`). A tws hook entry is any entry whose command contains `config/tws/`, so re-runs replace old entries; keep new commands under that path. The installer asks one main question; before it, `scan_plan` can ask `confirm_own_bar` or `confirm_nova_right` about the user's own status bar. `scan_plan` reads only: it finds each agent config (`~/.claude/settings.json`, `~/.codex`, `~/.pi`), the tmux config, a missing `PATH` line, a `prefix+F` conflict, and a missing `glow`. `print_plan` shows every change and every item it leaves alone (with the lines to add by hand when a step cannot run), and `confirm_plan` asks `Apply these changes? [Y/n]` one time. `apply_plan` then runs the steps with no more questions. A "no", or no terminal to read the answer from, changes nothing outside the binary, with one exception: before the question, `migrate_state` silently flattens an old `state.json` that has collections, and keeps `state.json.bak`. It writes through the path, so a symlinked `state.json` stays a link. The scan must start no tmux server: most tmux commands start one, and a new server runs the user's config (plugins, session restore). So the scan probes with `list-sessions` one time (`tmux_live`), and `list-keys` and `source-file` run only when a server already runs. With no server, a config that loads other files or plugins hides their keys, so the fork binding is skipped with a note. Editing the hook wiring does **not** reach existing installs — the mappings are copied into `~/.claude/settings.json` at install time, so protocol changes require re-running `install.sh`. Re-running it is not enough on its own: an agent that is already running holds the hook config it read earlier, so a session started before the upgrade keeps reporting the old protocol until you restart it. Neither gap is visible from the tree, so when a single pane misreports state and the rest look right, check that pane's agent age before suspecting the protocol.

## Tests

All Rust tests are in-file `#[cfg(test)]` modules. Coverage focuses on model construction, persistence round-trips, CRUD operations, selection resolution, and agent scan parsing. tmux command wrappers are not unit-tested (side-effectful).

The hook commands are shell, not Rust, so `cargo test` cannot reach them. `scripts/verify-agent-hooks.sh` runs each generated command against a throwaway `HOME`, with a JSON payload on stdin as Claude sends it, and checks three things: the status words a sequence of events produces, which pane file each command touches, and that a hook runs `jq` at most once. The pane checks supply a fake `tmux` that answers a bare pane query with a pane the caller does not own, so a re-introduced fallback writes somewhere visible instead of failing silently. The same fake answers a query scoped to `%7` with the flags in `FAKE_TMUX_STATE` (`111` in view, `101` window not active, `011` pane not active, `110` no client, `fail` for a failing tmux). The script also fails on any `display-message` call that lacks `-t "$TMUX_PANE"`. CI runs the script as the `hooks` job.

## CLI

```
tws              # launch TUI (default)
tws import       # interactive import of unmanaged tmux sessions
tws ack-pane [PANE_ID]  # review -> idle for one pane; tmux hooks call it
```

`tws ack-pane` reads `$TMUX_PANE` when it has no argument. It changes the pane file only when the word is `review`, and it touches the trigger only after a change. It always exits 0 and prints nothing, because a tmux hook runs it on every focus change. `install.sh` adds the `# tws ack hooks` block to the tmux config (`after-select-pane`, `after-select-window`, `client-session-changed`, `window-pane-changed`, `session-window-changed`, all at index 89). The block goes in whenever an agent hook step succeeds, because the hooks only clear the `review` that those agent hooks write. tmux loads each config file that exists: `~/.tmux.conf`, then `$XDG_CONFIG_HOME/tmux/tmux.conf`, then `~/.config/tmux/tmux.conf`. The tws blocks go into the first one (`tmux_conf_path`), and the checks for the user's own keys read all of them. A config that is a broken symlink or not writable (for example a Nix store link) skips both tmux steps with a note. With no config, the installer creates `~/.tmux.conf`. `load_into_tmux` then sources only the block into a running server, so no manual `source-file` is needed. It does not source the whole config, because that can repeat the user's own commands. The hooks pass `#{pane_id}`, which tmux expands to the pane that receives focus. A `select-pane` on a window that is not on screen also fires the hook, so a script can acknowledge a pane you did not see.

`last-pane`, `kill-pane` and `kill-window` fire none of the three `after-*` and `client-*` hooks. They fire `window-pane-changed` or `session-window-changed`, so those two hooks cover them. Some moves fire two hooks, and the double ack is harmless because `ack-pane` is idempotent. `kill-session` moves the client to another session and fires none of the five hooks. This is a known gap (checked on tmux 3.6a). `install.sh` does not write the block when the binary path has whitespace or one of `'` `"` `\` `$` `#` `;`, because tmux would break the `run-shell` string (`ack_path_is_safe`). It also writes the block back into the config file in place (`rewrite_conf_block`), so a symlinked file and its mode stay.

Detach from a session with `prefix + d` to return to the shell.

## Comments

Keep code self-explanatory through clear names, structure, and small functions. Do not add comments that merely restate what the code does or provide broad project context.

Add a comment only when it captures useful information that cannot be understood easily from the code. Comments may explain non-obvious implementation choices, necessary workarounds, invariants, subtle constraints, ownership or lifecycle details, externally imposed behavior, important edge cases, or non-obvious public API behavior.

Prefer explaining “why” over “what.” Keep comments short, specific, and next to the code they describe. Update or remove comments when the associated code changes.
