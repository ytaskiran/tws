//! The dim `kind · age` text at the end of an agent row, such as
//! `claude · 4m`. Both the sessions view and the agents view use it, so the
//! same agent reads the same way in each view.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::model::AgentType;

/// Current Unix time in seconds, for `age` and `label`.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

pub fn kind_label(kind: AgentType) -> &'static str {
    match kind {
        AgentType::ClaudeCode => "claude",
        AgentType::Codex => "codex",
        AgentType::Pi => "pi",
    }
}

/// Compact age of a status: "now", "4m", "2h", "3d". `None` when unknown.
pub fn age(since: i64, now: i64) -> Option<String> {
    if since <= 0 {
        return None;
    }
    let secs = (now - since).max(0);
    Some(match secs {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    })
}

/// `claude · 4m`, or `claude` alone when the status time is unknown. Running
/// subagents add a count between them: `claude · ⑂3 · 4m`.
pub fn label(kind: AgentType, status_since: i64, subagents: usize, now: i64) -> String {
    let mut s = kind_label(kind).to_string();
    if subagents > 0 {
        s += &format!(" · ⑂{subagents}");
    }
    if let Some(t) = age(status_since, now) {
        s += &format!(" · {t}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_uses_compact_units() {
        assert_eq!(age(0, 1000), None);
        assert_eq!(age(1000, 1030).as_deref(), Some("now"));
        assert_eq!(age(1000, 1000 + 4 * 60).as_deref(), Some("4m"));
        assert_eq!(age(1000, 1000 + 2 * 3600).as_deref(), Some("2h"));
        assert_eq!(age(1000, 1000 + 3 * 86_400).as_deref(), Some("3d"));
    }

    #[test]
    fn label_joins_kind_and_age() {
        assert_eq!(label(AgentType::ClaudeCode, 1000, 0, 1240), "claude · 4m");
        assert_eq!(label(AgentType::Pi, 0, 0, 1240), "pi");
    }

    #[test]
    fn label_shows_running_subagents() {
        assert_eq!(
            label(AgentType::ClaudeCode, 1000, 3, 1240),
            "claude · ⑂3 · 4m"
        );
        assert_eq!(label(AgentType::ClaudeCode, 0, 1, 1240), "claude · ⑂1");
    }
}
