use ratatui::layout::{Constraint, Flex, Layout, Rect};

pub mod agent_preview;
pub mod agents_view;
pub mod confirm_modal;
pub mod dir_picker_modal;
pub mod finder_modal;
pub mod input_modal;
pub mod notes_sidebar;
pub mod recent_bar;
pub mod sessions_view;
pub mod status_bar;

/// Centers a popup of fixed `height` and `percent_x` width inside `area`.
pub fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}

/// Scroll offset that keeps row `selected` inside a list `height` rows tall.
/// The offset is 0 until the row passes the bottom edge, then the row stays
/// on the last visible line.
pub fn scroll_to_keep_visible(selected: usize, height: u16) -> u16 {
    let offset = selected.saturating_sub((height as usize).saturating_sub(1));
    u16::try_from(offset).unwrap_or(u16::MAX)
}
