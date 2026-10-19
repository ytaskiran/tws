//! Pure core of agent resume: pointer files, the snapshot file, and the restore plan.

use std::path::PathBuf;

use crate::core::model::AgentType;
use crate::core::persistence::config_dir;
use crate::tmux::agent_scan::LivePane;

/// A server younger than this counts as freshly booted.
#[allow(dead_code)]
pub const NEW_SERVER_SECS: i64 = 300;

#[allow(dead_code)]
const PLAIN_SHELLS: [&str; 8] = ["sh", "bash", "zsh", "fish", "dash", "ksh", "tcsh", "nu"];

/// What a hook writes for a pane: `<ref>\t<cwd>\t<pid>\n`.
#[allow(dead_code)]
pub struct Pointer {
    pub session_ref: String,
    pub cwd: PathBuf,
    pub pid: u32,
}

#[allow(dead_code)]
pub fn parse_pointer(contents: &str) -> Option<Pointer> {
    let mut f = contents.trim_end_matches('\n').splitn(3, '\t');
    let (r, cwd, pid) = (f.next()?, f.next()?, f.next()?);
    if r.is_empty() || cwd.is_empty() {
        return None;
    }
    Some(Pointer {
        session_ref: r.to_string(),
        cwd: cwd.into(),
        pid: pid.parse().ok()?,
    })
}

#[allow(dead_code)]
pub fn pointer_path(pane_id: &str, agent: AgentType) -> PathBuf {
    config_dir()
        .join("sessions")
        .join(format!("{pane_id}.{}", agent.pointer_name()))
}

#[allow(dead_code)]
pub fn snapshot_path() -> PathBuf {
    config_dir().join("agents.snapshot")
}

#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub struct SnapshotLine {
    pub key: String,
    pub agent: AgentType,
    pub session_ref: String,
    pub cwd: PathBuf,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

/// Keeps a pane only if its pointer's pid is in the scanned agent pids.
#[allow(dead_code)]
pub fn build_snapshot(
    panes: &[LivePane],
    read_pointer: impl Fn(&str, AgentType) -> Option<String>,
) -> Vec<SnapshotLine> {
    panes
        .iter()
        .filter_map(|p| {
            let (agent, pids) = p.agent.as_ref()?;
            let ptr = parse_pointer(&read_pointer(&p.pane_id, *agent)?)?;
            pids.contains(&ptr.pid).then(|| SnapshotLine {
                key: p.key.clone(),
                agent: *agent,
                session_ref: ptr.session_ref,
                cwd: ptr.cwd,
            })
        })
        .collect()
}

/// POSIX single quotes.
#[allow(dead_code)]
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[allow(dead_code)]
fn is_plain_shell(current_command: &str, user_shell: Option<&str>) -> bool {
    let c = current_command.trim_start_matches('-');
    PLAIN_SHELLS.contains(&c)
        || user_shell
            .and_then(|s| s.rsplit('/').next())
            .is_some_and(|b| !b.is_empty() && b == c)
}

/// (pane_id, shell line) for each line to resume. Empty when server_age_secs > NEW_SERVER_SECS.
#[allow(dead_code)]
pub fn plan_restore(
    snapshot: &[SnapshotLine],
    panes: &[LivePane],
    server_age_secs: i64,
    user_shell: Option<&str>,
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
            let argv: Vec<String> = l
                .agent
                .resume_argv(&l.session_ref)
                .iter()
                .map(|a| shell_quote(a))
                .collect();
            let line = format!(
                "cd -- {} && {}",
                shell_quote(&l.cwd.to_string_lossy()),
                argv.join(" ")
            );
            Some((p.pane_id.clone(), line))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for bad in ["abc\t/x\n", "\t/x\t1", "abc\t\t1", "abc\t/x\tnope", ""] {
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
        let snap = build_snapshot(&panes, |pane, _| match pane {
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
            plan_restore(&snap, &panes, 10, Some("/bin/zsh")),
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
                None
            )
            .is_empty()
        );
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
