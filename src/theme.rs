use ratatui::style::{Color, Modifier, Style};

// Warm palette base colors
pub const ACCENT: Color = Color::Rgb(204, 120, 50);
pub const SAGE_GREEN: Color = Color::Rgb(130, 180, 130);
pub const SUBTLE_BORDER: Color = Color::Rgb(60, 60, 60);
pub const MUTED_TEXT: Color = Color::Rgb(100, 100, 100);
pub const DIM_TEXT: Color = Color::Rgb(160, 160, 160);

// Tree hierarchy styles
pub const COLLECTION_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const THREAD_STYLE: Style = Style::new().fg(Color::White);
pub const THREAD_DIM_STYLE: Style = Style::new().fg(DIM_TEXT);
pub const SESSION_STYLE: Style = Style::new().fg(SAGE_GREEN);
pub const HIGHLIGHT_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(ACCENT)
    .add_modifier(Modifier::BOLD);

// Chrome
pub const BORDER_STYLE: Style = Style::new().fg(SUBTLE_BORDER);
pub const SEPARATOR_STYLE: Style = Style::new().fg(SUBTLE_BORDER);

// Status bar — intentionally very dim/receded
pub const STATUSBAR_KEY_STYLE: Style = Style::new().fg(Color::Rgb(150, 150, 150));
pub const STATUSBAR_DESC_STYLE: Style = Style::new().fg(Color::Rgb(75, 75, 75));

// Modals
pub const MODAL_BORDER_STYLE: Style = Style::new().fg(ACCENT);
pub const MODAL_TITLE_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const MODAL_MUTED_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Empty state
pub const EMPTY_TITLE_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const EMPTY_HINT_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Badges
pub const BADGE_DOT_STYLE: Style = Style::new().fg(SAGE_GREEN);
pub const BADGE_COUNT_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Flash messages
pub const FLASH_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
