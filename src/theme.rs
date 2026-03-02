use ratatui::style::{Color, Modifier, Style};

pub const COLLECTION_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
pub const PROJECT_STYLE: Style = Style::new().fg(Color::White);
pub const HIGHLIGHT_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(Color::Cyan)
    .add_modifier(Modifier::BOLD);
pub const SESSION_STYLE: Style = Style::new().fg(Color::Green);
pub const BORDER_STYLE: Style = Style::new().fg(Color::DarkGray);
pub const TITLE_STYLE: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
