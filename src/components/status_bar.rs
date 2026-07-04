use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::config::keys::{Action, KeyMode, Keymap};
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

pub fn render(
    frame: &mut Frame,
    ctx: StatusContext,
    area: Rect,
    active_session_count: usize,
    flash: Option<&str>,
    theme: &Theme,
    keymap: &Keymap,
) {
    // Slot-assign mode renders a custom prompt instead of the hint row.
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
        // Hint row is empty when assign prompt is active — prompt replaces it.
        StatusContext::AgentsViewSlotAssign { .. } => vec![],
    };

    // Left side: assign prompt > flash message > key hints
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

    // Right side: session count or app name
    let right_text = if active_session_count > 0 {
        format!("{} active ", active_session_count)
    } else {
        "tws ".to_string()
    };
    let right_line = Line::from(Span::styled(&*right_text, theme.statusbar_desc));
    let right_width = right_text.len() as u16;

    let chunks =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).split(area);

    frame.render_widget(Line::from(left_spans), chunks[0]);
    frame.render_widget(
        Paragraph::new(right_line).alignment(Alignment::Right),
        chunks[1],
    );
}
