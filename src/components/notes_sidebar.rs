use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use crate::theme::Theme;

/// Data needed to render the notes sidebar.
pub struct SidebarState<'a> {
    pub rendered: Option<&'a Text<'static>>,
    pub scroll_offset: usize,
    pub is_empty: bool,
    pub title: &'a str,
    pub focused: bool,
}

/// Render the notes sidebar as a read-only markdown preview.
pub fn render(frame: &mut Frame, state: &SidebarState<'_>, area: Rect, theme: &Theme) {
    let (border_style, title_style) = if state.focused {
        (theme.notes_border_focused, theme.notes_title_focused)
    } else {
        (theme.notes_border_unfocused, theme.notes_title_unfocused)
    };

    // One rule on the left instead of a box. The label sits on the first row,
    // and the content starts one blank row below it.
    let block = Block::new()
        .borders(Borders::LEFT)
        .border_style(border_style)
        .padding(Padding::new(2, 1, 0, 0));
    let padded = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(state.title, title_style))),
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

    if state.is_empty {
        let msg = if state.focused {
            "Enter to edit"
        } else {
            "Tab to add notes"
        };
        let placeholder = Paragraph::new(Line::from(Span::styled(msg, theme.notes_placeholder)));
        frame.render_widget(placeholder, inner);
        return;
    }

    if let Some(text) = state.rendered {
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
}
