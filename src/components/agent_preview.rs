use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::theme;

/// Data needed to render the agent preview sidebar.
pub struct PreviewState<'a> {
    pub content: Option<&'a Text<'static>>,
    pub scroll_offset: usize,
    pub title: &'a str,
}

/// Render the agent pane preview as a read-only panel pinned to the bottom.
pub fn render(frame: &mut Frame, state: &PreviewState<'_>, area: Rect) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", state.title))
        .title_style(theme::PREVIEW_TITLE_STYLE)
        .border_style(theme::PREVIEW_BORDER_STYLE);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    match state.content {
        Some(text) => {
            let paragraph = Paragraph::new(text.clone())
                .scroll((state.scroll_offset as u16, 0));
            frame.render_widget(paragraph, inner);

            let total_lines = text.lines.len();
            let visible_height = inner.height as usize;
            if total_lines > visible_height {
                let mut scrollbar_state =
                    ScrollbarState::new(total_lines.saturating_sub(visible_height))
                        .position(state.scroll_offset);
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(theme::SCROLLBAR_THUMB_STYLE)
                    .track_style(theme::SCROLLBAR_TRACK_STYLE);
                frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
            }
        }
        None => {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                "No preview available",
                theme::PREVIEW_PLACEHOLDER_STYLE,
            )));
            frame.render_widget(placeholder, inner);
        }
    }
}
