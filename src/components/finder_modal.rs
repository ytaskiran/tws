use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::components::centered_rect;
use crate::theme::Theme;

const MAX_VISIBLE: usize = 10;

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    title: &str,
    // None hides the query line: a plain list with no filter.
    query: Option<&str>,
    entries: &[(String, String)],
    filtered: &[usize],
    cursor: usize,
    area: Rect,
    theme: &Theme,
) {
    let visible_count = filtered.len().min(MAX_VISIBLE);
    let query_rows = if query.is_some() { 2 } else { 0 };
    let height = (visible_count.max(1) + 3 + query_rows) as u16;
    let popup = centered_rect(60, height, area);
    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .style(theme.background)
        .title(title)
        .title_style(theme.modal_title)
        .border_style(theme.modal_border)
        .padding(Padding::new(1, 1, 1, 0));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::vertical([
        Constraint::Length(query_rows as u16 / 2),
        Constraint::Length(query_rows as u16 / 2),
        Constraint::Min(0),
    ])
    .split(inner);

    if let Some(query) = query {
        let query_line = Line::from(vec![
            Span::styled("/", theme.modal_muted),
            Span::raw(query),
            Span::styled("\u{2588}", theme.cursor),
        ]);
        frame.render_widget(Paragraph::new(query_line), chunks[0]);

        let sep = "\u{2500}".repeat(chunks[1].width as usize);
        frame.render_widget(
            Paragraph::new(Line::styled(sep, theme.separator)),
            chunks[1],
        );
    }

    if filtered.is_empty() {
        let empty = Line::from(Span::styled("No matches", theme.modal_muted));
        frame.render_widget(Paragraph::new(empty), chunks[2]);
        return;
    }

    let max_rows = chunks[2].height as usize;
    let scroll_offset = if cursor >= max_rows {
        cursor - max_rows + 1
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::with_capacity(max_rows);
    for (vi, &entry_idx) in filtered
        .iter()
        .skip(scroll_offset)
        .take(max_rows)
        .enumerate()
    {
        let (_, path) = &entries[entry_idx];
        let is_selected = scroll_offset + vi == cursor;
        let style = if is_selected {
            theme.highlight
        } else {
            Style::new().fg(theme.dim_text)
        };

        let prefix = if is_selected { " \u{203A} " } else { "   " };
        let line = Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(path.as_str(), style),
        ]);
        lines.push(line);
    }

    frame.render_widget(Paragraph::new(lines), chunks[2]);
}
