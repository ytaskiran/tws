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
    Notes,
    Input,
    Confirm,
    Finder,
}

pub fn render(frame: &mut Frame, ctx: StatusContext, area: Rect, active_session_count: usize, flash: Option<&str>) {
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
        StatusContext::NormalSession => &[("q", "quit"), ("Enter", "attach"), ("r", "rename"), ("x", "kill"), ("/", "find"), ("Tab", "notes")],
        StatusContext::Notes => &[("Enter", "edit"), ("Esc", "back"), ("j/k", "scroll")],
        StatusContext::Input => &[("Enter", "confirm"), ("Esc", "cancel")],
        StatusContext::Confirm => &[("y", "confirm"), ("Esc", "cancel")],
        StatusContext::Finder => &[("Enter", "attach"), ("Esc", "cancel"), ("\u{2191}\u{2193}", "navigate")],
    };

    // Left side: flash message (if active) or key hints
    let mut left_spans = Vec::new();
    if let Some(msg) = flash {
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
