//! Experimental Rex backend. A thread is a Rex session labeled with the thread
//! slug, and a tws session is a window (tab) in it. tws keeps its
//! `twsr_<thread>_<label>` names as keys and maps them to Rex IDs on each call.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::core::model::{AgentSession, AgentStatus};
use crate::tmux::agent_scan::{self, PaneInfo};

/// `TWS_BACKEND=rex` or `tmux` decides. Without it, a Rex terminal picks Rex,
/// unless tmux runs inside that terminal.
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("TWS_BACKEND") {
        Ok(v) => v == "rex",
        Err(_) => std::env::var_os("REX_SESSION").is_some() && std::env::var_os("TMUX").is_none(),
    })
}

/// One `rex do` call reads every session, window and terminal. Separate CLI
/// calls cost about 20 ms each, which is a visible stall at 50 calls a scan.
/// A view needs an attached session, so the script attaches to each one.
/// Rex encodes an empty Lua table as `{}`, so the script adds no empty lists.
const SNAPSHOT_LUA: &str = r#"
local T = "com.superlogical.terminal"
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
        local rec = st and st.records and st.records[1]
        blocks[#blocks + 1] = {block_id = b.block_id, pid = p and p.child and p.child.pid,
          title = t and t.title, state = rec and rec.state}
      end
    end
    if #blocks > 0 then
      ws[#ws + 1] = {window_id = w.window_id, label = w.label,
        focused_block_id = w.focused_block_id, blocks = blocks}
    end
  end
  if #ws > 0 then
    out[#out + 1] = {session_id = s.session_id, label = s.label, windows = ws}
  end
end
return out
"#;

#[derive(Debug, Deserialize)]
struct RexSession {
    session_id: String,
    #[serde(default)]
    label: String,
    windows: Vec<RexWindow>,
}

#[derive(Debug, Deserialize)]
struct RexWindow {
    window_id: String,
    #[serde(default)]
    label: String,
    focused_block_id: Option<String>,
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

fn rex(args: &[&str]) -> Option<String> {
    let out = Command::new("rex").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `output()`, not `status()`: rex must not print into the TUI.
fn rex_ok(args: &[&str]) -> std::io::Result<bool> {
    Ok(Command::new("rex").args(args).output()?.status.success())
}

/// An empty result is `{}`, not `[]`, so a failed parse means no sessions.
fn parse_snapshot(json: &str) -> Vec<RexSession> {
    serde_json::from_str(json).unwrap_or_default()
}

fn snapshot() -> Vec<RexSession> {
    rex(&["do", "-e", SNAPSHOT_LUA, "--json"])
        .map(|json| parse_snapshot(&json))
        .unwrap_or_default()
}

fn tws_name(session: &RexSession, window: &RexWindow) -> String {
    format!("twsr_{}_{}", session.label, window.label)
}

/// `twsr_<thread>_<label>` -> (thread, label). Slugs use `-`, never `_`.
fn split_name(name: &str) -> Option<(&str, &str)> {
    name.strip_prefix("twsr_")?.split_once('_')
}

fn find<'a>(sessions: &'a [RexSession], name: &str) -> Option<(&'a RexSession, &'a RexWindow)> {
    sessions.iter().find_map(|s| {
        let w = s.windows.iter().find(|w| tws_name(s, w) == name)?;
        Some((s, w))
    })
}

pub fn installed() -> bool {
    Command::new("rex").arg("--version").output().is_ok()
}

/// Rex has no last-attached time, so the recent bar stays empty.
pub fn list_tws_sessions_with_timestamps() -> Vec<(String, i64)> {
    snapshot()
        .iter()
        .flat_map(|s| s.windows.iter().map(move |w| (tws_name(s, w), 0)))
        .collect()
}

pub fn new_session(name: &str, cwd: Option<&Path>) -> std::io::Result<bool> {
    let Some((thread, label)) = split_name(name) else {
        return Ok(false);
    };
    let cwd = cwd.map(|d| d.to_string_lossy().into_owned());
    let sessions = snapshot();
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
    let sessions = snapshot();
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
    let sessions = snapshot();
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

/// A thread rename is one Rex session rename; every window follows.
pub fn rename_thread(old_slug: &str, new_slug: &str) {
    if let Some(s) = snapshot().iter().find(|s| s.label == old_slug) {
        let _ = rex_ok(&["session", "rename", &s.session_id, new_slug]);
    }
}

pub fn active_pane(name: &str) -> Option<String> {
    let sessions = snapshot();
    find(&sessions, name)?.1.focused_block_id.clone()
}

pub fn select_pane(block_id: &str) -> std::io::Result<bool> {
    rex_ok(&["block", "focus", block_id])
}

/// Shows the window in the Rex app. tws keeps running: unlike tmux, the app
/// shows it next to tws, so there is nothing to exit or attach.
pub fn show(name: &str) -> std::io::Result<bool> {
    let sessions = snapshot();
    let Some((s, w)) = find(&sessions, name) else {
        return Ok(false);
    };
    // ponytail: first app client wins; pick by label when several Macs connect.
    let Some(client) = app_client() else {
        return Ok(false);
    };
    let sid = format!("session_id={}", s.session_id);
    let wid = format!("window_id={}", w.window_id);
    rex_ok(&["-C", &client, "do", "session.select", &sid, &wid])
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

/// VT keeps the colors. It holds the scrollback too, which the preview scrolls.
pub fn capture_pane(block_id: &str) -> Option<String> {
    rex(&["capture", "-b", block_id, "--format=vt"])
}

pub fn scan_agents(tws_sessions: &[String]) -> Vec<AgentSession> {
    let wanted: HashSet<&str> = tws_sessions.iter().map(String::as_str).collect();
    let mut panes = Vec::new();
    for s in snapshot() {
        for w in &s.windows {
            let name = tws_name(&s, w);
            if !wanted.contains(name.as_str()) {
                continue;
            }
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
    agent_scan::match_panes(&panes)
}

/// Program status (OSC 7501) replaces the hook files under Rex. Only agents
/// that report it get a status; the others stay Unknown.
pub fn load_statuses() -> HashMap<String, (AgentStatus, i64)> {
    statuses_from(&snapshot())
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
                       {"block_id":"block:2","pid":43}]}]}]"#;
        let sessions = parse_snapshot(json);
        let (s, w) = find(&sessions, "twsr_tws_rex-backend").unwrap();
        assert_eq!(s.session_id, "session:1");
        assert_eq!(w.focused_block_id.as_deref(), Some("block:1"));
        let statuses = statuses_from(&sessions);
        assert_eq!(statuses["block:1"].0, AgentStatus::Waiting);
        assert!(!statuses.contains_key("block:2"));
        assert!(parse_snapshot("{}").is_empty());
        assert_eq!(
            split_name("twsr_tws_rex-backend"),
            Some(("tws", "rex-backend"))
        );
    }
}
