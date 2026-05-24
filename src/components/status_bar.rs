use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;

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
    AgentsViewSlotAssign { target_path: String },
}

pub fn render(frame: &mut Frame, ctx: StatusContext, area: Rect, active_session_count: usize, flash: Option<&str>) {
    // Slot-assign mode renders a custom prompt; render it ahead of the standard hint table.
    let slot_assign_prompt: Option<String> = match &ctx {
        StatusContext::AgentsViewSlotAssign { target_path } => {
            Some(format!("Set slot for {} — 0-9 assign · Esc cancel", target_path))
        }
        _ => None,
    };

    let hints: &[(&str, &str)] = match ctx {
        StatusContext::NormalNone => &[("q", "quit"), ("Enter", "quick session"), ("a", "add thread"), ("A", "add collection"), ("/", "find"), ("e", "toggle all")],
        StatusContext::NormalCollection => &[
            ("q", "quit"),
            ("Space", "toggle"),
            ("a", "add thread"),
            ("r", "rename"),
            ("d", "delete"),
            ("/", "find"),
            ("Tab", "notes"),
        ],
        StatusContext::NormalThread => &[
            ("q", "quit"),
            ("Enter", "new session"),
            ("a", "add thread"),
            ("r", "rename"),
            ("d", "delete"),
            ("x", "kill sessions"),
            ("/", "find"),
            ("Tab", "notes"),
        ],
        StatusContext::NormalSession => &[("q", "quit"), ("Enter", "attach"), ("r", "rename"), ("m", "move"), ("x", "kill"), ("/", "find"), ("Tab", "notes")],
        StatusContext::NormalAgent => &[("q", "quit"), ("Enter", "attach"), ("r", "rename"), ("/", "find")],
        StatusContext::Notes => &[("Enter", "edit"), ("Esc", "back"), ("j/k", "scroll")],
        StatusContext::Input => &[("Enter", "confirm"), ("Esc", "cancel")],
        StatusContext::Confirm => &[("y", "confirm"), ("Esc", "cancel")],
        StatusContext::Finder => &[("Enter", "attach"), ("Esc", "cancel"), ("\u{2191}\u{2193}", "navigate")],
        StatusContext::ThreadPicker => &[("Enter", "move"), ("Esc", "cancel"), ("\u{2191}\u{2193}", "navigate")],
        StatusContext::AgentsView => &[("j/k", "navigate"), ("Enter", "attach"), ("p", "pin"), ("0-9", "jump"), ("P", "set slot"), ("v", "tree view"), ("q", "quit")],
        // Hints empty when assign prompt is active — prompt itself replaces the hint row.
        StatusContext::AgentsViewSlotAssign { .. } => &[],
    };

    // Left side: assign prompt > flash message > key hints
    let mut left_spans = Vec::new();
    if let Some(prompt) = &slot_assign_prompt {
        left_spans.push(Span::styled(prompt.as_str(), theme::FLASH_STYLE));
    } else if let Some(msg) = flash {
        left_spans.push(Span::styled(msg, theme::FLASH_STYLE));
    } else {
        for (i, (key, desc)) in hints.iter().enumerate() {
            if i > 0 {
                left_spans.push(Span::styled("   ", theme::STATUSBAR_DESC_STYLE));
            }
            left_spans.push(Span::styled(*key, theme::STATUSBAR_KEY_STYLE));
            left_spans.push(Span::styled(" · ", theme::STATUSBAR_DESC_STYLE));
            left_spans.push(Span::styled(*desc, theme::STATUSBAR_DESC_STYLE));
        }
    }

    // Right side: session count or app name
    let right_text = if active_session_count > 0 {
        format!("{} active ", active_session_count)
    } else {
        "tws ".to_string()
    };
    let right_line = Line::from(Span::styled(&*right_text, theme::STATUSBAR_DESC_STYLE));
    let right_width = right_text.len() as u16;

    let chunks = Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)])
        .split(area);

    frame.render_widget(Line::from(left_spans), chunks[0]);
    frame.render_widget(
        Paragraph::new(right_line).alignment(Alignment::Right),
        chunks[1],
    );
}
