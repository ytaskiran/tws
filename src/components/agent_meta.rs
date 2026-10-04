//! The dim `kind · age` text at the end of an agent row, such as
//! `claude · 4m`. Both the sessions view and the agents view use it, so the
//! same agent reads the same way in each view. The subagent count and its
//! spinner live here for the same reason.

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

/// `claude · 4m`, or `claude` alone when the status time is unknown.
pub fn label(kind: AgentType, status_since: i64, now: i64) -> String {
    match age(status_since, now) {
        Some(t) => format!("{} · {}", kind_label(kind), t),
        None => kind_label(kind).to_string(),
    }
}

/// Current Unix time in milliseconds, for `spinner`.
pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// The app redraws on each 250 ms key poll, so a faster step would skip frames.
const SPINNER_STEP_MS: u128 = 250;

/// The spinner frame at `now_ms`. The frame comes from the clock, so every
/// spinner on the screen shows the same frame and none keeps state.
pub fn spinner(now_ms: u128) -> char {
    SPINNER[(now_ms / SPINNER_STEP_MS % SPINNER.len() as u128) as usize]
}

pub fn subagents(n: usize) -> String {
    format!("{n} subagent{}", if n == 1 { "" } else { "s" })
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
        assert_eq!(label(AgentType::ClaudeCode, 1000, 1240), "claude · 4m");
        assert_eq!(label(AgentType::Pi, 0, 1240), "pi");
    }

    #[test]
    fn spinner_steps_once_per_redraw_tick_and_wraps() {
        assert_eq!(spinner(0), '⠋');
        assert_eq!(spinner(249), '⠋');
        assert_eq!(spinner(250), '⠙');
        assert_eq!(spinner(10 * 250), '⠋');
    }

    #[test]
    fn subagent_count_reads_singular_and_plural() {
        assert_eq!(subagents(1), "1 subagent");
        assert_eq!(subagents(3), "3 subagents");
    }
}
