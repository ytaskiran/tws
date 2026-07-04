use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::core::model::{AgentSession, AgentStatus};
use crate::core::persistence::config_dir;

/// Directory where per-pane status files live: `~/.config/tws/agents/`.
#[allow(dead_code)]
pub fn agents_dir() -> PathBuf {
    config_dir().join("agents")
}

/// Map a status word written by a hook to an `AgentStatus`.
#[allow(dead_code)]
pub fn parse_status(word: &str) -> AgentStatus {
    match word.trim() {
        "working" => AgentStatus::Working,
        "waiting" => AgentStatus::Waiting,
        "idle" => AgentStatus::Idle,
        _ => AgentStatus::Unknown,
    }
}

/// Read every status file in `dir` into `pane_id -> (status, mtime_epoch_secs)`.
/// Missing/unreadable dir yields an empty map. Unreadable individual files are skipped.
#[allow(dead_code)]
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
#[allow(dead_code)]
pub fn load_statuses() -> HashMap<String, (AgentStatus, i64)> {
    load_statuses_from(&agents_dir())
}

/// Join loaded statuses onto agents by `pane_id`. Agents with no matching file
/// are set to `Unknown` / `0` (so a removed file resets state on the next scan).
/// Used in a later task for joining status-file data onto live agent sessions.
#[allow(dead_code)]
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
#[allow(dead_code)]
pub struct StatusCounts {
    pub working: usize,
    pub waiting: usize,
    pub idle: usize,
}

/// Count agent sessions by their status.
/// Used in a later task for rendering status tallies.
#[allow(dead_code)]
pub fn status_counts(agents: &[AgentSession]) -> StatusCounts {
    let mut c = StatusCounts {
        working: 0,
        waiting: 0,
        idle: 0,
    };
    for a in agents {
        match a.status {
            AgentStatus::Working => c.working += 1,
            AgentStatus::Waiting => c.waiting += 1,
            AgentStatus::Idle => c.idle += 1,
            AgentStatus::Unknown => {}
        }
    }
    c
}

/// Delete status files whose `pane_id` is not in the live set (pane died).
/// Missing or unreadable dir is a no-op (not an error).
/// Used in a later task for garbage collecting stale status files.
#[allow(dead_code)]
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
