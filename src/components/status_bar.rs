use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::config::keys::{Action, KeyMode, Keymap};
use crate::core::status::StatusCounts;
use crate::theme::Theme;

/// Simplified view of the app state for the status bar.
/// Constructed by `app.rs` so we don't expose `Mode` publicly.
pub enum StatusContext {
    NormalNone,
    NormalCollection,
    NormalThread,
    NormalSession,
    NormalAgent,
    Notes,
    Input,
    Confirm,
    Finder,
    ThreadPicker,
    AgentsView,
    /// Slot-assign mode: agents view is waiting for a digit to assign a slot to an agent.
    /// Carries the human-readable path of the target agent for display.
    AgentsViewSlotAssign {
        target_path: String,
    },
}

fn right_group(counts: &StatusCounts, session_count: usize, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    // Waiting and Review share the ◐ "your turn" dot in the UI (they stay
    // distinct in the model; see status_glyph), so their counts merge here.
    let review = counts.waiting + counts.review;
    if review > 0 {
        spans.push(Span::styled(format!("◐ {} ", review), theme.status_waiting));
    }
    if counts.working > 0 {
        spans.push(Span::styled(
            format!("● {} ", counts.working),
            theme.status_working,
        ));
    }
    let tail = if session_count > 0 {
        format!("{} active ", session_count)
    } else {
        "tws ".to_string()
    };
    spans.push(Span::styled(tail, theme.statusbar_desc));
    spans
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    ctx: StatusContext,
    area: Rect,
    active_session_count: usize,
    counts: StatusCounts,
    flash: Option<&str>,
    theme: &Theme,
    keymap: &Keymap,
) {
    let slot_assign_prompt: Option<String> = match &ctx {
        StatusContext::AgentsViewSlotAssign { target_path } => Some(format!(
            "Set slot for {} — 0-9 assign · Esc cancel",
            target_path
        )),
        _ => None,
    };

    let hints: Vec<(String, &str)> = match ctx {
        StatusContext::NormalNone => vec![
            (keymap.key_hint(KeyMode::Normal, Action::Quit), "quit"),
            (
                keymap.key_hint(KeyMode::Normal, Action::Enter),
                "quick session",
            ),
            (keymap.key_hint(KeyMode::Normal, Action::Add), "add thread"),
            (
                keymap.key_hint(KeyMode::Normal, Action::AddCollection),
                "add collection",
            ),
            (keymap.key_hint(KeyMode::Normal, Action::Finder), "find"),
            (
                keymap.key_hint(KeyMode::Normal, Action::ExpandAll),
                "toggle all",
            ),
        ],
        StatusContext::NormalCollection => vec![
            (keymap.key_hint(KeyMode::Normal, Action::Quit), "quit"),
            (
                keymap.key_hint(KeyMode::Normal, Action::ToggleSelect),
                "toggle",
            ),
            (keymap.key_hint(KeyMode::Normal, Action::Add), "add thread"),
            (keymap.key_hint(KeyMode::Normal, Action::Rename), "rename"),
            (keymap.key_hint(KeyMode::Normal, Action::Delete), "delete"),
            (keymap.key_hint(KeyMode::Normal, Action::Finder), "find"),
            ("Tab".to_string(), "notes"),
        ],
        StatusContext::NormalThread => vec![
            (keymap.key_hint(KeyMode::Normal, Action::Quit), "quit"),
            (
                keymap.key_hint(KeyMode::Normal, Action::Enter),
                "new session",
            ),
            (keymap.key_hint(KeyMode::Normal, Action::Add), "add thread"),
            (keymap.key_hint(KeyMode::Normal, Action::Rename), "rename"),
            (keymap.key_hint(KeyMode::Normal, Action::Delete), "delete"),
            (
                keymap.key_hint(KeyMode::Normal, Action::KillSession),
                "kill sessions",
            ),
            (keymap.key_hint(KeyMode::Normal, Action::Finder), "find"),
            ("Tab".to_string(), "notes"),
        ],
        StatusContext::NormalSession => vec![
            (keymap.key_hint(KeyMode::Normal, Action::Quit), "quit"),
            (keymap.key_hint(KeyMode::Normal, Action::Enter), "attach"),
            (keymap.key_hint(KeyMode::Normal, Action::Rename), "rename"),
            (keymap.key_hint(KeyMode::Normal, Action::Move), "move"),
            (
                keymap.key_hint(KeyMode::Normal, Action::KillSession),
                "kill",
            ),
            (keymap.key_hint(KeyMode::Normal, Action::Finder), "find"),
            ("Tab".to_string(), "notes"),
        ],
        StatusContext::NormalAgent => vec![
            (keymap.key_hint(KeyMode::Normal, Action::Quit), "quit"),
            (keymap.key_hint(KeyMode::Normal, Action::Enter), "attach"),
            (keymap.key_hint(KeyMode::Normal, Action::Rename), "rename"),
            (keymap.key_hint(KeyMode::Normal, Action::Finder), "find"),
        ],
        StatusContext::Notes => vec![
            (keymap.key_hint(KeyMode::Notes, Action::OpenEditor), "edit"),
            (keymap.key_hint(KeyMode::Notes, Action::Cancel), "back"),
            (
                keymap.key_hint_pair(KeyMode::Notes, Action::ScrollUp, Action::ScrollDown),
                "scroll",
            ),
        ],
        StatusContext::Input => vec![
            (keymap.key_hint(KeyMode::Input, Action::Confirm), "confirm"),
            (keymap.key_hint(KeyMode::Input, Action::Cancel), "cancel"),
        ],
        StatusContext::Confirm => vec![
            (
                keymap.key_hint(KeyMode::ConfirmModal, Action::Confirm),
                "confirm",
            ),
            (
                keymap.key_hint(KeyMode::ConfirmModal, Action::Cancel),
                "cancel",
            ),
        ],
        StatusContext::Finder => vec![
            (keymap.key_hint(KeyMode::Finder, Action::Confirm), "attach"),
            (keymap.key_hint(KeyMode::Finder, Action::Cancel), "cancel"),
            (
                keymap.key_hint_pair(KeyMode::Finder, Action::MoveUp, Action::MoveDown),
                "navigate",
            ),
        ],
        StatusContext::ThreadPicker => vec![
            (keymap.key_hint(KeyMode::Finder, Action::Confirm), "move"),
            (keymap.key_hint(KeyMode::Finder, Action::Cancel), "cancel"),
            (
                keymap.key_hint_pair(KeyMode::Finder, Action::MoveUp, Action::MoveDown),
                "navigate",
            ),
        ],
        StatusContext::AgentsView => vec![
            (
                keymap.key_hint_pair(KeyMode::Agents, Action::MoveUp, Action::MoveDown),
                "navigate",
            ),
            (keymap.key_hint(KeyMode::Agents, Action::Enter), "attach"),
            (keymap.key_hint(KeyMode::Agents, Action::PinAgent), "pin"),
            (
                keymap.key_hint(KeyMode::Agents, Action::PinAgentSlot),
                "set slot",
            ),
            (
                keymap.key_hint(KeyMode::Normal, Action::ToggleView),
                "tree view",
            ),
            (keymap.key_hint(KeyMode::Agents, Action::Quit), "quit"),
        ],
        StatusContext::AgentsViewSlotAssign { .. } => vec![],
    };

    let mut left_spans = Vec::new();
    if let Some(prompt) = &slot_assign_prompt {
        left_spans.push(Span::styled(prompt.as_str(), theme.flash));
    } else if let Some(msg) = flash {
        left_spans.push(Span::styled(msg, theme.flash));
    } else {
        for (i, (key, desc)) in hints.iter().enumerate() {
            if i > 0 {
                left_spans.push(Span::styled("   ", theme.statusbar_desc));
            }
            left_spans.push(Span::styled(key.clone(), theme.statusbar_key));
            left_spans.push(Span::styled(" · ", theme.statusbar_desc));
            left_spans.push(Span::styled(*desc, theme.statusbar_desc));
        }
    }

    let right_spans = right_group(&counts, active_session_count, theme);

    let right_width: u16 = right_spans
        .iter()
        .map(|s| s.content.chars().count() as u16)
        .sum();
    let right_line = Line::from(right_spans);

    let chunks =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).split(area);

    frame.render_widget(Line::from(left_spans), chunks[0]);
    frame.render_widget(
        Paragraph::new(right_line).alignment(Alignment::Right),
        chunks[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::palette::Palette;

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn theme() -> Theme {
        Theme::build(&Palette::default())
    }

    fn counts(working: usize, waiting: usize, review: usize) -> StatusCounts {
        StatusCounts {
            working,
            waiting,
            review,
            idle: 0,
        }
    }

    #[test]
    fn review_and_waiting_agents_merge_into_one_count() {
        let spans = right_group(&counts(0, 1, 2), 0, &theme());
        assert!(text_of(&spans).contains("◐ 3"));
    }

    #[test]
    fn working_agents_render_their_own_count() {
        let spans = right_group(&counts(4, 0, 0), 0, &theme());
        assert!(text_of(&spans).contains("● 4"));
    }

    #[test]
    fn tail_shows_the_session_count() {
        let spans = right_group(&counts(0, 0, 0), 6, &theme());
        assert_eq!(text_of(&spans), "6 active ");
    }

    #[test]
    fn tail_falls_back_to_the_app_name() {
        let spans = right_group(&counts(0, 0, 0), 0, &theme());
        assert_eq!(text_of(&spans), "tws ");
    }
}
