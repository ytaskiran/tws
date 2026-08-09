use std::path::{Path, PathBuf};

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
}
