use ratatui::layout::{Constraint, Flex, Layout, Rect};

pub mod agent_preview;
pub mod agents_view;
pub mod confirm_modal;
pub mod dir_picker_modal;
pub mod finder_modal;
pub mod input_modal;
pub mod notes_sidebar;
pub mod recent_bar;
pub mod status_bar;
pub mod tree_view;

/// Centers a popup of fixed `height` and `percent_x` width inside `area`.
pub fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
