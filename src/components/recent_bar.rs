use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

/// Renders a vertical list of recently attached sessions with number shortcuts.
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
