use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use ratatui::Frame;

use crate::theme;

/// Data needed to render the notes sidebar.
pub struct SidebarState<'a> {
    pub rendered: Option<&'a Text<'static>>,
    pub scroll_offset: usize,
    pub is_empty: bool,
    pub title: &'a str,
    pub focused: bool,
}

/// Render the notes sidebar as a read-only markdown preview.
pub fn render(frame: &mut Frame, state: &SidebarState<'_>, area: Rect) {
    let (border_style, title_style) = if state.focused {
        (theme::NOTES_BORDER_FOCUSED, theme::NOTES_TITLE_FOCUSED)
    } else {
        (theme::NOTES_BORDER_UNFOCUSED, theme::NOTES_TITLE_UNFOCUSED)
    };

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", state.title))
        .title_style(title_style)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if state.is_empty {
        let msg = if state.focused { "Enter to edit" } else { "Tab to add notes" };
        let placeholder = Paragraph::new(Line::from(Span::styled(msg, theme::NOTES_PLACEHOLDER_STYLE)));
        frame.render_widget(placeholder, inner);
        return;
    }

    if let Some(text) = state.rendered {
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
}
