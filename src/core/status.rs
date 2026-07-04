use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::core::model::AgentStatus;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::AgentStatus;

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
}
