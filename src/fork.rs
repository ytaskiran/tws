use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::model::AgentType;
use crate::core::persistence::config_dir;
use crate::tmux::agent_scan::agent_in_pane;

#[derive(Debug, PartialEq, Eq)]
pub struct ForkTarget {
    pub session_id: String,
    pub cwd: PathBuf,
}

pub fn parse_pointer(contents: &str) -> Option<ForkTarget> {
    let line = contents.lines().next()?;
    let (session_id, cwd) = line.split_once('\t')?;
    if session_id.is_empty() || cwd.is_empty() {
        return None;
    }
    Some(ForkTarget {
        session_id: session_id.to_string(),
        cwd: PathBuf::from(cwd),
    })
}

pub fn pointer_path(pane_id: &str) -> PathBuf {
    config_dir().join("sessions").join(pane_id)
}

/// Claude Code names its per-project transcript directory after the session cwd
/// with every non-alphanumeric character replaced by `-`. Undocumented internal;
/// `classify` treats a missing projects dir as "rule changed" rather than an error.
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn transcript_path(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(project_slug(cwd))
        .join(format!("{session_id}.jsonl"))
}

#[derive(Debug, PartialEq, Eq)]
pub enum ForkError {
    NoAgent,
    UnsupportedAgent(AgentType),
    NoPointer,
    StalePointer,
    WrongDirectory,
}

pub struct Facts<'a> {
    pub pointer: Option<&'a str>,
    pub agent: Option<AgentType>,
    pub pane_cwd: Option<&'a Path>,
    pub project_dir_exists: bool,
    pub transcript_exists: bool,
}

pub fn classify(f: &Facts) -> Result<ForkTarget, ForkError> {
    let Some(raw) = f.pointer else {
        return Err(match f.agent {
            Some(AgentType::ClaudeCode) => ForkError::NoPointer,
            Some(other) => ForkError::UnsupportedAgent(other),
            None => ForkError::NoAgent,
        });
    };

    let target = parse_pointer(raw).ok_or(ForkError::StalePointer)?;

    if let Some(pane_cwd) = f.pane_cwd
        && pane_cwd != target.cwd.as_path()
    {
        return Err(ForkError::WrongDirectory);
    }

    // Fail open: a missing projects dir means the upstream slug rule changed,
    // not that the session is gone. Let claude report its own error instead.
    if f.project_dir_exists && !f.transcript_exists {
        return Err(ForkError::StalePointer);
    }

    Ok(target)
}

pub fn message(e: &ForkError) -> String {
    match e {
        ForkError::NoAgent => "fork: no agent running in this pane".into(),
        ForkError::UnsupportedAgent(AgentType::Codex) => {
            "fork: only Claude Code supports forking — codex resume appends to the parent session"
                .into()
        }
        ForkError::UnsupportedAgent(other) => {
            format!("fork: only Claude Code supports forking (this pane runs {})", other.display_name())
        }
        ForkError::NoPointer => {
            "fork: no session recorded for this pane — it started before the tws hooks were installed. Restart Claude in this pane."
                .into()
        }
        ForkError::StalePointer => {
            "fork: the recorded session no longer exists".into()
        }
        ForkError::WrongDirectory => {
            "fork: the recorded session belongs to a different directory (tmux reused this pane id)"
                .into()
        }
    }
}

pub fn build_argv(t: &ForkTarget) -> Vec<String> {
    vec![
        "--resume".to_string(),
        t.session_id.clone(),
        "--fork-session".to_string(),
    ]
}

fn pane_cwd(pane_id: &str) -> Option<PathBuf> {
    let out = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "-t",
            pane_id,
            "#{pane_current_path}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

fn fail(e: &ForkError) -> ! {
    eprintln!("{}", message(e));
    eprint!("\npress any key to close ");
    let _ = std::io::stderr().flush();
    let mut buf = [0u8; 1];
    let _ = std::io::stdin().read(&mut buf);
    std::process::exit(1);
}

pub fn run(pane_id: &str) -> ! {
    let raw = std::fs::read_to_string(pointer_path(pane_id)).ok();
    let home = dirs::home_dir().unwrap_or_default();
    let live_cwd = pane_cwd(pane_id);

    let parsed = raw.as_deref().and_then(parse_pointer);
    let (project_dir_exists, transcript_exists) = match &parsed {
        Some(t) => {
            let transcript = transcript_path(&home, &t.cwd, &t.session_id);
            let dir_exists = transcript.parent().is_some_and(|p| p.exists());
            (dir_exists, transcript.exists())
        }
        None => (false, false),
    };

    let facts = Facts {
        pointer: raw.as_deref(),
        agent: agent_in_pane(pane_id),
        pane_cwd: live_cwd.as_deref(),
        project_dir_exists,
        transcript_exists,
    };

    let target = match classify(&facts) {
        Ok(t) => t,
        Err(e) => fail(&e),
    };

    // TWS_FORK stops the fork's own SessionStart hook from overwriting the
    // parent's pointer — without it, the next fork would fork this fork.
    let err = Command::new("claude")
        .args(build_argv(&target))
        .current_dir(&target.cwd)
        .env("TWS_FORK", "1")
        .exec();

    eprintln!("fork: could not launch claude: {err}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pointer_well_formed() {
        let t = parse_pointer("abc-123\t/Users/me/proj\n").unwrap();
        assert_eq!(t.session_id, "abc-123");
        assert_eq!(t.cwd, PathBuf::from("/Users/me/proj"));
    }

    #[test]
    fn parse_pointer_without_trailing_newline() {
        let t = parse_pointer("abc-123\t/Users/me/proj").unwrap();
        assert_eq!(t.cwd, PathBuf::from("/Users/me/proj"));
    }

    #[test]
    fn parse_pointer_rejects_malformed() {
        assert!(parse_pointer("").is_none());
        assert!(parse_pointer("no-tab-here\n").is_none());
        assert!(parse_pointer("\t/Users/me\n").is_none());
        assert!(parse_pointer("abc-123\t\n").is_none());
    }

    #[test]
    fn project_slug_replaces_non_alphanumerics() {
        assert_eq!(
            project_slug(Path::new("/Users/ytaskiran")),
            "-Users-ytaskiran"
        );
        assert_eq!(
            project_slug(Path::new("/Users/me/my.proj-1")),
            "-Users-me-my-proj-1"
        );
    }

    #[test]
    fn transcript_path_composes_projects_dir() {
        let p = transcript_path(Path::new("/home/me"), Path::new("/home/me/proj"), "sid-9");
        assert_eq!(
            p,
            PathBuf::from("/home/me/.claude/projects/-home-me-proj/sid-9.jsonl")
        );
    }

    fn facts<'a>(pointer: Option<&'a str>, agent: Option<AgentType>) -> Facts<'a> {
        Facts {
            pointer,
            agent,
            pane_cwd: None,
            project_dir_exists: true,
            transcript_exists: true,
        }
    }

    #[test]
    fn classify_happy_path() {
        let f = facts(Some("sid-1\t/proj\n"), Some(AgentType::ClaudeCode));
        let t = classify(&f).unwrap();
        assert_eq!(t.session_id, "sid-1");
    }

    #[test]
    fn classify_no_pointer_claude_pane_means_hook_missing() {
        let f = facts(None, Some(AgentType::ClaudeCode));
        assert_eq!(classify(&f), Err(ForkError::NoPointer));
    }

    #[test]
    fn classify_no_pointer_codex_pane_is_unsupported() {
        let f = facts(None, Some(AgentType::Codex));
        assert_eq!(
            classify(&f),
            Err(ForkError::UnsupportedAgent(AgentType::Codex))
        );
        let f = facts(None, Some(AgentType::Pi));
        assert_eq!(
            classify(&f),
            Err(ForkError::UnsupportedAgent(AgentType::Pi))
        );
    }

    #[test]
    fn classify_no_pointer_no_agent() {
        let f = facts(None, None);
        assert_eq!(classify(&f), Err(ForkError::NoAgent));
    }

    #[test]
    fn classify_malformed_pointer_is_stale() {
        let f = facts(Some("garbage"), Some(AgentType::ClaudeCode));
        assert_eq!(classify(&f), Err(ForkError::StalePointer));
    }

    #[test]
    fn classify_missing_transcript_is_stale() {
        let mut f = facts(Some("sid-1\t/proj\n"), Some(AgentType::ClaudeCode));
        f.transcript_exists = false;
        assert_eq!(classify(&f), Err(ForkError::StalePointer));
    }

    #[test]
    fn classify_missing_project_dir_fails_open() {
        let mut f = facts(Some("sid-1\t/proj\n"), Some(AgentType::ClaudeCode));
        f.project_dir_exists = false;
        f.transcript_exists = false;
        assert!(classify(&f).is_ok());
    }

    #[test]
    fn classify_recycled_pane_id_detected_by_cwd_mismatch() {
        let mut f = facts(Some("sid-1\t/proj\n"), Some(AgentType::ClaudeCode));
        f.pane_cwd = Some(Path::new("/other-proj"));
        assert_eq!(classify(&f), Err(ForkError::WrongDirectory));
    }

    #[test]
    fn classify_checks_cwd_before_transcript() {
        let mut f = facts(Some("sid-1\t/proj\n"), Some(AgentType::ClaudeCode));
        f.pane_cwd = Some(Path::new("/other-proj"));
        f.transcript_exists = false;
        // project_dir_exists remains true from facts() default.
        // If cwd check runs first, result is WrongDirectory.
        // If transcript check runs first, result is StalePointer.
        assert_eq!(classify(&f), Err(ForkError::WrongDirectory));
    }

    #[test]
    fn classify_matching_cwd_is_ok() {
        let mut f = facts(Some("sid-1\t/proj\n"), Some(AgentType::ClaudeCode));
        f.pane_cwd = Some(Path::new("/proj"));
        assert!(classify(&f).is_ok());
    }

    #[test]
    fn build_argv_exact_flag_order() {
        let t = ForkTarget {
            session_id: "sid-7".into(),
            cwd: PathBuf::from("/proj"),
        };
        assert_eq!(
            build_argv(&t),
            vec![
                "--resume".to_string(),
                "sid-7".to_string(),
                "--fork-session".to_string(),
            ]
        );
    }

    #[test]
    fn message_is_non_empty_for_every_variant() {
        for e in [
            ForkError::NoAgent,
            ForkError::UnsupportedAgent(AgentType::Codex),
            ForkError::NoPointer,
            ForkError::StalePointer,
            ForkError::WrongDirectory,
        ] {
            assert!(!message(&e).is_empty());
        }
    }
}
