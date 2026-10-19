//! Pure core of agent resume: pointer files, the snapshot file, and the restore plan.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::model::AgentType;
use crate::core::persistence::config_dir;
use crate::core::status::{prune_stale_files, write_atomic};
use crate::tmux::agent_scan::{LivePane, scan_live_panes};

/// A server younger than this counts as freshly booted.
pub const NEW_SERVER_SECS: i64 = 300;

/// Only POSIX-like shells: the typed line uses `&&` and `'\''`, which nushell rejects.
const PLAIN_SHELLS: [&str; 7] = ["sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh"];

/// What a hook writes for a pane: `<ref>\t<cwd>\t<pid>\n`.
pub struct Pointer {
    pub session_ref: String,
    pub cwd: PathBuf,
    pub pid: u32,
}

pub fn parse_pointer(contents: &str) -> Option<Pointer> {
    let mut f = contents.trim_end_matches('\n').splitn(3, '\t');
    let (r, cwd, pid) = (f.next()?, f.next()?, f.next()?);
    if r.is_empty() || cwd.is_empty() || [r, cwd].iter().any(|f| f.contains(['\n', '\r'])) {
        return None;
    }
    Some(Pointer {
        session_ref: r.to_string(),
        cwd: cwd.into(),
        pid: pid.parse().ok()?,
    })
}

pub fn pointer_path(pane_id: &str, agent: AgentType) -> PathBuf {
    config_dir()
        .join("sessions")
        .join(format!("{pane_id}.{}", agent.pointer_name()))
}

pub fn snapshot_path() -> PathBuf {
    config_dir().join("agents.snapshot")
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapshotLine {
    pub key: String,
    pub agent: AgentType,
    pub session_ref: String,
    pub cwd: PathBuf,
}

pub fn format_snapshot(lines: &[SnapshotLine]) -> String {
    lines
        .iter()
        .map(|l| {
            format!(
                "{}\t{}\t{}\t{}\n",
                l.key,
                l.agent.pointer_name(),
                l.session_ref,
                l.cwd.display()
            )
        })
        .collect()
}

pub fn parse_snapshot(contents: &str) -> Vec<SnapshotLine> {
    contents
        .lines()
        .filter_map(|line| {
            let mut f = line.splitn(4, '\t');
            let (key, agent, r, cwd) = (f.next()?, f.next()?, f.next()?, f.next()?);
            if key.is_empty() || r.is_empty() || cwd.is_empty() {
                return None;
            }
            Some(SnapshotLine {
                key: key.to_string(),
                agent: AgentType::from_pointer_name(agent)?,
                session_ref: r.to_string(),
                cwd: cwd.into(),
            })
        })
        .collect()
}

/// Keeps a pane only if its pointer's pid is in the scanned agent pids. A live
/// agent with no valid pointer keeps its `previous` line of the same key and agent.
pub fn build_snapshot(
    panes: &[LivePane],
    previous: &[SnapshotLine],
    read_pointer: impl Fn(&str, AgentType) -> Option<String>,
) -> Vec<SnapshotLine> {
    panes
        .iter()
        .filter_map(|p| {
            let (agent, pids) = p.agent.as_ref()?;
            let fresh = read_pointer(&p.pane_id, *agent)
                .and_then(|c| parse_pointer(&c))
                .filter(|ptr| pids.contains(&ptr.pid))
                .map(|ptr| SnapshotLine {
                    key: p.key.clone(),
                    agent: *agent,
                    session_ref: ptr.session_ref,
                    cwd: ptr.cwd,
                });
            // A resumed opencode writes its pointer only at its first message.
            fresh.or_else(|| {
                previous
                    .iter()
                    .find(|l| l.key == p.key && l.agent == *agent)
                    .cloned()
            })
        })
        .collect()
}

/// POSIX single quotes.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn is_plain_shell(current_command: &str, user_shell: Option<&str>) -> bool {
    let c = current_command.trim_start_matches('-');
    PLAIN_SHELLS.contains(&c)
        || user_shell
            .and_then(|s| s.rsplit('/').next())
            .is_some_and(|b| !b.is_empty() && b == c)
}

/// True when `@resurrect-processes` makes tmux-resurrect start `bin` itself. Each
/// word may be `"~name->cmd`; only `name` counts, by its basename.
fn resurrect_restarts(resurrect_processes: &str, bin: &str) -> bool {
    resurrect_processes.split_whitespace().any(|w| {
        let w = w.trim_matches(['"', '\'']);
        let w = w.strip_prefix('~').unwrap_or(w);
        let w = w.split("->").next().unwrap_or(w);
        w == ":all:" || w.rsplit('/').next() == Some(bin)
    })
}

/// (pane_id, shell line) for each line to resume. Empty when server_age_secs > NEW_SERVER_SECS.
pub fn plan_restore(
    snapshot: &[SnapshotLine],
    panes: &[LivePane],
    server_age_secs: i64,
    user_shell: Option<&str>,
    resurrect_processes: &str,
) -> Vec<(String, String)> {
    if server_age_secs > NEW_SERVER_SECS {
        return Vec::new();
    }
    snapshot
        .iter()
        .filter_map(|l| {
            let p = panes.iter().find(|p| p.key == l.key)?;
            if p.agent.is_some() || !is_plain_shell(&p.current_command, user_shell) {
                return None;
            }
            let argv = l.agent.resume_argv(&l.session_ref);
            // resurrect typed the agent into the pane already; the shell is still in front.
            if resurrect_restarts(resurrect_processes, &argv[0]) {
                return None;
            }
            let argv: Vec<String> = argv.iter().map(|a| shell_quote(a)).collect();
            let line = format!(
                "cd -- {} && {}",
                shell_quote(&l.cwd.to_string_lossy()),
                argv.join(" ")
            );
            Some((p.pane_id.clone(), line))
        })
        .collect()
}

/// Never fails or prints: a tmux hook runs it. A failed scan keeps the old snapshot.
pub fn save_with(
    panes: Option<Vec<LivePane>>,
    sessions_dir: &Path,
    snapshot: &Path,
    scan_started_at: SystemTime,
) {
    let Some(panes) = panes else { return };
    let previous = parse_snapshot(&std::fs::read_to_string(snapshot).unwrap_or_default());
    let lines = build_snapshot(&panes, &previous, |pane, agent| {
        std::fs::read_to_string(sessions_dir.join(format!("{pane}.{}", agent.pointer_name()))).ok()
    });
    if let Some(dir) = snapshot.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if write_atomic(snapshot, &format_snapshot(&lines)).is_err() {
        return;
    }
    let live: HashSet<String> = panes.into_iter().map(|p| p.pane_id).collect();
    prune_stale_files(sessions_dir, &live, scan_started_at);
}

pub fn run_save() {
    let started = SystemTime::now();
    save_with(
        scan_live_panes(),
        &config_dir().join("sessions"),
        &snapshot_path(),
        started,
    );
}

fn tmux_ok(args: &[&str]) -> bool {
    Command::new("tmux")
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

pub fn run_restore() {
    let Some(start) = Command::new("tmux")
        .args(["display-message", "-p", "#{start_time}"])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<i64>()
                .ok()
        })
    else {
        return;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let Ok(contents) = std::fs::read_to_string(snapshot_path()) else {
        return;
    };
    let Some(panes) = scan_live_panes() else {
        return;
    };
    let shell = std::env::var("SHELL").ok();
    // resurrect types these programs into their panes just before this hook runs.
    let resurrect_processes = Command::new("tmux")
        .args(["show-options", "-gv", "@resurrect-processes"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    for (pane, line) in plan_restore(
        &parse_snapshot(&contents),
        &panes,
        now - start,
        shell.as_deref(),
        &resurrect_processes,
    ) {
        // -l types the line literally, so tmux reads no key name in it.
        if tmux_ok(&["send-keys", "-t", &pane, "-l", &line]) {
            tmux_ok(&["send-keys", "-t", &pane, "Enter"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    type Agent = Option<(AgentType, Vec<u32>)>;

    fn lp_cmd(pane_id: &str, key: &str, cmd: &str, agent: Agent) -> LivePane {
        LivePane {
            pane_id: pane_id.into(),
            key: key.into(),
            current_command: cmd.into(),
            agent,
        }
    }
    fn lp(pane_id: &str, key: &str, agent: Agent) -> LivePane {
        lp_cmd(pane_id, key, "zsh", agent)
    }

    fn tempdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tws-resume-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    #[test]
    fn save_keeps_old_snapshot_when_scan_fails() {
        let d = tempdir("save-fail");
        std::fs::write(d.join("agents.snapshot"), "s:1.0\tclaude\tid\t/w\n").unwrap();
        save_with(
            None,
            &d.join("sessions"),
            &d.join("agents.snapshot"),
            SystemTime::now(),
        );
        assert_eq!(
            std::fs::read_to_string(d.join("agents.snapshot")).unwrap(),
            "s:1.0\tclaude\tid\t/w\n"
        );
    }
    #[test]
    fn save_writes_empty_snapshot_when_no_agents_run() {
        let d = tempdir("save-empty");
        std::fs::write(d.join("agents.snapshot"), "old\n").unwrap();
        save_with(
            Some(vec![]),
            &d.join("sessions"),
            &d.join("agents.snapshot"),
            SystemTime::now(),
        );
        assert_eq!(
            std::fs::read_to_string(d.join("agents.snapshot")).unwrap(),
            ""
        );
    }
    #[test]
    fn save_prunes_pointers_of_dead_panes_after_writing() {
        let d = tempdir("save-prune");
        let s = d.join("sessions");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(s.join("%1.claude"), "id1\t/w\t200\n").unwrap();
        std::fs::write(s.join("%9.codex"), "gone\t/w\t1\n").unwrap();
        let started = SystemTime::now() + Duration::from_secs(1);
        save_with(
            Some(vec![lp(
                "%1",
                "s:1.0",
                Some((AgentType::ClaudeCode, vec![200])),
            )]),
            &s,
            &d.join("agents.snapshot"),
            started,
        );
        assert!(s.join("%1.claude").exists());
        assert!(!s.join("%9.codex").exists());
        assert_eq!(
            std::fs::read_to_string(d.join("agents.snapshot")).unwrap(),
            "s:1.0\tclaude\tid1\t/w\n"
        );
    }
    fn line(key: &str, agent: AgentType, r: &str) -> SnapshotLine {
        SnapshotLine {
            key: key.into(),
            agent,
            session_ref: r.into(),
            cwd: "/w".into(),
        }
    }
    #[test]
    fn build_snapshot_carries_forward_a_line_without_a_pointer() {
        let panes = vec![lp("%1", "s:1.0", Some((AgentType::OpenCode, vec![500])))];
        let prev = vec![line("s:1.0", AgentType::OpenCode, "ses_a")];
        assert_eq!(build_snapshot(&panes, &prev, |_, _| None), prev);
    }
    #[test]
    fn build_snapshot_does_not_carry_forward_a_different_agent() {
        let panes = vec![
            lp("%1", "s:1.0", Some((AgentType::OpenCode, vec![500]))),
            lp("%2", "s:1.1", None),
        ];
        let prev = vec![
            line("s:1.0", AgentType::ClaudeCode, "id"),
            line("s:1.1", AgentType::OpenCode, "ses_b"),
        ];
        assert_eq!(build_snapshot(&panes, &prev, |_, _| None), vec![]);
    }
    #[test]
    fn build_snapshot_prefers_a_valid_pointer_over_previous() {
        let panes = vec![lp("%1", "s:1.0", Some((AgentType::OpenCode, vec![500])))];
        let prev = vec![line("s:1.0", AgentType::OpenCode, "ses_old")];
        assert_eq!(
            build_snapshot(&panes, &prev, |_, _| Some("ses_new\t/w\t500\n".into())),
            vec![line("s:1.0", AgentType::OpenCode, "ses_new")]
        );
    }
    #[test]
    fn save_keeps_a_saved_line_when_the_pointer_is_gone() {
        let d = tempdir("save-carry");
        let s = d.join("sessions");
        std::fs::create_dir_all(&s).unwrap();
        std::fs::write(s.join("%1.opencode"), "ses_a\t/w\t500\n").unwrap();
        let panes = || {
            Some(vec![lp(
                "%1",
                "s:1.0",
                Some((AgentType::OpenCode, vec![500])),
            )])
        };
        save_with(panes(), &s, &d.join("agents.snapshot"), SystemTime::now());
        std::fs::remove_file(s.join("%1.opencode")).unwrap();
        save_with(panes(), &s, &d.join("agents.snapshot"), SystemTime::now());
        assert_eq!(
            std::fs::read_to_string(d.join("agents.snapshot")).unwrap(),
            "s:1.0\topencode\tses_a\t/w\n"
        );
    }
    #[test]
    fn parse_pointer_reads_three_fields() {
        let p = parse_pointer("abc\t/Users/me/my proj\t51234\n").unwrap();
        assert_eq!(
            (p.session_ref.as_str(), p.cwd.to_str().unwrap(), p.pid),
            ("abc", "/Users/me/my proj", 51234)
        );
    }
    #[test]
    fn parse_pointer_rejects_old_and_bad() {
        for bad in [
            "abc\t/x\n",
            "\t/x\t1",
            "abc\t\t1",
            "abc\t/x\tnope",
            "",
            "abc\r\t/x\t1",
            "abc\t/x\r\t1",
            "ab\nc\t/x\t1",
            "abc\t/x\ny\t1",
        ] {
            assert!(parse_pointer(bad).is_none(), "{bad:?}");
        }
    }
    #[test]
    fn snapshot_round_trips_with_spaces() {
        let l = vec![SnapshotLine {
            key: "my sess:1.0".into(),
            agent: AgentType::Pi,
            session_ref: "/p/s.jsonl".into(),
            cwd: "/a b".into(),
        }];
        assert_eq!(parse_snapshot(&format_snapshot(&l)), l);
    }
    #[test]
    fn parse_snapshot_skips_bad_lines() {
        let s = "k:1.0\tclaude\tid\t/x\ngarbage\nk:1.1\tnobody\tid\t/x\nk:1.2\tcodex\tid2\t/y\n";
        assert_eq!(
            parse_snapshot(s)
                .iter()
                .map(|l| l.key.as_str())
                .collect::<Vec<_>>(),
            ["k:1.0", "k:1.2"]
        );
    }
    #[test]
    fn build_snapshot_keeps_matching_pid_only() {
        // %1: pointer pid 200 is the agent. %2: pointer pid 999 is dead (reused pane id). %3: no pointer. %4: no agent.
        let panes = vec![
            lp("%1", "s:1.0", Some((AgentType::ClaudeCode, vec![200, 201]))),
            lp("%2", "s:1.1", Some((AgentType::ClaudeCode, vec![300]))),
            lp("%3", "s:1.2", Some((AgentType::Codex, vec![400]))),
            lp("%4", "s:1.3", None),
        ];
        let snap = build_snapshot(&panes, &[], |pane, _| match pane {
            "%1" => Some("id1\t/w\t201\n".into()),
            "%2" => Some("old\t/w\t999\n".into()),
            _ => None,
        });
        assert_eq!(
            snap,
            vec![SnapshotLine {
                key: "s:1.0".into(),
                agent: AgentType::ClaudeCode,
                session_ref: "id1".into(),
                cwd: "/w".into()
            }]
        );
    }
    #[test]
    fn plan_restore_types_into_plain_shell_only() {
        let snap = parse_snapshot(
            "s:1.0\tclaude\tid1\t/it's here\ns:1.1\tcodex\tid2\t/w\ns:1.2\tpi\t/p\t/w\ns:9.9\tdevin\tid\t/w\n",
        );
        let panes = vec![
            lp_cmd("%5", "s:1.0", "-zsh", None),
            lp_cmd("%6", "s:1.1", "vim", None),
            lp_cmd("%7", "s:1.2", "zsh", Some((AgentType::Pi, vec![1]))),
        ];
        assert_eq!(
            plan_restore(&snap, &panes, 10, Some("/bin/zsh"), ""),
            vec![(
                "%5".into(),
                r#"cd -- '/it'\''s here' && 'claude' '--resume' 'id1'"#.into()
            )]
        );
    }
    #[test]
    fn plan_restore_does_nothing_on_an_old_server() {
        let snap = parse_snapshot("s:1.0\tclaude\tid1\t/w\n");
        assert!(
            plan_restore(
                &snap,
                &[lp_cmd("%5", "s:1.0", "zsh", None)],
                NEW_SERVER_SECS + 1,
                None,
                ""
            )
            .is_empty()
        );
    }
    #[test]
    fn nu_is_not_a_plain_shell() {
        // The typed line is POSIX; nushell rejects `&&` and `'\''`.
        assert!(!is_plain_shell("nu", None));
    }
    fn planned(resurrect_processes: &str) -> Vec<String> {
        let snap = parse_snapshot("s:1.0\tclaude\tid1\t/w\ns:1.1\tcodex\tid2\t/w\n");
        let panes = vec![
            lp_cmd("%5", "s:1.0", "zsh", None),
            lp_cmd("%6", "s:1.1", "zsh", None),
        ];
        plan_restore(&snap, &panes, 10, None, resurrect_processes)
            .into_iter()
            .map(|(pane, _)| pane)
            .collect()
    }
    #[test]
    fn plan_restore_skips_agents_that_resurrect_restarts() {
        assert_eq!(planned(""), ["%5", "%6"]);
        assert!(planned(":all:").is_empty());
        assert_eq!(planned("vim claude"), ["%6"]);
        assert_eq!(planned(r#"vim "~claude->claude --continue""#), ["%6"]);
        assert_eq!(planned("'/usr/local/bin/codex'"), ["%5"]);
        assert_eq!(planned("claudette"), ["%5", "%6"]);
    }
    #[test]
    fn resume_argv_per_agent() {
        assert_eq!(AgentType::Codex.resume_argv("x"), ["codex", "resume", "x"]);
        assert_eq!(
            AgentType::Devin.resume_argv("x"),
            ["devin", "--resume", "x"]
        );
        assert_eq!(AgentType::Pi.resume_argv("x"), ["pi", "--session", "x"]);
        assert_eq!(
            AgentType::OpenCode.resume_argv("x"),
            ["opencode", "--session", "x"]
        );
        assert_eq!(
            AgentType::ClaudeCode.resume_argv("x"),
            ["claude", "--resume", "x"]
        );
    }
}
