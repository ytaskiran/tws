use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;

use crate::core::notes::NoteEditor;
use crate::theme;

pub fn render(frame: &mut Frame, editor: &NoteEditor, title: &str, focused: bool, area: Rect) {
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

    // Inner area after border (2 chars horizontal, 2 chars vertical)
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let visible_height = inner.height as usize;

    // Empty state placeholder
    if editor.is_empty() && !focused {
        let placeholder = Paragraph::new(Line::from(Span::styled(
            "Tab to add notes",
            theme::NOTES_PLACEHOLDER_STYLE,
        )));
        frame.render_widget(placeholder, inner);
        return;
    }

    // Build visible lines with cursor
    let cursor_style = Style::new()
        .fg(theme::ACCENT)
        .add_modifier(Modifier::SLOW_BLINK);

    let mut lines: Vec<Line> = Vec::with_capacity(visible_height);
    let start = editor.scroll_offset;
    let end = (start + visible_height).min(editor.lines.len());

    for row_idx in start..end {
        let line_text = &editor.lines[row_idx];
        let is_cursor_line = focused && row_idx == editor.cursor_row;

        if is_cursor_line {
            let col = editor.cursor_col.min(line_text.len());
            let before = &line_text[..col];
            let cursor_char = if col < line_text.len() {
                &line_text[col..col + 1]
            } else {
                "\u{2588}" // block cursor at end of line
            };
            let after = if col < line_text.len() {
                &line_text[col + 1..]
            } else {
                ""
            };

            let mut spans = vec![Span::raw(before.to_string())];
            if col < line_text.len() {
                spans.push(Span::styled(cursor_char.to_string(), cursor_style));
            } else {
                spans.push(Span::styled("\u{2588}".to_string(), cursor_style));
            }
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
