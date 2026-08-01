use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::core::model::{AgentSession, AgentStatus};
use crate::core::persistence::config_dir;

pub fn agents_dir() -> PathBuf {
    config_dir().join("agents")
}

pub fn parse_status(word: &str) -> AgentStatus {
    match word.trim() {
        "working" => AgentStatus::Working,
        "waiting" => AgentStatus::Waiting,
        "review" => AgentStatus::Review,
        "idle" => AgentStatus::Idle,
        _ => AgentStatus::Unknown,
    }
}

/// Missing directories and unreadable files are ignored.
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

pub fn load_statuses() -> HashMap<String, (AgentStatus, i64)> {
    load_statuses_from(&agents_dir())
}

/// Agents without a matching file reset to `Unknown` / `0`.
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

pub struct StatusCounts {
    pub working: usize,
    pub waiting: usize,
    pub review: usize,
    pub idle: usize,
}

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

/// Remove files for panes that are no longer live.
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

/// Convert a status to its on-disk representation; `Unknown` maps to `idle`.
pub fn status_word(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "working",
        AgentStatus::Waiting => "waiting",
        AgentStatus::Review => "review",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}

pub fn write_status_to(dir: &Path, pane_id: &str, status: AgentStatus) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(pane_id), status_word(status))
}

/// Write to the real agents directory; failures do not interrupt attach.
pub fn write_status(pane_id: &str, status: AgentStatus) {
    let _ = write_status_to(&agents_dir(), pane_id, status);
}

/// Return the landing pane only when it contains an agent in `Review`.
/// Acknowledgment is pane-scoped because a session may contain several agents.
pub fn agent_to_ack(agents: &[AgentSession], landing_pane: &str) -> Option<String> {
    agents
        .iter()
        .find(|a| a.pane_id == landing_pane && a.status == AgentStatus::Review)
        .map(|a| a.pane_id.clone())
}

/// Map statuses to their display glyphs. `Waiting` and `Review` intentionally share one.
pub fn status_glyph(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Working => "●",
        AgentStatus::Waiting | AgentStatus::Review => "◐",
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
    fn glyphs_map_states_to_dots() {
        assert_eq!(status_glyph(AgentStatus::Working), "●");
        assert_eq!(status_glyph(AgentStatus::Idle), "○");
        // Waiting and Review deliberately share the "your turn" dot, even though
        // they remain distinct states that clear differently.
        assert_eq!(status_glyph(AgentStatus::Waiting), "◐");
        assert_eq!(status_glyph(AgentStatus::Review), "◐");
        // The shared dot is still distinct from working and idle.
        assert_ne!(
            status_glyph(AgentStatus::Review),
            status_glyph(AgentStatus::Working)
        );
        assert_ne!(
            status_glyph(AgentStatus::Review),
            status_glyph(AgentStatus::Idle)
        );
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
    fn ack_selects_the_landing_pane_when_it_is_in_review() {
        let mut agents = vec![mk_agent("%1"), mk_agent("%2")];
        agents[0].status = AgentStatus::Review;
        agents[1].status = AgentStatus::Working;

        assert_eq!(agent_to_ack(&agents, "%1"), Some("%1".to_string()));
        // Landing on a pane that isn't in review acknowledges nothing.
        assert_eq!(agent_to_ack(&agents, "%2"), None);
        // A pane with no agent in it is not an ack target either.
        assert_eq!(agent_to_ack(&agents, "%99"), None);
    }

    #[test]
    fn ack_does_not_reach_review_siblings_sharing_the_session() {
        // The regression: three agents split across panes of a single tmux window,
        // all delivered and awaiting review. Attaching to one of them is an
        // acknowledgment of that one only — the other two stay in review.
        let mut agents = vec![mk_agent("%52"), mk_agent("%53"), mk_agent("%61")];
        for a in agents.iter_mut() {
            a.tmux_session_name = "twsr_experiments_software-factory".into();
            a.window_index = 0;
            a.status = AgentStatus::Review;
        }

        assert_eq!(agent_to_ack(&agents, "%53"), Some("%53".to_string()));
        assert_eq!(agent_to_ack(&agents, "%52"), Some("%52".to_string()));
        assert_eq!(agent_to_ack(&agents, "%61"), Some("%61".to_string()));
    }

    #[test]
    fn acking_one_pane_leaves_sibling_status_files_in_review() {
        // End-to-end over the files that actually hold the state: only the landing
        // pane's file is rewritten, so a later scan still reports the siblings as
        // needing review.
        let dir = std::env::temp_dir().join(format!("tws-test-ack-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let panes = ["%52", "%53", "%61"];
        for p in panes {
            write_status_to(&dir, p, AgentStatus::Review).unwrap();
        }

        let mut agents: Vec<AgentSession> = panes.iter().map(|p| mk_agent(p)).collect();
        apply_statuses(&mut agents, &load_statuses_from(&dir));

        if let Some(pane) = agent_to_ack(&agents, "%53") {
            write_status_to(&dir, &pane, AgentStatus::Idle).unwrap();
        }

        let map = load_statuses_from(&dir);
        assert_eq!(map.get("%53").unwrap().0, AgentStatus::Idle);
        assert_eq!(map.get("%52").unwrap().0, AgentStatus::Review);
        assert_eq!(map.get("%61").unwrap().0, AgentStatus::Review);

        std::fs::remove_dir_all(&dir).ok();
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
