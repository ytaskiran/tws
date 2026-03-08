use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;

/// Renders a vertical list of recently attached sessions with number shortcuts.
///
/// Each session gets its own line: `1 Collection/Thread/label`
/// Numbers are styled with RECENT_NUMBER_STYLE, paths with RECENT_NAME_STYLE.
pub fn render(frame: &mut Frame, sessions: &[(String, String)], area: Rect) {
    let lines: Vec<Line> = sessions
        .iter()
        .enumerate()
        .map(|(i, (_tmux_name, display_path))| {
            Line::from(vec![
                Span::styled(format!("{}", i + 1), theme::RECENT_NUMBER_STYLE),
                Span::raw(" "),
                Span::styled(display_path.clone(), theme::RECENT_NAME_STYLE),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}
