use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: Uuid,
    pub name: String,
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
}

/// Runtime-only, never serialized. Represents a live tmux session.
#[derive(Debug, Clone)]
pub struct Session {
    pub tmux_session_name: String,
    pub display_name: String,
    pub thread_id: Uuid,
    pub alive: bool,
}

impl Collection {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            threads: Vec::new(),
        }
    }
}

impl Thread {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
        }
    }
}

/// Converts a string to a URL-style slug: lowercase, non-alphanumeric → hyphens,
/// consecutive hyphens collapsed, leading/trailing hyphens stripped.
fn slugify(s: &str) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();

    // Collapse consecutive hyphens
    let mut result = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push('-');
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }

    // Strip leading/trailing hyphens
    result.trim_matches('-').to_string()
}

/// Generates the base prefix for tmux session names for a given collection/thread.
/// Format: `tws_{collection_slug}_{thread_slug}`
///
/// Individual sessions append `_1`, `_2`, etc.
pub fn tmux_session_prefix(collection_name: &str, thread_name: &str) -> String {
    format!("tws_{}_{}", slugify(collection_name), slugify(thread_name))
}

/// Generates a labeled tmux session name.
/// Format: `tws_{collection_slug}_{thread_slug}_{label_slug}`
pub fn tmux_session_name_labeled(collection_name: &str, thread_name: &str, label: &str) -> String {
    format!("{}_{}", tmux_session_prefix(collection_name, thread_name), slugify(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_has_unique_id() {
        let a = Collection::new("Work");
        let b = Collection::new("Work");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn thread_has_unique_id() {
        let a = Thread::new("Rust Book");
        let b = Thread::new("Rust Book");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn collection_starts_empty() {
        let c = Collection::new("Test");
        assert!(c.threads.is_empty());
    }

    #[test]
    fn thread_description_defaults_to_none() {
        let p = Thread::new("Test");
        assert!(p.description.is_none());
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("CI/CD Overhaul"), "ci-cd-overhaul");
    }

    #[test]
    fn slugify_consecutive_special() {
        assert_eq!(slugify("foo---bar"), "foo-bar");
    }

    #[test]
    fn slugify_leading_trailing() {
        assert_eq!(slugify("--hello--"), "hello");
    }

    #[test]
    fn slugify_unicode() {
        assert_eq!(slugify("Derin Notlar"), "derin-notlar");
    }

    #[test]
    fn tmux_session_prefix_format() {
        assert_eq!(
            tmux_session_prefix("Work", "Edge Device Pipeline"),
            "tws_work_edge-device-pipeline"
        );
    }

    #[test]
    fn tmux_session_name_labeled_format() {
        assert_eq!(
            tmux_session_name_labeled("Work", "Edge Device Pipeline", "bugfix"),
            "tws_work_edge-device-pipeline_bugfix"
        );
        assert_eq!(
            tmux_session_name_labeled("Work", "Edge Device Pipeline", "Hot Fix 2"),
            "tws_work_edge-device-pipeline_hot-fix-2"
        );
    }

    #[test]
    fn tmux_session_prefix_special_chars() {
        assert_eq!(
            tmux_session_prefix("Derin Notlar Podcast", "Episode 13 - Planning"),
            "tws_derin-notlar-podcast_episode-13-planning"
        );
    }
}
