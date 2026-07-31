use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::core::model::{AgentSession, AgentStatus};
use crate::core::persistence::config_dir;

/// Directory where per-pane status files live: `~/.config/tws/agents/`.
pub fn agents_dir() -> PathBuf {
    config_dir().join("agents")
}

/// Map a status word written by a hook to an `AgentStatus`.
pub fn parse_status(word: &str) -> AgentStatus {
    match word.trim() {
        "working" => AgentStatus::Working,
        "waiting" => AgentStatus::Waiting,
        "review" => AgentStatus::Review,
        "idle" => AgentStatus::Idle,
        _ => AgentStatus::Unknown,
    }
}

/// Read every status file in `dir` into `pane_id -> (status, mtime_epoch_secs)`.
/// Missing/unreadable dir yields an empty map. Unreadable individual files are skipped.
pub fn load_statuses_from(dir: &Path) -> HashMap<String, (AgentStatus, i64)> {
    let mut map = HashMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return map,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let pane_id = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => continue,
        };
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        map.insert(pane_id, (parse_status(&contents), mtime));
    }
    map
}

/// Production entry point: read the real agents dir.
pub fn load_statuses() -> HashMap<String, (AgentStatus, i64)> {
    load_statuses_from(&agents_dir())
}

/// Join loaded statuses onto agents by `pane_id`. Agents with no matching file
/// are set to `Unknown` / `0` (so a removed file resets state on the next scan).
/// Used in a later task for joining status-file data onto live agent sessions.
pub fn apply_statuses(agents: &mut [AgentSession], map: &HashMap<String, (AgentStatus, i64)>) {
    for agent in agents.iter_mut() {
        match map.get(&agent.pane_id) {
            Some(&(status, ts)) => {
                agent.status = status;
                agent.status_since = ts;
            }
            None => {
                agent.status = AgentStatus::Unknown;
                agent.status_since = 0;
            }
        }
    }
}

/// Aggregate counts for the status-bar summary.
pub struct StatusCounts {
    pub working: usize,
    pub waiting: usize,
    pub review: usize,
    pub idle: usize,
}

/// Count agent sessions by their status.
pub fn status_counts(agents: &[AgentSession]) -> StatusCounts {
    let mut c = StatusCounts {
        working: 0,
        waiting: 0,
        review: 0,
        idle: 0,
    };
    for a in agents {
        match a.status {
            AgentStatus::Working => c.working += 1,
            AgentStatus::Waiting => c.waiting += 1,
            AgentStatus::Review => c.review += 1,
            AgentStatus::Idle => c.idle += 1,
            AgentStatus::Unknown => {}
        }
    }
    c
}

/// Delete status files whose `pane_id` is not in the live set (pane died).
/// Missing or unreadable dir is a no-op (not an error).
/// Used in a later task for garbage collecting stale status files.
pub fn prune_stale_files(dir: &Path, live_pane_ids: &HashSet<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !live_pane_ids.contains(name) {
            std::fs::remove_file(&path).ok();
        }
    }
}

/// Map an `AgentStatus` to the word written into a status file — the inverse of
/// `parse_status`. `Unknown` has no on-disk form; it maps to "idle" defensively.
pub fn status_word(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "working",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Review => "review",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}

/// Write `status` as the pane's word into `dir/<pane_id>`, creating `dir` if needed.
pub fn write_status_to(dir: &Path, pane_id: &str, status: AgentStatus) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(pane_id), status_word(status))
}

/// Production entry point: write into the real agents dir. Errors are swallowed —
/// a failed status write must never crash an attach.
pub fn write_status(pane_id: &str, status: AgentStatus) {
    let _ = write_status_to(&agents_dir(), pane_id, status);
}

/// pane_ids of agents in `session_name` currently in `Review` — the panes an
/// attach should acknowledge by writing them back to `idle`.
pub fn agents_to_ack(agents: &[AgentSession], session_name: &str) -> Vec<String> {
    agents
        .iter()
        .filter(|a| a.tmux_session_name == session_name && a.status == AgentStatus::Review)
        .map(|a| a.pane_id.clone())
        .collect()
}

/// The single-character dot used to render a status in the agents view.
///
/// `Unknown` renders as the idle dot: an agent with no status file yet (freshly
/// spawned, or never prompted since the hooks were installed) has nothing in
/// flight, so idle is the honest presentation. The variants stay distinct in the
/// model because `status_since` differs — `Unknown` carries 0, not a real mtime.
pub fn status_glyph(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "●",
        AgentStatus::Waiting => "◐",
        AgentStatus::Review => "◆",
        AgentStatus::Idle | AgentStatus::Unknown => "○",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::AgentType;

    fn mk_agent(pane_id: &str) -> AgentSession {
        AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: "tws_x".into(),
            window_index: 0,
            pane_id: pane_id.into(),
            display_name: "claude".into(),
            renamed: false,
            pin_slot: None,
            status: AgentStatus::Unknown,
            status_since: 0,
        }
    }

    #[test]
    fn parse_known_words() {
        assert_eq!(parse_status("working"), AgentStatus::Working);
        assert_eq!(parse_status("waiting"), AgentStatus::Waiting);
        assert_eq!(parse_status("idle"), AgentStatus::Idle);
        assert_eq!(parse_status("review"), AgentStatus::Review);
    }

    #[test]
    fn parse_unknown_word_is_unknown() {
        assert_eq!(parse_status("garbage"), AgentStatus::Unknown);
        assert_eq!(parse_status(""), AgentStatus::Unknown);
    }

    #[test]
    fn load_missing_dir_is_empty() {
        let dir = std::env::temp_dir().join(format!("tws-test-missing-{}", std::process::id()));
        let map = load_statuses_from(&dir);
        assert!(map.is_empty());
    }

    #[test]
    fn load_reads_word_and_mtime() {
        let dir = std::env::temp_dir().join(format!("tws-test-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("%3"), "waiting\n").unwrap();

        let map = load_statuses_from(&dir);
        let (status, ts) = map.get("%3").copied().unwrap();
        assert_eq!(status, AgentStatus::Waiting);
        assert!(ts > 0);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_sets_matching_and_defaults_missing() {
        let mut agents = vec![mk_agent("%1"), mk_agent("%2")];
        let mut map = HashMap::new();
        map.insert("%1".to_string(), (AgentStatus::Waiting, 1234));

        apply_statuses(&mut agents, &map);

        assert_eq!(agents[0].status, AgentStatus::Waiting);
        assert_eq!(agents[0].status_since, 1234);
        assert_eq!(agents[1].status, AgentStatus::Unknown);
        assert_eq!(agents[1].status_since, 0);
    }

    #[test]
    fn counts_tally_by_state() {
        let mut agents = vec![mk_agent("%1"), mk_agent("%2"), mk_agent("%3")];
        agents[0].status = AgentStatus::Working;
        agents[1].status = AgentStatus::Working;
        agents[2].status = AgentStatus::Waiting;

        let c = status_counts(&agents);
        assert_eq!(c.working, 2);
        assert_eq!(c.waiting, 1);
        assert_eq!(c.idle, 0);
    }

    #[test]
    fn counts_tally_review() {
        let mut agents = vec![mk_agent("%1"), mk_agent("%2")];
        agents[0].status = AgentStatus::Review;
        agents[1].status = AgentStatus::Idle;
        let c = status_counts(&agents);
        assert_eq!(c.review, 1);
        assert_eq!(c.idle, 1);
    }

    #[test]
    fn glyphs_are_distinct_per_state() {
        assert_eq!(status_glyph(AgentStatus::Working), "●");
        assert_eq!(status_glyph(AgentStatus::Waiting), "◐");
        assert_eq!(status_glyph(AgentStatus::Review), "◆");
        assert_eq!(status_glyph(AgentStatus::Idle), "○");
        // Review is visually distinct from every other dot.
        assert_ne!(status_glyph(AgentStatus::Review), status_glyph(AgentStatus::Waiting));
        assert_ne!(status_glyph(AgentStatus::Review), status_glyph(AgentStatus::Idle));
    }

    #[test]
    fn unknown_renders_as_idle_dot() {
        assert_eq!(
            status_glyph(AgentStatus::Unknown),
            status_glyph(AgentStatus::Idle)
        );
    }

    #[test]
    fn unknown_is_still_distinct_in_the_model() {
        // Sharing a glyph must not leak into the data: a status-less agent keeps
        // status_since = 0 rather than claiming a real timestamp.
        let mut agents = vec![mk_agent("%1")];
        apply_statuses(&mut agents, &HashMap::new());
        assert_eq!(agents[0].status, AgentStatus::Unknown);
        assert_eq!(agents[0].status_since, 0);
        assert_eq!(status_counts(&agents).idle, 0);
    }

    #[test]
    fn status_word_round_trips_through_parse() {
        for s in [
            AgentStatus::Working,
            AgentStatus::Waiting,
            AgentStatus::Review,
            AgentStatus::Idle,
        ] {
            assert_eq!(parse_status(status_word(s)), s);
        }
    }

    #[test]
    fn write_then_load_reads_back_the_status() {
        let dir = std::env::temp_dir().join(format!("tws-test-write-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        write_status_to(&dir, "%7", AgentStatus::Review).unwrap();

        let map = load_statuses_from(&dir);
        assert_eq!(map.get("%7").unwrap().0, AgentStatus::Review);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ack_selects_only_review_agents_in_the_named_session() {
        let mut agents = vec![mk_agent("%1"), mk_agent("%2"), mk_agent("%3")];
        agents[0].tmux_session_name = "A".into();
        agents[0].status = AgentStatus::Review;
        agents[1].tmux_session_name = "A".into();
        agents[1].status = AgentStatus::Working; // same session, not review
        agents[2].tmux_session_name = "B".into();
        agents[2].status = AgentStatus::Review; // review, wrong session

        let acked = agents_to_ack(&agents, "A");
        assert_eq!(acked, vec!["%1".to_string()]);
    }

    #[test]
    fn prune_removes_files_without_live_pane() {
        let dir = std::env::temp_dir().join(format!("tws-test-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("%1"), "idle").unwrap();
        std::fs::write(dir.join("%2"), "idle").unwrap();

        let mut live = HashSet::new();
        live.insert("%1".to_string());
        prune_stale_files(&dir, &live);

        assert!(dir.join("%1").exists());
        assert!(!dir.join("%2").exists());

        std::fs::remove_dir_all(&dir).ok();
    }
}
