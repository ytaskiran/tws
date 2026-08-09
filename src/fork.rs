use std::path::{Path, PathBuf};

use crate::core::model::AgentType;
use crate::core::persistence::config_dir;

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ForkTarget {
    pub session_id: String,
    pub cwd: PathBuf,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn pointer_path(pane_id: &str) -> PathBuf {
    config_dir().join("sessions").join(pane_id)
}

/// Claude Code names its per-project transcript directory after the session cwd
/// with every non-alphanumeric character replaced by `-`. Undocumented internal;
/// `classify` treats a missing projects dir as "rule changed" rather than an error.
#[allow(dead_code)]
pub fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[allow(dead_code)]
pub fn transcript_path(home: &Path, cwd: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(project_slug(cwd))
        .join(format!("{session_id}.jsonl"))
}

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ForkError {
    NoAgent,
    UnsupportedAgent(AgentType),
    NoPointer,
    StalePointer,
    WrongDirectory,
}

#[allow(dead_code)]
pub struct Facts<'a> {
    pub pointer: Option<&'a str>,
    pub agent: Option<AgentType>,
    pub pane_cwd: Option<&'a Path>,
    pub project_dir_exists: bool,
    pub transcript_exists: bool,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
