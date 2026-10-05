//! The dim harness name at the end of an agent row, such as `claude`. Both
//! the sessions view and the agents view use it, so the same agent reads the
//! same way in each view. The subagent count and its
//! spinner live here for the same reason.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::core::model::AgentType;

/// Current Unix time in seconds.
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
