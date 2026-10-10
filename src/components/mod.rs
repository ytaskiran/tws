use ratatui::layout::{Constraint, Flex, Layout, Rect};

pub mod agent_meta;
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

/// Scroll offset for a list of `len` lines, `height` tall, that keeps lines
/// `first..=last` visible and moves as little as possible from `prev`. A
/// hover selects a row, so the list must not move while the selection stays
/// on screen. A pane too short for the whole range shows `first`.
pub fn sticky_scroll(prev: u16, first: usize, last: usize, len: usize, height: u16) -> u16 {
    let height = usize::from(height.max(1));
    let lo = last.saturating_sub(height - 1).min(first);
    let offset = usize::from(prev)
        .min(len.saturating_sub(height))
        .clamp(lo, first);
    u16::try_from(offset).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticky_scroll_moves_only_when_the_selection_leaves_the_screen() {
        // 10 lines, 4 visible, one-line selection.
        assert_eq!(sticky_scroll(0, 2, 2, 10, 4), 0, "visible: stay");
        assert_eq!(sticky_scroll(0, 5, 5, 10, 4), 2, "below: last line");
        assert_eq!(
            sticky_scroll(5, 7, 7, 10, 4),
            5,
            "visible after scroll: stay"
        );
        assert_eq!(sticky_scroll(5, 3, 3, 10, 4), 3, "above: first line");
        assert_eq!(
            sticky_scroll(8, 9, 9, 10, 4),
            6,
            "no blank lines at the end"
        );
        assert_eq!(sticky_scroll(3, 0, 0, 10, 0), 0, "zero height");
    }

    #[test]
    fn sticky_scroll_keeps_a_two_line_selection_on_screen() {
        assert_eq!(
            sticky_scroll(0, 3, 4, 10, 4),
            1,
            "second line below: scroll it in"
        );
        assert_eq!(
            sticky_scroll(5, 4, 5, 10, 4),
            4,
            "first line above: show it"
        );
        assert_eq!(
            sticky_scroll(0, 3, 4, 10, 1),
            3,
            "one-line pane: show the first"
        );
    }
}
