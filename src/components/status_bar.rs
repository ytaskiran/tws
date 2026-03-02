use ratatui::prelude::*;
use ratatui::Frame;

use crate::theme;

/// Simplified view of the app state for the status bar.
/// Constructed by `app.rs` so we don't expose `Mode` publicly.
pub enum StatusContext {
    NormalNone,
    NormalCollection,
    NormalProject,
    NormalSession,
    Input,
    Confirm,
}

pub fn render(frame: &mut Frame, ctx: StatusContext, area: Rect) {
    let hints: &[(&str, &str)] = match ctx {
        StatusContext::NormalNone => &[("q", "quit"), ("a", "add collection")],
        StatusContext::NormalCollection => &[
            ("q", "quit"),
            ("Space", "toggle"),
            ("a", "add project"),
            ("r", "rename"),
            ("d", "delete"),
        ],
        StatusContext::NormalProject => &[
            ("q", "quit"),
            ("Enter", "new session"),
            ("a", "add project"),
            ("r", "rename"),
            ("d", "delete"),
            ("x", "kill sessions"),
        ],
        StatusContext::NormalSession => &[("q", "quit"), ("Enter", "attach"), ("x", "kill")],
        StatusContext::Input => &[("Enter", "confirm"), ("Esc", "cancel")],
        StatusContext::Confirm => &[("y", "confirm"), ("n", "cancel")],
    };

    let mut spans = Vec::new();
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(format!(" {} ", key), theme::STATUSBAR_KEY_STYLE));
        spans.push(Span::styled(format!(" {}", desc), theme::STATUSBAR_DESC_STYLE));
    }

    frame.render_widget(Line::from(spans), area);
}
