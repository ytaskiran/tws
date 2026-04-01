use ratatui::style::{Color, Modifier, Style};

// Warm palette base colors
pub const ACCENT: Color = Color::Rgb(204, 120, 50);
pub const SAGE_GREEN: Color = Color::Rgb(130, 180, 130);
pub const SUBTLE_BORDER: Color = Color::Rgb(60, 60, 60);
pub const MUTED_TEXT: Color = Color::Rgb(100, 100, 100);
pub const DIM_TEXT: Color = Color::Rgb(160, 160, 160);

// Tree hierarchy styles
pub const COLLECTION_STYLE: Style = Style::new()
    .fg(Color::Rgb(220, 140, 65))
    .add_modifier(Modifier::BOLD);
pub const THREAD_STYLE: Style = Style::new().fg(ACCENT);
pub const THREAD_DIM_STYLE: Style = Style::new().fg(Color::Rgb(130, 80, 35));
pub const SESSION_STYLE: Style = Style::new().fg(SAGE_GREEN);
pub const HIGHLIGHT_STYLE: Style = Style::new()
    .fg(Color::Black)
    .bg(ACCENT)
    .add_modifier(Modifier::BOLD);
pub const HIGHLIGHT_UNFOCUSED_STYLE: Style = Style::new()
    .fg(Color::White)
    .bg(SUBTLE_BORDER);

// Chrome
pub const SEPARATOR_STYLE: Style = Style::new().fg(SUBTLE_BORDER);

// Status bar — intentionally very dim/receded
pub const STATUSBAR_KEY_STYLE: Style = Style::new().fg(Color::Rgb(150, 150, 150));
pub const STATUSBAR_DESC_STYLE: Style = Style::new().fg(Color::Rgb(75, 75, 75));

// Shared cursor (blinking block character)
pub const CURSOR_STYLE: Style = Style::new()
    .fg(ACCENT)
    .add_modifier(Modifier::SLOW_BLINK);

// Modals
pub const MODAL_BORDER_STYLE: Style = Style::new().fg(ACCENT);
pub const MODAL_TITLE_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const MODAL_MUTED_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Empty state
pub const EMPTY_TITLE_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const EMPTY_HINT_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Agent sessions
pub const AGENT_STYLE: Style = Style::new().fg(Color::Rgb(180, 180, 180));

// Badges
pub const BADGE_DOT_STYLE: Style = Style::new().fg(SAGE_GREEN);
pub const BADGE_COUNT_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Flash messages
pub const FLASH_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);

// Recent sessions bar
pub const RECENT_NUMBER_STYLE: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const RECENT_NAME_STYLE: Style = Style::new().fg(DIM_TEXT);

// Scrollbar
pub const SCROLLBAR_THUMB_STYLE: Style = Style::new().fg(MUTED_TEXT);
pub const SCROLLBAR_TRACK_STYLE: Style = Style::new().fg(SUBTLE_BORDER);

// Notes sidebar
pub const NOTES_BORDER_FOCUSED: Style = Style::new().fg(ACCENT);
pub const NOTES_BORDER_UNFOCUSED: Style = Style::new().fg(SUBTLE_BORDER);
pub const NOTES_TITLE_FOCUSED: Style = Style::new().fg(ACCENT).add_modifier(Modifier::BOLD);
pub const NOTES_TITLE_UNFOCUSED: Style = Style::new().fg(DIM_TEXT);
pub const NOTES_PLACEHOLDER_STYLE: Style = Style::new().fg(MUTED_TEXT);

// Markdown rendering — custom stylesheet for tui-markdown
#[derive(Clone)]
pub struct NoteStyleSheet;

impl tui_markdown::StyleSheet for NoteStyleSheet {
    fn heading(&self, level: u8) -> Style {
        match level {
            1 => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            2 => Style::new().fg(ACCENT),
            _ => Style::new().fg(DIM_TEXT).add_modifier(Modifier::ITALIC),
        }
    }

    fn code(&self) -> Style {
        Style::new().fg(SAGE_GREEN)
    }

    fn link(&self) -> Style {
        Style::new()
            .fg(ACCENT)
            .add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        Style::new()
            .fg(MUTED_TEXT)
            .add_modifier(Modifier::ITALIC)
    }

    fn heading_meta(&self) -> Style {
        Style::new().fg(MUTED_TEXT)
    }

    fn metadata_block(&self) -> Style {
        Style::new().fg(MUTED_TEXT)
    }
}
