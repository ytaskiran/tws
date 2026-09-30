use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::model::{AgentSession, AgentStatus};
use crate::core::persistence::config_dir;

pub fn agents_dir() -> PathBuf {
    config_dir().join("agents")
}

/// One directory per pane, one marker file per running subagent.
pub fn subagents_dir() -> PathBuf {
    config_dir().join("subagents")
}

/// One directory per pane, one key file per open permission request.
pub fn permissions_dir() -> PathBuf {
    config_dir().join("permissions")
}

/// One directory per pane, one marker file per tool call in flight.
pub fn inflight_dir() -> PathBuf {
    config_dir().join("inflight")
}

/// One file per pane. A hook touches it on each tool call while the pane works.
/// The status file keeps the state entry time, so the heartbeat needs its own mtime.
pub fn heartbeat_dir() -> PathBuf {
    config_dir().join("heartbeat")
}

pub fn trigger_path() -> PathBuf {
    config_dir().join("agent.trigger")
}

/// Tracks the trigger file that agent hooks touch after writing a status.
///
/// Callers must snapshot [`mtime`](Self::mtime) before reading any status file
/// and [`acknowledge`](Self::acknowledge) that snapshot afterwards, so a touch
/// landing mid-scan is not marked seen while its status went unread.
pub struct AgentTrigger {
    path: PathBuf,
    acknowledged: Option<SystemTime>,
}

impl AgentTrigger {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            acknowledged: None,
        }
    }

    pub fn mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok()
    }

    pub fn is_pending(&self) -> bool {
        match (self.mtime(), self.acknowledged) {
            (Some(mtime), Some(seen)) => mtime > seen,
            (Some(_), None) => true,
            (None, _) => false,
        }
    }

    /// A `None` snapshot acknowledges nothing, so a trigger created during the
    /// scan still fires.
    pub fn acknowledge(&mut self, mtime: Option<SystemTime>) {
        if let Some(m) = mtime {
            self.acknowledged = Some(m);
        }
    }
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
        if is_temp_name(&pane_id) {
            continue;
        }
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

/// A hook writes a dot temp file and renames it in the same instant, so a dot
/// file older than this belongs to a hook that died between the two steps.
pub const TEMP_FILE_MAX_AGE_SECS: u64 = 60;

/// Remove files and directories for panes that are no longer live.
///
/// Entries written since `scan_started_at` are kept regardless: an agent that
/// spawned after the caller took its pane snapshot is absent from
/// `live_pane_ids` but running, and deleting its fresh status blanks it out.
pub fn prune_stale_files(dir: &Path, live_pane_ids: &HashSet<String>, scan_started_at: SystemTime) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_temp_name(name) {
            let abandoned = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age.as_secs() > TEMP_FILE_MAX_AGE_SECS);
            if abandoned {
                std::fs::remove_file(&path).ok();
            }
            continue;
        }
        if live_pane_ids.contains(name) {
            continue;
        }
        let written_during_scan = entry
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|m| m >= scan_started_at);
        if !written_during_scan {
            if path.is_dir() {
                std::fs::remove_dir_all(&path).ok();
            } else {
                std::fs::remove_file(&path).ok();
            }
        }
    }
}

/// A tool call touches the pane's heartbeat file, so real activity beats every
/// few seconds; silence this long means the turn ended without a hook firing.
/// `SUBAGENT_FRESH_MINS` in `install.sh` must equal this window.
pub const STALE_WORKING_SECS: i64 = 15 * 60;

/// A tool call has no heartbeat while it runs, and a build or a test run can
/// last hours. A pane with an in-flight marker younger than this stays `working`.
pub const MAX_TOOL_SECS: i64 = 4 * 60 * 60;
const _: () = assert!(MAX_TOOL_SECS > STALE_WORKING_SECS);

/// Downgrade `working` files whose heartbeat stopped to `idle`.
///
/// A pane is alive at the newer of its status mtime and its `heartbeat_dir/<pane>`
/// mtime. The status mtime alone is not enough: a hook writes it only when the
/// word changes, so a long turn leaves it old.
///
/// A pane is spared while `inflight_dir/<pane>` holds a marker younger than
/// `MAX_TOOL_SECS`: a tool call is running, and it sends no heartbeat.
///
/// Backstop for turn-ends with no hook to fire — hard kills, API errors in Codex,
/// and an ESC interrupt in Claude Code (Codex has an `Interrupt` hook).
/// `idle` rather than `review` because those panes finished nothing, so alerting
/// on them is noise.
pub fn expire_stale_working(dir: &Path, inflight_dir: &Path, heartbeat_dir: &Path, now: i64) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if is_temp_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let path = entry.path();
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if parse_status(&contents) != AgentStatus::Working {
            continue;
        }
        let status_mtime = entry.metadata().ok().map(|m| mtime_secs(&m)).unwrap_or(0);
        let beat_mtime = std::fs::metadata(heartbeat_dir.join(entry.file_name()))
            .map(|m| mtime_secs(&m))
            .unwrap_or(0);
        let mtime = status_mtime.max(beat_mtime);
        if now - mtime > STALE_WORKING_SECS
            && !has_tool_in_flight(&inflight_dir.join(entry.file_name()), now)
        {
            write_atomic(&path, status_word(AgentStatus::Idle)).ok();
        }
    }
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// True when the pane's in-flight directory holds a marker younger than
/// `MAX_TOOL_SECS`. The cap bounds the damage from a marker that outlives its
/// call: an ESC in Claude Code fires no `PostToolUse`, and no `Stop` follows.
fn has_tool_in_flight(pane_dir: &Path, now: i64) -> bool {
    let Ok(entries) = std::fs::read_dir(pane_dir) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .any(|m| now - mtime_secs(&m) <= MAX_TOOL_SECS)
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
    write_atomic(&dir.join(pane_id), status_word(status))
}

/// Names that start with `.` are temp files from an atomic write, not panes.
fn is_temp_name(name: &str) -> bool {
    name.starts_with('.')
}

/// Write beside `path` and rename over it. A plain write truncates first, and a
/// reader that lands in that gap sees an empty file. The hook commands in
/// `install.sh` use the same temp name shape, so `is_temp_name` covers both.
fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.{}", std::process::id()));
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        std::fs::remove_file(&tmp).ok();
    })
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

/// Turn `review` into `idle` for one pane. Return `true` only when the file changed.
///
/// A tmux hook passes the pane id, so the id is untrusted: a name with a path
/// separator must never reach outside `dir`.
pub fn ack_pane_in(dir: &Path, pane_id: &str) -> bool {
    if pane_id.is_empty() || pane_id.contains(['/', '\\']) {
        return false;
    }
    let path = dir.join(pane_id);
    let is_review =
        std::fs::read_to_string(&path).is_ok_and(|c| parse_status(&c) == AgentStatus::Review);
    is_review && write_status_to(dir, pane_id, AgentStatus::Idle).is_ok()
}

/// Ack one pane in the real agents directory and wake tws.
///
/// The trigger moves only when a file changed, so a focus change on a pane
/// with no `review` status costs tws no rescan.
pub fn ack_pane(pane_id: &str) {
    if ack_pane_in(&agents_dir(), pane_id) {
        touch_trigger(&trigger_path());
    }
}

fn touch_trigger(path: &Path) {
    let _ = std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|f| f.set_modified(SystemTime::now()));
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

    fn ack_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tws-test-ackpane-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ack_pane_turns_review_into_idle() {
        let dir = ack_dir("review");
        std::fs::write(dir.join("%4"), "review").unwrap();

        assert!(ack_pane_in(&dir, "%4"));
        assert_eq!(std::fs::read_to_string(dir.join("%4")).unwrap(), "idle");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ack_pane_leaves_other_states_alone() {
        let dir = ack_dir("others");
        for word in ["working", "waiting", "idle"] {
            std::fs::write(dir.join("%4"), word).unwrap();

            assert!(!ack_pane_in(&dir, "%4"), "{word} must not change");
            assert_eq!(std::fs::read_to_string(dir.join("%4")).unwrap(), word);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ack_pane_ignores_a_missing_file() {
        let dir = ack_dir("missing");

        assert!(!ack_pane_in(&dir, "%4"));
        assert!(!dir.join("%4").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ack_pane_ignores_a_missing_directory() {
        let dir =
            std::env::temp_dir().join(format!("tws-test-ackpane-nodir-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();

        assert!(!ack_pane_in(&dir, "%4"));
        assert!(!dir.exists());
    }

    #[test]
    fn ack_pane_touches_only_the_named_pane() {
        let dir = ack_dir("sibling");
        std::fs::write(dir.join("%4"), "review").unwrap();
        std::fs::write(dir.join("%5"), "review").unwrap();

        assert!(ack_pane_in(&dir, "%4"));
        assert_eq!(std::fs::read_to_string(dir.join("%5")).unwrap(), "review");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ack_pane_rejects_ids_that_are_not_file_names() {
        let dir = ack_dir("badid");
        std::fs::write(dir.join("review-file"), "review").unwrap();

        assert!(!ack_pane_in(&dir, ""));
        assert!(!ack_pane_in(&dir, "../agents/review-file"));
        assert!(!ack_pane_in(&dir, "a/b"));
        assert_eq!(
            std::fs::read_to_string(dir.join("review-file")).unwrap(),
            "review"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    fn stale_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tws-test-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An in-flight directory that does not exist, for tests of the plain expiry.
    fn no_inflight(dir: &Path) -> PathBuf {
        dir.with_file_name(format!(
            "{}-no-inflight",
            dir.file_name().unwrap().to_string_lossy()
        ))
    }

    /// A heartbeat directory that does not exist, for tests that need no heartbeat.
    fn no_heartbeat(dir: &Path) -> PathBuf {
        dir.with_file_name(format!(
            "{}-no-heartbeat",
            dir.file_name().unwrap().to_string_lossy()
        ))
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    /// A `working` pane `%1` silent for longer than the stale window.
    fn stale_working_pane(tag: &str) -> (PathBuf, PathBuf) {
        let dir = stale_dir(tag);
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();
        backdate(&dir.join("%1"), STALE_WORKING_SECS as u64 + 60);
        let inflight = stale_dir(&format!("{tag}-inflight"));
        (dir, inflight)
    }

    fn status_of(dir: &Path, pane: &str) -> AgentStatus {
        load_statuses_from(dir).get(pane).unwrap().0
    }

    #[test]
    fn expire_keeps_a_stale_pane_with_a_fresh_in_flight_file() {
        let (dir, inflight) = stale_working_pane("flight-fresh");
        std::fs::create_dir_all(inflight.join("%1")).unwrap();
        std::fs::write(inflight.join("%1").join("m.toolu_1"), "").unwrap();

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Working);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
    }

    #[test]
    fn expire_keeps_a_pane_held_by_a_subagent_in_flight_file() {
        let (dir, inflight) = stale_working_pane("flight-sub");
        std::fs::create_dir_all(inflight.join("%1")).unwrap();
        std::fs::write(inflight.join("%1").join("s.toolu_2"), "").unwrap();

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Working);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
    }

    #[test]
    fn expire_ignores_an_in_flight_file_older_than_the_cap() {
        let (dir, inflight) = stale_working_pane("flight-old");
        std::fs::create_dir_all(inflight.join("%1")).unwrap();
        let marker = inflight.join("%1").join("m.toolu_1");
        std::fs::write(&marker, "").unwrap();
        backdate(&marker, MAX_TOOL_SECS as u64 + 60);

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
    }

    #[test]
    fn expire_keeps_a_pane_when_only_one_of_its_in_flight_files_is_old() {
        let (dir, inflight) = stale_working_pane("flight-mixed");
        std::fs::create_dir_all(inflight.join("%1")).unwrap();
        let old = inflight.join("%1").join("m.toolu_1");
        std::fs::write(&old, "").unwrap();
        backdate(&old, MAX_TOOL_SECS as u64 + 60);
        std::fs::write(inflight.join("%1").join("s.toolu_2"), "").unwrap();

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Working);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
    }

    #[test]
    fn expire_still_expires_a_pane_with_no_in_flight_directory() {
        let (dir, inflight) = stale_working_pane("flight-none");
        std::fs::remove_dir_all(&inflight).ok();

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expire_still_expires_a_pane_with_an_empty_in_flight_directory() {
        let (dir, inflight) = stale_working_pane("flight-empty");
        std::fs::create_dir_all(inflight.join("%1")).unwrap();

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
    }

    #[test]
    fn expire_ignores_the_in_flight_files_of_another_pane() {
        let (dir, inflight) = stale_working_pane("flight-other");
        std::fs::create_dir_all(inflight.join("%2")).unwrap();
        std::fs::write(inflight.join("%2").join("m.toolu_1"), "").unwrap();

        expire_stale_working(&dir, &inflight, &no_heartbeat(&dir), now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
    }

    #[test]
    fn prune_removes_in_flight_directories_without_live_pane() {
        let dir = stale_dir("prune-flight");
        std::fs::create_dir_all(dir.join("%1")).unwrap();
        std::fs::create_dir_all(dir.join("%2")).unwrap();
        std::fs::write(dir.join("%1").join("m.toolu_1"), "").unwrap();
        std::fs::write(dir.join("%2").join("s.toolu_2"), "").unwrap();

        let live: HashSet<String> = ["%1".to_string()].into();
        prune_stale_files(&dir, &live, SystemTime::now());

        assert!(dir.join("%1").join("m.toolu_1").exists());
        assert!(!dir.join("%2").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_an_in_flight_directory_written_after_the_scan_began() {
        let dir = stale_dir("prune-flight-race");
        std::fs::create_dir_all(dir.join("%old")).unwrap();

        let scan_started_at = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::create_dir_all(dir.join("%fresh")).unwrap();
        std::fs::write(dir.join("%fresh").join("m.toolu_1"), "").unwrap();

        prune_stale_files(&dir, &HashSet::new(), scan_started_at);

        assert!(dir.join("%fresh").join("m.toolu_1").exists());
        assert!(!dir.join("%old").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn inflight_dir_sits_beside_agents_dir() {
        assert_eq!(inflight_dir().parent(), agents_dir().parent());
        assert_ne!(inflight_dir(), agents_dir());
        assert_ne!(inflight_dir(), subagents_dir());
        assert_ne!(inflight_dir(), permissions_dir());
    }

    #[test]
    fn heartbeat_dir_sits_beside_agents_dir() {
        assert_eq!(heartbeat_dir().parent(), agents_dir().parent());
        assert_ne!(heartbeat_dir(), agents_dir());
        assert_ne!(heartbeat_dir(), inflight_dir());
        assert_ne!(heartbeat_dir(), subagents_dir());
        assert_ne!(heartbeat_dir(), permissions_dir());
    }

    #[test]
    fn expire_keeps_a_pane_with_an_old_status_and_a_fresh_heartbeat() {
        let (dir, inflight) = stale_working_pane("beat-fresh");
        let beat = stale_dir("beat-fresh-beat");
        std::fs::write(beat.join("%1"), "").unwrap();

        expire_stale_working(&dir, &inflight, &beat, now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Working);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
        std::fs::remove_dir_all(&beat).ok();
    }

    #[test]
    fn expire_downgrades_a_pane_when_status_and_heartbeat_are_both_old() {
        let (dir, inflight) = stale_working_pane("beat-old");
        let beat = stale_dir("beat-old-beat");
        std::fs::write(beat.join("%1"), "").unwrap();
        backdate(&beat.join("%1"), STALE_WORKING_SECS as u64 + 60);

        expire_stale_working(&dir, &inflight, &beat, now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
        std::fs::remove_dir_all(&beat).ok();
    }

    #[test]
    fn expire_uses_the_status_mtime_when_there_is_no_heartbeat_file() {
        let (dir, inflight) = stale_working_pane("beat-none");
        let beat = stale_dir("beat-none-beat");

        expire_stale_working(&dir, &inflight, &beat, now_secs());
        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);

        write_status_to(&dir, "%2", AgentStatus::Working).unwrap();
        expire_stale_working(&dir, &inflight, &beat, now_secs());
        assert_eq!(status_of(&dir, "%2"), AgentStatus::Working);

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
        std::fs::remove_dir_all(&beat).ok();
    }

    #[test]
    fn expire_ignores_the_heartbeat_of_another_pane() {
        let (dir, inflight) = stale_working_pane("beat-other");
        let beat = stale_dir("beat-other-beat");
        std::fs::write(beat.join("%2"), "").unwrap();

        expire_stale_working(&dir, &inflight, &beat, now_secs());

        assert_eq!(status_of(&dir, "%1"), AgentStatus::Idle);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
        std::fs::remove_dir_all(&beat).ok();
    }

    #[test]
    fn status_mtime_stays_the_state_entry_time_under_a_heartbeat() {
        let (dir, inflight) = stale_working_pane("beat-since");
        let beat = stale_dir("beat-since-beat");
        std::fs::write(beat.join("%1"), "").unwrap();
        let entered = load_statuses_from(&dir).get("%1").unwrap().1;

        expire_stale_working(&dir, &inflight, &beat, now_secs());

        assert_eq!(load_statuses_from(&dir).get("%1").unwrap().1, entered);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&inflight).ok();
        std::fs::remove_dir_all(&beat).ok();
    }

    #[test]
    fn prune_removes_heartbeat_files_without_live_pane_and_keeps_live_ones() {
        let dir = stale_dir("prune-beat");
        std::fs::write(dir.join("%1"), "").unwrap();
        std::fs::write(dir.join("%2"), "").unwrap();

        let live: HashSet<String> = ["%1".to_string()].into();
        prune_stale_files(&dir, &live, SystemTime::now());

        assert!(dir.join("%1").exists());
        assert!(!dir.join("%2").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_a_heartbeat_file_made_after_the_scan_began() {
        let dir = stale_dir("prune-beat-race");
        std::fs::write(dir.join("%old"), "").unwrap();

        let scan_started_at = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(dir.join("%fresh"), "").unwrap();

        prune_stale_files(&dir, &HashSet::new(), scan_started_at);

        assert!(dir.join("%fresh").exists());
        assert!(!dir.join("%old").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expires_working_past_the_threshold() {
        let dir = stale_dir("expire");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();

        let now = load_statuses_from(&dir).get("%1").unwrap().1 + STALE_WORKING_SECS + 1;
        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);

        assert_eq!(
            load_statuses_from(&dir).get("%1").unwrap().0,
            AgentStatus::Idle
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keeps_working_inside_the_threshold() {
        let dir = stale_dir("fresh");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();

        let now = load_statuses_from(&dir).get("%1").unwrap().1 + STALE_WORKING_SECS - 1;
        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);

        assert_eq!(
            load_statuses_from(&dir).get("%1").unwrap().0,
            AgentStatus::Working
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resting_states_never_expire() {
        // Downgrading a long-resting `review` would swallow a real alert.
        let dir = stale_dir("resting");
        write_status_to(&dir, "%1", AgentStatus::Review).unwrap();
        write_status_to(&dir, "%2", AgentStatus::Waiting).unwrap();
        write_status_to(&dir, "%3", AgentStatus::Idle).unwrap();

        let base = load_statuses_from(&dir).get("%1").unwrap().1;
        expire_stale_working(
            &dir,
            &no_inflight(&dir),
            &no_heartbeat(&dir),
            base + STALE_WORKING_SECS * 100,
        );

        let map = load_statuses_from(&dir);
        assert_eq!(map.get("%1").unwrap().0, AgentStatus::Review);
        assert_eq!(map.get("%2").unwrap().0, AgentStatus::Waiting);
        assert_eq!(map.get("%3").unwrap().0, AgentStatus::Idle);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expiry_is_idempotent() {
        // The flip bumps mtime, so a second pass must see a non-working word.
        let dir = stale_dir("idem");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();

        let now = load_statuses_from(&dir).get("%1").unwrap().1 + STALE_WORKING_SECS + 1;
        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);
        let after_first = load_statuses_from(&dir).get("%1").copied().unwrap();
        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);
        let after_second = load_statuses_from(&dir).get("%1").copied().unwrap();

        assert_eq!(after_first.0, AgentStatus::Idle);
        assert_eq!(after_second, after_first);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expire_on_missing_dir_is_a_noop() {
        let dir = std::env::temp_dir().join(format!("tws-test-noexist-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), 1_000_000);
    }

    #[test]
    fn expires_against_a_real_backdated_mtime() {
        // The other tests drive the clock through `now`; this one exercises the
        // mtime read path itself.
        use std::fs::{File, FileTimes};
        use std::time::{Duration, SystemTime};

        let dir = stale_dir("backdate");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();

        let old = SystemTime::now() - Duration::from_secs(STALE_WORKING_SECS as u64 + 60);
        let f = File::options().write(true).open(dir.join("%1")).unwrap();
        f.set_times(FileTimes::new().set_modified(old)).unwrap();
        drop(f);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);

        assert_eq!(
            load_statuses_from(&dir).get("%1").unwrap().0,
            AgentStatus::Idle
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Sleeps so each write lands in a distinct filesystem timestamp tick.
    fn touch(path: &Path) {
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(path, "").unwrap();
    }

    fn trigger_fixture(tag: &str) -> (PathBuf, AgentTrigger) {
        let dir = std::env::temp_dir().join(format!("tws-test-trig-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agent.trigger");
        let trigger = AgentTrigger::new(path.clone());
        (path, trigger)
    }

    #[test]
    fn trigger_missing_file_never_fires() {
        let (path, trigger) = trigger_fixture("missing");
        assert!(!trigger.is_pending());
        assert_eq!(trigger.mtime(), None);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn trigger_fires_until_acknowledged() {
        let (path, mut trigger) = trigger_fixture("ack");
        touch(&path);

        assert!(trigger.is_pending());
        // Still pending until acknowledged — merely observing must not consume.
        assert!(trigger.is_pending());

        trigger.acknowledge(trigger.mtime());
        assert!(!trigger.is_pending());

        touch(&path);
        assert!(trigger.is_pending());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn touch_landing_mid_scan_is_not_swallowed() {
        // The regression: a hook firing while a scan is in flight wrote its
        // status after that scan read them, so it must survive to the next poll.
        let (path, mut trigger) = trigger_fixture("race");
        touch(&path);

        let snapshot = trigger.mtime(); // scan begins
        touch(&path); // hook lands mid-scan
        trigger.acknowledge(snapshot); // scan ends

        assert!(
            trigger.is_pending(),
            "a touch that landed after the scan's snapshot must still fire"
        );

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn acknowledging_a_post_scan_read_would_swallow_the_touch() {
        // Inverse of the test above: pins down why the snapshot must be taken
        // up front, rather than read fresh at the end.
        let (path, mut trigger) = trigger_fixture("swallow");
        touch(&path);

        let _scan_read_statuses_here = trigger.mtime();
        touch(&path); // hook lands mid-scan
        trigger.acknowledge(trigger.mtime()); // read *after* the scan — the bug

        assert!(!trigger.is_pending());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn trigger_created_during_scan_still_fires() {
        let (path, mut trigger) = trigger_fixture("late-create");

        let snapshot = trigger.mtime(); // no trigger file yet
        touch(&path);
        trigger.acknowledge(snapshot);

        assert!(trigger.is_pending());

        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn prune_removes_files_without_live_pane() {
        let dir = std::env::temp_dir().join(format!("tws-test-prune-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("%1"), "idle").unwrap();
        std::fs::write(dir.join("%2"), "idle").unwrap();

        let mut live = HashSet::new();
        live.insert("%1".to_string());
        // Both files predate this scan, so membership alone decides.
        prune_stale_files(&dir, &live, SystemTime::now());

        assert!(dir.join("%1").exists());
        assert!(!dir.join("%2").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_a_status_written_after_the_scan_began() {
        // An agent that spawned mid-scan is missing from the live pane set but
        // is running, so deleting its fresh status would blank it out.
        let dir = std::env::temp_dir().join(format!("tws-test-prune-race-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("%old"), "idle").unwrap();

        let scan_started_at = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(dir.join("%fresh"), "working").unwrap();

        prune_stale_files(&dir, &HashSet::new(), scan_started_at);

        assert!(
            dir.join("%fresh").exists(),
            "a status written during the scan must survive the prune"
        );
        assert!(!dir.join("%old").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_removes_marker_directories_without_live_pane() {
        let dir = std::env::temp_dir().join(format!("tws-test-prune-dirs-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("%1")).unwrap();
        std::fs::create_dir_all(dir.join("%2")).unwrap();
        std::fs::write(dir.join("%1").join("a1"), "").unwrap();
        std::fs::write(dir.join("%2").join("a2"), "").unwrap();

        let mut live = HashSet::new();
        live.insert("%1".to_string());
        prune_stale_files(&dir, &live, SystemTime::now());

        assert!(dir.join("%1").join("a1").exists());
        assert!(!dir.join("%2").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_a_marker_directory_written_after_the_scan_began() {
        let dir =
            std::env::temp_dir().join(format!("tws-test-prune-dirs-race-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("%old")).unwrap();

        let scan_started_at = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::create_dir_all(dir.join("%fresh")).unwrap();

        prune_stale_files(&dir, &HashSet::new(), scan_started_at);

        assert!(
            dir.join("%fresh").exists(),
            "a marker directory made during the scan must survive the prune"
        );
        assert!(!dir.join("%old").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_removes_permission_directories_without_live_pane() {
        let dir = std::env::temp_dir().join(format!("tws-test-prune-perms-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("%1")).unwrap();
        std::fs::create_dir_all(dir.join("%2")).unwrap();
        std::fs::write(dir.join("%1").join("3735928559-42"), "").unwrap();
        std::fs::write(dir.join("%2").join("3735928559-42"), "").unwrap();

        let live: HashSet<String> = ["%1".to_string()].into();
        prune_stale_files(&dir, &live, SystemTime::now());

        assert!(dir.join("%1").join("3735928559-42").exists());
        assert!(!dir.join("%2").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_a_permission_directory_made_after_the_scan_began() {
        // A request raised mid-scan comes from a pane the snapshot missed.
        let dir =
            std::env::temp_dir().join(format!("tws-test-prune-perms-race-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("%old")).unwrap();

        let scan_started_at = SystemTime::now();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::create_dir_all(dir.join("%fresh")).unwrap();
        std::fs::write(dir.join("%fresh").join("3735928559-42"), "").unwrap();

        prune_stale_files(&dir, &HashSet::new(), scan_started_at);

        assert!(dir.join("%fresh").join("3735928559-42").exists());
        assert!(!dir.join("%old").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn permissions_dir_sits_beside_agents_dir() {
        assert_eq!(permissions_dir().parent(), agents_dir().parent());
        assert_ne!(permissions_dir(), agents_dir());
        assert_ne!(permissions_dir(), subagents_dir());
    }

    fn backdate(path: &Path, secs: u64) {
        use std::fs::{File, FileTimes};
        let old = SystemTime::now() - std::time::Duration::from_secs(secs);
        let f = File::options().write(true).open(path).unwrap();
        f.set_times(FileTimes::new().set_modified(old)).unwrap();
    }

    #[test]
    fn load_skips_dot_temp_files() {
        let dir = stale_dir("load-dot");
        std::fs::write(dir.join("%1"), "working").unwrap();
        std::fs::write(dir.join(".%1.4242"), "").unwrap();

        let map = load_statuses_from(&dir);

        assert_eq!(map.len(), 1);
        assert!(map.contains_key("%1"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expire_ignores_dot_temp_files() {
        let dir = stale_dir("expire-dot");
        std::fs::write(dir.join(".%1.4242"), "working").unwrap();
        backdate(&dir.join(".%1.4242"), STALE_WORKING_SECS as u64 + 60);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);

        assert_eq!(
            std::fs::read_to_string(dir.join(".%1.4242")).unwrap(),
            "working"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn expire_leaves_no_temp_file_behind() {
        let dir = stale_dir("expire-atomic");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();
        backdate(&dir.join("%1"), STALE_WORKING_SECS as u64 + 60);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        expire_stale_working(&dir, &no_inflight(&dir), &no_heartbeat(&dir), now);

        assert_eq!(std::fs::read_to_string(dir.join("%1")).unwrap(), "idle");
        assert_eq!(dir_names(&dir), vec!["%1".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    fn dir_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn write_status_replaces_the_file_and_leaves_no_temp_file() {
        let dir = stale_dir("write-atomic");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();
        write_status_to(&dir, "%1", AgentStatus::Review).unwrap();

        assert_eq!(std::fs::read_to_string(dir.join("%1")).unwrap(), "review");
        assert_eq!(dir_names(&dir), vec!["%1".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_status_replaces_a_file_through_a_rename() {
        // A rename gives the path a new inode. A truncating write keeps the
        // inode, and so keeps the empty-file gap that readers can see.
        use std::os::unix::fs::MetadataExt;
        let dir = stale_dir("write-inode");
        write_status_to(&dir, "%1", AgentStatus::Working).unwrap();
        let before = std::fs::metadata(dir.join("%1")).unwrap().ino();

        write_status_to(&dir, "%1", AgentStatus::Review).unwrap();

        let after = std::fs::metadata(dir.join("%1")).unwrap().ino();
        assert_ne!(before, after);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_removes_a_dot_file_older_than_a_minute() {
        let dir = stale_dir("prune-dot-old");
        std::fs::write(dir.join(".%1.4242"), "working").unwrap();
        backdate(&dir.join(".%1.4242"), TEMP_FILE_MAX_AGE_SECS + 5);

        prune_stale_files(&dir, &HashSet::new(), SystemTime::UNIX_EPOCH);

        assert!(!dir.join(".%1.4242").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_a_fresh_dot_file() {
        // A hook can be between its write and its rename right now.
        let dir = stale_dir("prune-dot-fresh");
        std::fs::write(dir.join(".%1.4242"), "working").unwrap();

        prune_stale_files(&dir, &HashSet::new(), SystemTime::now());

        assert!(dir.join(".%1.4242").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_never_treats_a_dot_file_as_a_pane() {
        // Even a live-pane entry that matches a dot name does not protect it.
        let dir = stale_dir("prune-dot-live");
        std::fs::write(dir.join(".%1.4242"), "working").unwrap();
        backdate(&dir.join(".%1.4242"), TEMP_FILE_MAX_AGE_SECS + 5);
        let live: HashSet<String> = [".%1.4242".to_string()].into();

        prune_stale_files(&dir, &live, SystemTime::now());

        assert!(!dir.join(".%1.4242").exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subagents_dir_sits_beside_agents_dir() {
        assert_eq!(subagents_dir().parent(), agents_dir().parent());
        assert_ne!(subagents_dir(), agents_dir());
    }

    #[test]
    fn install_marker_window_matches_the_stale_working_window() {
        let install = include_str!("../../install.sh");
        let minutes: i64 = install
            .lines()
            .find_map(|l| l.strip_prefix("SUBAGENT_FRESH_MINS="))
            .expect("install.sh defines SUBAGENT_FRESH_MINS")
            .trim()
            .parse()
            .expect("SUBAGENT_FRESH_MINS is a number");
        assert_eq!(minutes * 60, STALE_WORKING_SECS);
    }
}
