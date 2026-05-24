use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use ratatui::Frame;

use crate::theme::Theme;

/// Maximum number of result rows visible at once.
const MAX_VISIBLE: usize = 10;

pub fn render(
    frame: &mut Frame,
    title: &str,
    query: &str,
    entries: &[(String, String)],
    filtered: &[usize],
    cursor: usize,
    area: Rect,
    theme: &Theme,
) {
    let visible_count = filtered.len().min(MAX_VISIBLE);
    // borders (2) + query line (1) + separator (1) + padding top (1) + at least 1 result row
    let height = (visible_count.max(1) + 5) as u16;
    let popup = centered_rect(60, height, area);
    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(title)
        .title_style(theme.modal_title)
        .border_style(theme.modal_border)
        .padding(Padding::new(1, 1, 1, 0));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    // Layout: query line, separator, results area
    let chunks = Layout::vertical([
        Constraint::Length(1), // query
        Constraint::Length(1), // separator
        Constraint::Min(0),   // results
    ])
    .split(inner);

    // Query line with blinking cursor
    let query_line = Line::from(vec![
        Span::styled("/", theme.modal_muted),
        Span::raw(query),
        Span::styled("\u{2588}", theme.cursor),
    ]);
    frame.render_widget(Paragraph::new(query_line), chunks[0]);

    // Separator
    let sep = "\u{2500}".repeat(chunks[1].width as usize);
    frame.render_widget(
        Paragraph::new(Line::styled(sep, theme.separator)),
        chunks[1],
    );

    // Results
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
    for (vi, &entry_idx) in filtered.iter().skip(scroll_offset).take(max_rows).enumerate() {
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

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
