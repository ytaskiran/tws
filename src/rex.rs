//! The Rex backend, the default. A thread is a Rex session labeled with the thread
//! slug, and a tws session is a window (tab) in it. tws keeps its
//! `twsr_<thread>_<label>` names as keys and maps them to Rex IDs on each call.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::config::Backend;
use crate::core::model::{AgentSession, AgentStatus};
use crate::tmux::agent_scan::{self, PaneInfo};

/// `backend` in config.toml picks the backend, and `TWS_BACKEND` overrides it.
/// `run_tui` loads the config first, so a config that does not parse stops
/// there with its own error before this default applies.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let env = std::env::var("TWS_BACKEND").map(|v| v.to_ascii_lowercase());
        let backend = match env.as_deref() {
            Ok("tmux") => Backend::Tmux,
            Ok("rex") => Backend::Rex,
            _ => crate::config::try_load_config()
                .map(|c| c.backend)
                .unwrap_or_default(),
        };
        backend == Backend::Rex
    })
}

/// One `rex do` call reads every session, window and terminal. Separate CLI
/// calls cost about 20 ms each, which is a visible stall at 50 calls a scan.
/// A view needs an attached session, so the script attaches to each one.
/// Rex encodes an empty Lua table as `{}`, so the script leaves empty lists
/// out and serde fills them in.
/// A program can report child records (a subagent), so the most urgent state
/// of all records wins: blocked, then working, then done or error.
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
        local state
        for _, r in ipairs(st and st.records or {}) do
          if state == nil or (RANK[r.state] or 1) > (RANK[state] or 1) then
            state = r.state
          end
        end
        blocks[#blocks + 1] = {block_id = b.block_id, pid = p and p.child and p.child.pid,
          title = t and t.title, state = state}
      end
    end
    ws[#ws + 1] = {window_id = w.window_id, label = w.label,
      focused_block_id = w.focused_block_id, blocks = #blocks > 0 and blocks or nil}
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
    focused_block_id: Option<String>,
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

/// Takes a new snapshot for the readers. On a failure it keeps the last one
/// and returns false, so one missed poll does not clear the tree, the pins
/// and the agent names.
pub fn refresh() -> bool {
    match take() {
        Some(sessions) => {
            *CACHE.lock().unwrap_or_else(|e| e.into_inner()) = sessions;
            true
        }
        None => false,
    }
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
    Command::new("rex").arg("--help").output().is_ok()
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

pub fn active_pane(name: &str) -> Option<String> {
    let sessions = take()?;
    find(&sessions, name)?.1.focused_block_id.clone()
}

pub fn select_pane(block_id: &str) -> std::io::Result<bool> {
    rex_ok(&["block", "focus", block_id])
}

/// Shows the window in the Rex app. tws keeps running: unlike tmux, the app
/// shows it next to tws, so there is nothing to exit or attach.
/// The error is a flash message for the user.
pub fn show(name: &str) -> Result<(), &'static str> {
    let sessions = take().ok_or("Could not read the Rex sessions")?;
    let (s, w) = find(&sessions, name).ok_or("The session is gone")?;
    // ponytail: first app client wins; pick by label when several Macs connect.
    let client = app_client().ok_or("No Rex app is connected")?;
    let sid = format!("session_id={}", s.session_id);
    let wid = format!("window_id={}", w.window_id);
    match rex_ok(&["-C", &client, "do", "session.select", &sid, &wid]) {
        Ok(true) => Ok(()),
        _ => Err("Could not open it — turn on Remote Control in Rex settings"),
    }
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
fn expand_cursor_forward(vt: &str) -> String {
    let mut out = String::with_capacity(vt.len());
    let mut rest = vt;
    while let Some(i) = rest.find("\x1b[") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if after.as_bytes().get(digits) == Some(&b'C') {
            let n = after[..digits].parse().unwrap_or(1).max(1);
            out.extend(std::iter::repeat_n(' ', n));
            rest = &after[digits + 1..];
        } else {
            out.push_str("\x1b[");
            rest = after;
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

pub fn scan_agents(tws_sessions: &[String]) -> Vec<AgentSession> {
    let wanted: HashSet<&str> = tws_sessions.iter().map(String::as_str).collect();
    let panes = cached(|sessions| {
        let mut panes = Vec::new();
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
                }
            }
        }
        panes
    });
    agent_scan::match_panes(&panes)
}

/// Program status (OSC 7501) replaces the hook files under Rex. Only agents
/// that report it get a status; the others stay Unknown.
pub fn load_statuses() -> HashMap<String, (AgentStatus, i64)> {
    cached(statuses_from)
}

fn statuses_from(sessions: &[RexSession]) -> HashMap<String, (AgentStatus, i64)> {
    sessions
        .iter()
        .flat_map(|s| &s.windows)
        .flat_map(|w| &w.blocks)
        .filter_map(|b| Some((b.block_id.clone(), (map_state(b.state.as_deref()?), 0))))
        .collect()
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
            {"window_id":"window:1","label":"rex-backend","focused_block_id":"block:1",
             "blocks":[{"block_id":"block:1","pid":42,"title":"t","state":"blocked"},
                       {"block_id":"block:2","pid":43}]},
            {"window_id":"window:2","label":"my_tab"}]},
            {"session_id":"session:2","label":"empty"}]"#;
        let sessions = parse_snapshot(json).unwrap();
        let (s, w) = find(&sessions, "twsr_tws_rex-backend").unwrap();
        assert_eq!(s.session_id, "session:1");
        assert_eq!(w.focused_block_id.as_deref(), Some("block:1"));
        // A label with `_` is not a tws session; an empty session stays known.
        assert!(tws_name(s, &s.windows[1]).is_none());
        assert!(sessions[1].windows.is_empty());
        let statuses = statuses_from(&sessions);
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
    fn preview_keeps_the_last_lines() {
        assert_eq!(tail_lines("a\nb\nc\n", 2), "b\nc");
        assert_eq!(tail_lines("a\nb", 5), "a\nb");
    }

    #[test]
    fn preview_expands_cursor_forward_to_spaces() {
        let vt = "a\x1b[1Cb\x1b[Cc\x1b[3C\x1b[1md\x1b[0m";
        assert_eq!(expand_cursor_forward(vt), "a b c   \x1b[1md\x1b[0m");
    }
}
