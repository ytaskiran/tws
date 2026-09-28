use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::theme::Theme;

/// Data needed to render the agent preview sidebar.
pub struct PreviewState<'a> {
    pub content: Option<&'a Text<'static>>,
    pub scroll_offset: usize,
    pub title: &'a str,
}

/// Render the agent pane preview as a read-only panel pinned to the bottom.
pub fn render(frame: &mut Frame, state: &PreviewState<'_>, area: Rect, theme: &Theme) {
    // Same frame as the notes pane: one left rule and a label row.
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(theme.preview_border)
        .padding(Padding::new(2, 1, 0, 0));
    let padded = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(state.title, theme.preview_title))),
        padded,
    );
    let inner = Rect {
        y: padded.y + 2,
        height: padded.height.saturating_sub(2),
        ..padded
    };

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    match state.content {
        Some(text) => {
            let paragraph = Paragraph::new(text.clone()).scroll((state.scroll_offset as u16, 0));
            frame.render_widget(paragraph, inner);

            let total_lines = text.lines.len();
            let visible_height = inner.height as usize;
            if total_lines > visible_height {
                let mut scrollbar_state =
                    ScrollbarState::new(total_lines.saturating_sub(visible_height))
                        .position(state.scroll_offset);
                let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(theme.scrollbar_thumb)
                    .track_style(theme.scrollbar_track);
                frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
            }
        }
        None => {
            let placeholder = Paragraph::new(Line::from(Span::styled(
                "No preview available",
                theme.preview_placeholder,
            )));
            frame.render_widget(placeholder, inner);
        }
    }
}
