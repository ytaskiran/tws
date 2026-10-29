//! The Rex backend, the default. A thread is a Rex session labeled with the thread
//! slug, and a tws session is a window (tab) in it. tws keeps its
//! `twsr_<thread>_<label>` names as keys and maps them to Rex IDs on each call.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use serde::Deserialize;

use crate::core::model::{AgentSession, AgentStatus};
use crate::tmux::agent_scan::{self, PaneInfo};

/// One `rex do` call reads every session, window and terminal. Separate CLI
/// calls cost about 20 ms each, which is a visible stall at 50 calls a scan.
/// A view needs an attached session, so the script attaches to each one.
/// Rex encodes an empty Lua table as `{}`, so the script leaves empty lists
/// out and serde fills them in.
/// A program can report child records (a subagent), so the most urgent state
/// of all records wins: blocked, then working, then done or error.
/// A child record (one with an `id`) that is `working` or `blocked` is a
/// running subagent. Claude Code clears the record when the subagent stops,
/// and Rex drops both states when the process exits.
const SNAPSHOT_LUA: &str = r#"
local T = "com.superlogical.terminal"
local RANK = {blocked = 4, working = 3, done = 2, error = 2}
local out = {}
for _, s in ipairs(rex.call("session.list").sessions) do
  rex.session.attach{session_id = s.session_id}
  local view = rex.session.view{session_id = s.session_id}
  local ws = {}
  for _, w in ipairs(view and view.windows or {}) do
    local blocks = {}
    for _, l in ipairs(w.layers or {}) do
      for _, b in ipairs(l.blocks or {}) do
        local a = {block_id = b.block_id, session_id = s.session_id}
        local p = rex.block.call(T, "process", a)
        local t = rex.block.call(T, "title", a)
        local st = rex.block.call(T, "program_status", a)
        -- The top rank picks the state; in a tie the newest report gives `at`,
        -- so a new subagent `error` after a seen `done` shows again.
        local state, at, subs = nil, nil, 0
        for _, r in ipairs(st and st.records or {}) do
          local rank, top = RANK[r.state] or 1, RANK[state] or 1
          if state == nil or rank > top or (rank == top and (r.updated_at or "") > (at or "")) then
            state, at = r.state, r.updated_at
          end
          if r.id and rank >= 3 then subs = subs + 1 end
        end
        blocks[#blocks + 1] = {block_id = b.block_id, pid = p and p.child and p.child.pid,
          title = t and t.title, state = state, at = at, subagents = subs}
      end
    end
    ws[#ws + 1] = {window_id = w.window_id, label = w.label, focused = w.focused_block_id,
      blocks = #blocks > 0 and blocks or nil}
  end
  out[#out + 1] = {session_id = s.session_id, label = s.label, windows = #ws > 0 and ws or nil}
end
return out
"#;

#[derive(Debug, Deserialize)]
struct RexSession {
    session_id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    windows: Vec<RexWindow>,
}

#[derive(Debug, Deserialize)]
struct RexWindow {
    window_id: String,
    #[serde(default)]
    label: String,
    /// The pane that the app lands on when it shows the window.
    focused: Option<String>,
    #[serde(default)]
    blocks: Vec<RexBlock>,
}

#[derive(Debug, Deserialize)]
struct RexBlock {
    block_id: String,
    pid: Option<u32>,
    #[serde(default)]
    title: String,
    state: Option<String>,
    /// When the program reported `state`. A new report gets a new time.
    at: Option<String>,
    /// Running subagents: live child records.
    #[serde(default, deserialize_with = "lua_count")]
    subagents: usize,
}

/// A Lua number can come back as `1.0`. A strict `usize` would then fail the
/// whole snapshot, so the count reads any number.
fn lua_count<'de, D: serde::Deserializer<'de>>(d: D) -> Result<usize, D::Error> {
    Ok(f64::deserialize(d)? as usize)
}

/// tws runs in a Rex terminal, so it inherits `REX_SESSION` and `REX_BLOCK`.
/// With them, `rex capture -b` and `rex block focus` look only in that one
/// session. tws always passes full IDs, so it removes them.
/// The timeout keeps a hung server from freezing the UI for the 10 s default.
fn command(args: &[&str]) -> Command {
    let mut cmd = Command::new("rex");
    cmd.env_remove("REX_SESSION")
        .env_remove("REX_BLOCK")
        .args(["--timeout", "3s"])
        .args(args);
    cmd
}

fn rex(args: &[&str]) -> Option<String> {
    let out = command(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `output()`, not `status()`: rex must not print into the TUI.
fn rex_ok(args: &[&str]) -> std::io::Result<bool> {
    Ok(command(args).output()?.status.success())
}

/// An empty result is `{}`, not `[]`.
fn parse_snapshot(json: &str) -> Option<Vec<RexSession>> {
    if json.trim() == "{}" {
        return Some(Vec::new());
    }
    serde_json::from_str(json).ok()
}

/// A fresh snapshot, or `None` when Rex did not answer.
fn take() -> Option<Vec<RexSession>> {
    parse_snapshot(&rex(&["do", "-e", SNAPSHOT_LUA, "--json"])?)
}

/// The snapshot of the last refresh. The list, the agent scan and the status
/// load of one refresh all read it, so a refresh costs one `rex do` call.
static CACHE: Mutex<Vec<RexSession>> = Mutex::new(Vec::new());

/// The `ps` output of the last refresh, for the agent scan.
static PROCESSES: Mutex<Option<String>> = Mutex::new(None);

/// Takes a new snapshot for the readers. On a failure it keeps the last one
/// and returns false, so one missed poll does not clear the tree, the pins
/// and the agent names.
/// `ps` does not need the snapshot, so it runs next to `rex do` and the two
/// waits overlap. Each one costs 30 to 200 ms. The last snapshot tells if the
/// agent scan needs `ps`; the first refresh has none, so it runs `ps`. A miss
/// (a first tws pane) runs it after the snapshot.
pub fn refresh() -> bool {
    let expect_panes = cached(|s| s.is_empty() || has_tws_panes(s));
    let (sessions, mut processes) = std::thread::scope(|scope| {
        let ps = expect_panes.then(|| scope.spawn(agent_scan::list_all_processes));
        (take(), ps.and_then(|ps| ps.join().ok().flatten()))
    });
    let Some(sessions) = sessions else {
        return false;
    };
    if !has_tws_panes(&sessions) {
        processes = None;
    } else if processes.is_none() {
        processes = agent_scan::list_all_processes();
    }
    *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = sessions;
    *PROCESSES.lock().unwrap_or_else(|e| e.into_inner()) = processes;
    true
}

/// True when a tws window has a block with a process: the agent scan input.
fn has_tws_panes(sessions: &[RexSession]) -> bool {
    sessions.iter().any(|s| {
        s.windows
            .iter()
            .any(|w| tws_name(s, w).is_some() && w.blocks.iter().any(|b| b.pid.is_some()))
    })
}

fn cached<T>(f: impl FnOnce(&[RexSession]) -> T) -> T {
    f(&CACHE.lock().unwrap_or_else(|e| e.into_inner()))
}

/// A label with `_` would make an ambiguous name, and tws slugs never have
/// one, so such sessions and windows are not tws sessions.
fn tws_name(session: &RexSession, window: &RexWindow) -> Option<String> {
    let ok = |l: &str| !l.is_empty() && !l.contains('_');
    (ok(&session.label) && ok(&window.label))
        .then(|| format!("twsr_{}_{}", session.label, window.label))
}

/// `twsr_<thread>_<label>` -> (thread, label). Slugs use `-`, never `_`.
/// An empty part (a thread name of only symbols) has no Rex label.
fn split_name(name: &str) -> Option<(&str, &str)> {
    let (thread, label) = name.strip_prefix("twsr_")?.split_once('_')?;
    (!thread.is_empty() && !label.is_empty()).then_some((thread, label))
}

fn find<'a>(sessions: &'a [RexSession], name: &str) -> Option<(&'a RexSession, &'a RexWindow)> {
    sessions.iter().find_map(|s| {
        let w = s
            .windows
            .iter()
            .find(|w| tws_name(s, w).as_deref() == Some(name))?;
        Some((s, w))
    })
}

pub fn installed() -> bool {
    crate::core::on_path("rex")
}

/// Rex has no last-attached time, so the recent bar stays empty.
pub fn list_tws_sessions_with_timestamps() -> Vec<(String, i64)> {
    cached(|sessions| {
        sessions
            .iter()
            .flat_map(|s| s.windows.iter().filter_map(move |w| tws_name(s, w)))
            .map(|name| (name, 0))
            .collect()
    })
}

/// Rex accepts duplicate labels, and tmux does not. A duplicate would map two
/// windows to one tws name, so refuse it here as tmux does.
pub fn new_session(name: &str, cwd: Option<&Path>) -> std::io::Result<bool> {
    let Some((thread, label)) = split_name(name) else {
        return Ok(false);
    };
    let Some(sessions) = take() else {
        return Ok(false);
    };
    if find(&sessions, name).is_some() {
        return Ok(false);
    }
    let cwd = cwd.map(|d| d.to_string_lossy().into_owned());
    let mut args = match sessions.iter().find(|s| s.label == thread) {
        Some(s) => vec!["window", "new", "-s", &s.session_id, label, "--focus=false"],
        None => vec!["new", thread, "--window", label],
    };
    if let Some(dir) = &cwd {
        args.extend(["-c", dir]);
    }
    rex_ok(&args)
}

/// Closing the last window leaves an empty Rex session, so kill it instead.
pub fn kill_session(name: &str) -> std::io::Result<bool> {
    let Some(sessions) = take() else {
        return Ok(false);
    };
    let Some((s, w)) = find(&sessions, name) else {
        return Ok(false);
    };
    if s.windows.len() == 1 {
        rex_ok(&["session", "kill", &s.session_id])
    } else {
        rex_ok(&["window", "close", "-s", &s.session_id, &w.window_id])
    }
}

/// Rex cannot move a window to another session, so a move between threads fails.
pub fn rename_session(old_name: &str, new_name: &str) -> std::io::Result<bool> {
    let (Some((old_thread, _)), Some((new_thread, new_label))) =
        (split_name(old_name), split_name(new_name))
    else {
        return Ok(false);
    };
    if old_thread != new_thread {
        return Ok(false);
    }
    let Some(sessions) = take() else {
        return Ok(false);
    };
    if find(&sessions, new_name).is_some() {
        return Ok(false);
    }
    let Some((s, w)) = find(&sessions, old_name) else {
        return Ok(false);
    };
    rex_ok(&[
        "window",
        "rename",
        "-s",
        &s.session_id,
        &w.window_id,
        new_label,
    ])
}

/// A thread rename is one Rex session rename; every window follows. True when
/// there is nothing to rename. False when Rex fails or `new_slug` is taken.
pub fn rename_thread(old_slug: &str, new_slug: &str) -> bool {
    if old_slug == new_slug {
        return true;
    }
    let Some(sessions) = take() else {
        return false;
    };
    let Some(s) = sessions.iter().find(|s| s.label == old_slug) else {
        return true;
    };
    if new_slug.is_empty() || sessions.iter().any(|s| s.label == new_slug) {
        return false;
    }
    matches!(
        rex_ok(&["session", "rename", &s.session_id, new_slug]),
        Ok(true)
    )
}

pub fn select_pane(block_id: &str) -> std::io::Result<bool> {
    rex_ok(&["block", "focus", block_id])
}

/// Shows the window in the Rex app. tws keeps running: unlike tmux, the app
/// shows it next to tws, so there is nothing to exit or attach.
/// The error is a flash message for the user. Rex's own text names the cause,
/// for example Remote Control that is off, so it wins over a fixed one.
/// The last snapshot first: it is what you picked from, and a window closed
/// since then makes `session.select` fail with its own error. A window that
/// tws made after that snapshot (a launch) needs a new one.
pub fn show(name: &str) -> Result<(), String> {
    let ids = |sessions: &[RexSession]| {
        find(sessions, name).map(|(s, w)| (s.session_id.clone(), w.window_id.clone()))
    };
    let (session_id, window_id) = match cached(ids) {
        Some(found) => found,
        None => {
            ids(&take().ok_or("Could not read the Rex sessions")?).ok_or("The session is gone")?
        }
    };
    // ponytail: first app client wins; pick by label when several Macs connect.
    let client = app_client().ok_or("No Rex app is connected")?;
    let sid = format!("session_id={session_id}");
    let wid = format!("window_id={window_id}");
    let out = command(&["-C", &client, "do", "session.select", &sid, &wid])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(error_line(&stderr)
        .unwrap_or("Could not open it — turn on Remote Control in Rex settings")
        .to_string())
}

/// The first line of a rex error, without the `Error: ` prefix of the CLI.
fn error_line(stderr: &str) -> Option<&str> {
    let line = stderr.lines().map(str::trim).find(|l| !l.is_empty())?;
    Some(line.strip_prefix("Error: ").unwrap_or(line))
}

fn app_client() -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(&rex(&["client", "ls", "--json"])?).ok()?;
    json["clients"]
        .as_array()?
        .iter()
        .find(|c| c["info"]["kind"] == "app")?["client_id"]
        .as_str()
        .map(str::to_string)
}

/// tmux captured only the screen. Rex gives the full scrollback, which can be
/// 100,000 lines: too much to parse every few seconds, and past the u16
/// scroll offset of the preview.
const PREVIEW_LINES: usize = 500;

/// VT keeps the colors.
// ponytail: rex still sends the full scrollback; read only the tail with
// `format` + `max_bytes` and `blob_read` if that transfer gets slow.
pub fn capture_pane(block_id: &str) -> Option<String> {
    let vt = rex(&["capture", "-b", block_id, "--format=vt"])?;
    Some(expand_cursor_forward(tail_lines(&vt, PREVIEW_LINES)))
}

/// Rex writes a run of blank cells as `ESC[nC` (cursor forward), where tmux
/// wrote spaces. `ansi_to_tui` drops cursor moves, so turn them into spaces.
/// The skipped cells have no style, so the spaces go out unstyled and the
/// pen (the SGR codes since the last reset) is restored after them.
fn expand_cursor_forward(vt: &str) -> String {
    // A terminal clamps the move at the right margin; no pane is this wide.
    const MAX_MOVE: usize = 1024;
    let mut out = String::with_capacity(vt.len());
    let mut pen = String::new();
    let mut rest = vt;
    while let Some(i) = rest.find("\x1b[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let len = after
            .bytes()
            .take_while(|b| b.is_ascii_digit() || *b == b';')
            .count();
        let params = &after[..len];
        match after.as_bytes().get(len) {
            Some(b'C') if !params.contains(';') => {
                let n = if params.is_empty() {
                    1
                } else {
                    params.parse().unwrap_or(MAX_MOVE).clamp(1, MAX_MOVE)
                };
                out.push_str("\x1b[0m");
                out.extend(std::iter::repeat_n(' ', n));
                out.push_str(&pen);
                rest = &after[len + 1..];
            }
            Some(b'm') => {
                let seq = &rest[i..i + 2 + len + 1];
                if params.is_empty() || params == "0" {
                    pen.clear();
                } else {
                    pen.push_str(seq);
                }
                out.push_str(seq);
                rest = &after[len + 1..];
            }
            _ => {
                out.push_str("\x1b[");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn tail_lines(text: &str, n: usize) -> &str {
    let trimmed = text.trim_end_matches('\n');
    match trimmed.rmatch_indices('\n').nth(n.saturating_sub(1)) {
        Some((i, _)) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// The snapshot changes only here, so the scan sets the subagent counts and
/// the 250 ms recount has nothing to do under Rex.
pub fn scan_agents(tws_sessions: &[String]) -> Vec<AgentSession> {
    let wanted: HashSet<&str> = tws_sessions.iter().map(String::as_str).collect();
    let (panes, counts) = cached(|sessions| panes_from(sessions, &wanted));
    let processes = PROCESSES.lock().unwrap_or_else(|e| e.into_inner());
    let mut agents = agent_scan::match_panes(&panes, processes.as_deref());
    for agent in &mut agents {
        agent.subagents = counts.get(&agent.pane_id).copied().unwrap_or(0);
    }
    agents
}

/// The panes of the tws windows, and the subagent count of each one.
fn panes_from(
    sessions: &[RexSession],
    wanted: &HashSet<&str>,
) -> (Vec<PaneInfo>, HashMap<String, usize>) {
    let mut panes = Vec::new();
    let mut counts = HashMap::new();
    for s in sessions {
        for w in &s.windows {
            let Some(name) = tws_name(s, w).filter(|n| wanted.contains(n.as_str())) else {
                continue;
            };
            for b in &w.blocks {
                let Some(pid) = b.pid else { continue };
                panes.push(PaneInfo {
                    session_name: name.clone(),
                    window_index: 0,
                    pane_id: b.block_id.clone(),
                    pane_pid: pid,
                    pane_title: b.title.clone(),
                });
                counts.insert(b.block_id.clone(), b.subagents);
            }
        }
    }
    (panes, counts)
}

/// Program status (OSC 7501) replaces the hook files under Rex. Only agents
/// that report it get a status; the others stay Unknown.
pub fn load_statuses() -> HashMap<String, (AgentStatus, i64)> {
    let seen = load_seen();
    cached(|sessions| statuses_from(sessions, &seen))
}

/// Rex keeps `done` until the program reports again, and it does not say
/// what the user looked at. So tws remembers the reports that you opened.
fn statuses_from(
    sessions: &[RexSession],
    seen: &HashMap<String, String>,
) -> HashMap<String, (AgentStatus, i64)> {
    sessions
        .iter()
        .flat_map(|s| &s.windows)
        .flat_map(|w| &w.blocks)
        .filter_map(|b| {
            let mut status = map_state(b.state.as_deref()?);
            if status == AgentStatus::Review
                && b.at.is_some()
                && seen.get(&b.block_id) == b.at.as_ref()
            {
                status = AgentStatus::Idle;
            }
            Some((b.block_id.clone(), (status, 0)))
        })
        .collect()
}

/// After an open: the review of the pane you land on counts as seen. A fresh
/// snapshot, so a report that came after the last refresh counts too.
pub fn mark_seen(name: &str, pane: Option<&str>) {
    let Some(sessions) = take() else { return };
    let mut seen = load_seen();
    mark_seen_in(&sessions, name, pane, &mut seen);
    let _ = save_seen(&seen_path(), &seen);
}

/// Atomic: the popup and another tws can read the file while this one writes.
fn save_seen(path: &Path, seen: &HashMap<String, String>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::core::status::write_atomic(path, &format_seen(seen))
}

/// `pane` is the agent that you picked; without it, the app lands on the
/// focused pane of the window. Blocks that are gone drop out.
fn mark_seen_in(
    sessions: &[RexSession],
    name: &str,
    pane: Option<&str>,
    seen: &mut HashMap<String, String>,
) {
    let live: HashSet<&str> = sessions
        .iter()
        .flat_map(|s| &s.windows)
        .flat_map(|w| &w.blocks)
        .map(|b| b.block_id.as_str())
        .collect();
    seen.retain(|id, _| live.contains(id.as_str()));
    let Some((_, w)) = find(sessions, name) else {
        return;
    };
    let Some(id) = pane.or(w.focused.as_deref()) else {
        return;
    };
    if let Some(b) = w.blocks.iter().find(|b| b.block_id == id)
        && b.state.as_deref().map(map_state) == Some(AgentStatus::Review)
        && let Some(at) = &b.at
    {
        seen.insert(id.to_string(), at.clone());
    }
}

/// One `block_id updated_at` line for each seen review.
fn seen_path() -> std::path::PathBuf {
    crate::core::persistence::config_dir().join("rex-seen")
}

fn load_seen() -> HashMap<String, String> {
    std::fs::read_to_string(seen_path())
        .map(|text| parse_seen(&text))
        .unwrap_or_default()
}

fn parse_seen(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(id, at)| (id.to_string(), at.to_string()))
        .collect()
}

fn format_seen(seen: &HashMap<String, String>) -> String {
    let mut lines: Vec<String> = seen.iter().map(|(id, at)| format!("{id} {at}\n")).collect();
    lines.sort();
    lines.concat()
}

/// `done` and `error` wait for the user to look. Rex docs say an unknown
/// state counts as idle.
fn map_state(state: &str) -> AgentStatus {
    match state {
        "working" => AgentStatus::Working,
        "blocked" => AgentStatus::Waiting,
        "done" | "error" => AgentStatus::Review,
        _ => AgentStatus::Idle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_maps_names_and_statuses() {
        let json = r#"[{"session_id":"session:1","label":"tws","windows":[
            {"window_id":"window:1","label":"rex-backend",
             "blocks":[{"block_id":"block:1","pid":42,"title":"t","state":"blocked"},
                       {"block_id":"block:2","pid":43}]},
            {"window_id":"window:2","label":"my_tab"}]},
            {"session_id":"session:2","label":"empty"}]"#;
        let sessions = parse_snapshot(json).unwrap();
        let (s, w) = find(&sessions, "twsr_tws_rex-backend").unwrap();
        assert_eq!(s.session_id, "session:1");
        assert_eq!(w.window_id, "window:1");
        // A label with `_` is not a tws session; an empty session stays known.
        assert!(tws_name(s, &s.windows[1]).is_none());
        assert!(sessions[1].windows.is_empty());
        // Only a tws window with a process needs `ps`.
        assert!(has_tws_panes(&sessions));
        assert!(!has_tws_panes(&sessions[1..]));
        let statuses = statuses_from(&sessions, &HashMap::new());
        assert_eq!(statuses["block:1"].0, AgentStatus::Waiting);
        assert!(!statuses.contains_key("block:2"));
        assert_eq!(parse_snapshot("{}").map(|s| s.len()), Some(0));
        assert!(parse_snapshot("not json").is_none());
        assert_eq!(
            split_name("twsr_tws_rex-backend"),
            Some(("tws", "rex-backend"))
        );
        assert_eq!(split_name("twsr__main"), None);
    }

    #[test]
    fn error_line_takes_rex_message() {
        let stderr = "Error: Rex.app is not accepting actions. Turn on Remote Control in its Settings under Rex Server.\n";
        assert_eq!(
            error_line(stderr),
            Some(
                "Rex.app is not accepting actions. Turn on Remote Control in its Settings under Rex Server."
            )
        );
        assert_eq!(error_line("\n  \n"), None);
        assert_eq!(error_line("plain\nsecond"), Some("plain"));
    }

    #[test]
    fn preview_keeps_the_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\n", 2), "b\nc");
        assert_eq!(tail_lines("a\nb", 5), "a\nb");
    }

    #[test]
    fn preview_expands_cursor_forward_to_spaces() {
        let vt = "a\x1b[1Cb\x1b[Cc\x1b[3C\x1b[1md\x1b[0m";
        assert_eq!(
            expand_cursor_forward(vt),
            "a\x1b[0m b\x1b[0m c\x1b[0m   \x1b[1md\x1b[0m"
        );
    }

    #[test]
    fn preview_gaps_drop_the_pen_and_restore_it() {
        let vt = "\x1b[1m\x1b[48;2;1;2;3ma\x1b[2Cb\x1b[0m\x1b[Cc";
        assert_eq!(
            expand_cursor_forward(vt),
            "\x1b[1m\x1b[48;2;1;2;3ma\x1b[0m  \x1b[1m\x1b[48;2;1;2;3mb\x1b[0m\x1b[0m c"
        );
    }

    #[test]
    fn preview_clamps_huge_moves() {
        assert_eq!(
            expand_cursor_forward("\x1b[99999999999999999999C").len(),
            4 + 1024
        );
    }

    #[test]
    fn subagent_counts_come_from_the_snapshot() {
        // `1.0`: a Lua number may come back as a float.
        let json = r#"[{"session_id":"session:1","label":"tws","windows":[
            {"window_id":"window:1","label":"w",
             "blocks":[{"block_id":"block:1","pid":1,"state":"working","subagents":2},
                       {"block_id":"block:2","pid":2,"subagents":1.0},
                       {"block_id":"block:3","pid":3}]}]}]"#;
        let sessions = parse_snapshot(json).unwrap();
        let (panes, counts) = panes_from(&sessions, &HashSet::from(["twsr_tws_w"]));
        assert_eq!(panes.len(), 3);
        assert_eq!(counts["block:1"], 2);
        assert_eq!(counts["block:2"], 1);
        assert_eq!(counts["block:3"], 0);
    }

    const SEEN_SNAPSHOT: &str = r#"[{"session_id":"session:1","label":"tws","windows":[
        {"window_id":"window:1","label":"w","focused":"block:2",
         "blocks":[{"block_id":"block:1","pid":1,"state":"done","at":"t1"},
                   {"block_id":"block:2","pid":2,"state":"error","at":"t2"},
                   {"block_id":"block:3","pid":3,"state":"working","at":"t3"}]}]}]"#;

    #[test]
    fn a_seen_review_shows_idle_until_a_new_report() {
        let sessions = parse_snapshot(SEEN_SNAPSHOT).unwrap();
        let seen = HashMap::from([
            ("block:1".to_string(), "t1".to_string()),
            ("block:2".to_string(), "older".to_string()),
        ]);
        let statuses = statuses_from(&sessions, &seen);
        assert_eq!(statuses["block:1"].0, AgentStatus::Idle);
        assert_eq!(statuses["block:2"].0, AgentStatus::Review);
        assert_eq!(statuses["block:3"].0, AgentStatus::Working);
    }

    #[test]
    fn an_open_marks_the_landing_pane_seen() {
        let sessions = parse_snapshot(SEEN_SNAPSHOT).unwrap();
        let mut seen = HashMap::from([("block:gone".to_string(), "t0".to_string())]);
        // Without a pane, the app lands on the focused one. A gone block drops out.
        mark_seen_in(&sessions, "twsr_tws_w", None, &mut seen);
        assert_eq!(
            seen,
            HashMap::from([("block:2".to_string(), "t2".to_string())])
        );
        mark_seen_in(&sessions, "twsr_tws_w", Some("block:1"), &mut seen);
        assert_eq!(seen.get("block:1").map(String::as_str), Some("t1"));
        // Only a review is remembered: a working agent has nothing to see.
        mark_seen_in(&sessions, "twsr_tws_w", Some("block:3"), &mut seen);
        assert!(!seen.contains_key("block:3"));
    }

    #[test]
    fn the_seen_file_reads_back_what_it_wrote() {
        let seen = HashMap::from([
            (
                "block:1".to_string(),
                "2026-10-29T15:35:53.454314+03:00".to_string(),
            ),
            ("block:2".to_string(), "t2".to_string()),
        ]);
        assert_eq!(parse_seen(&format_seen(&seen)), seen);
    }

    #[test]
    fn saving_seen_makes_the_folder_and_replaces_the_file() {
        let dir = std::env::temp_dir().join(format!("tws-seen-{}", std::process::id()));
        let path = dir.join("tws").join("rex-seen");
        let first = HashMap::from([("block:1".to_string(), "t1".to_string())]);
        save_seen(&path, &first).unwrap();
        let second = HashMap::from([("block:2".to_string(), "t2".to_string())]);
        save_seen(&path, &second).unwrap();
        assert_eq!(parse_seen(&std::fs::read_to_string(&path).unwrap()), second);
        std::fs::remove_dir_all(&dir).ok();
    }
}
