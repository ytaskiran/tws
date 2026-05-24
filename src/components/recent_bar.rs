use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme::Theme;

/// Renders a vertical list of recently attached sessions with number shortcuts.
///
/// Each session gets its own line: `1 Collection/Thread/label`
/// Numbers are styled with RECENT_NUMBER_STYLE, paths with RECENT_NAME_STYLE.
pub fn render(frame: &mut Frame, sessions: &[(String, String)], area: Rect, theme: &Theme) {
    let lines: Vec<Line> = sessions
        .iter()
        .enumerate()
        .map(|(i, (_tmux_name, display_path))| {
            Line::from(vec![
                Span::styled(format!("{}", i + 1), theme.recent_number),
                Span::raw(" "),
                Span::styled(display_path.clone(), theme.recent_name),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}
