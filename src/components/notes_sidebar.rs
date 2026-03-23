use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

use crate::theme;

/// Render the notes sidebar.
///
/// Accepts raw display data rather than a NoteEditor reference to avoid
/// borrow conflicts with the render closure that also needs `&mut tree_state`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    lines_data: &[String],
    cursor: (usize, usize), // (row, col)
    scroll_offset: usize,
    is_empty: bool,
    title: &str,
    focused: bool,
    area: Rect,
) {
    let (border_style, title_style) = if focused {
        (theme::NOTES_BORDER_FOCUSED, theme::NOTES_TITLE_FOCUSED)
    } else {
        (theme::NOTES_BORDER_UNFOCUSED, theme::NOTES_TITLE_UNFOCUSED)
    };

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", title))
        .title_style(title_style)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let visible_height = inner.height as usize;

    if is_empty && !focused {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "Tab to add notes",
            theme::NOTES_PLACEHOLDER_STYLE,
        )));
        frame.render_widget(placeholder, inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(visible_height);
    let start = scroll_offset;
    let end = (start + visible_height).min(lines_data.len());
    let (cursor_row, cursor_col) = cursor;

    for (row_idx, line_text) in lines_data.iter().enumerate().skip(start).take(end - start) {
        let is_cursor_line = focused && row_idx == cursor_row;

        if is_cursor_line {
            let col = cursor_col.min(line_text.len());
            let before = &line_text[..col];
            let after = if col < line_text.len() {
                &line_text[col + 1..]
            } else {
                ""
            };

            let cursor_char = if col < line_text.len() {
                &line_text[col..col + 1]
            } else {
                "\u{2588}"
            };

            let mut spans = vec![Span::raw(before.to_string())];
            spans.push(Span::styled(cursor_char.to_string(), theme::CURSOR_STYLE));
            if !after.is_empty() {
                spans.push(Span::raw(after.to_string()));
            }
            lines.push(Line::from(spans));
        } else {
            lines.push(Line::from(line_text.as_str()));
        }
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}
