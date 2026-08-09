use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::components::centered_rect;
use crate::core::workdir::{DirPicker, shorten_home};
use crate::theme::Theme;

const MAX_VISIBLE: usize = 10;

#[allow(dead_code)]
pub fn render(frame: &mut Frame, picker: &DirPicker, thread_name: &str, area: Rect, theme: &Theme) {
    let names = picker.filtered_names();
    let visible_count = names.len().min(MAX_VISIBLE);
    let height = (visible_count.max(1) + 5) as u16;
    let popup = centered_rect(60, height, area);
    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .style(theme.background)
        .title(format!(" Directory for \"{}\" ", thread_name))
        .title_style(theme.modal_title)
        .border_style(theme.modal_border)
        .padding(Padding::new(1, 1, 1, 0));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    let mut base = shorten_home(picker.current());
    if !base.ends_with('/') {
        base.push('/');
    }
    let path_line = Line::from(vec![
        Span::styled(base, theme.modal_muted),
        Span::raw(picker.query().to_string()),
        Span::styled("\u{2588}", theme.cursor),
    ]);
    frame.render_widget(Paragraph::new(path_line), chunks[0]);

    let sep = "\u{2500}".repeat(chunks[1].width as usize);
    frame.render_widget(
        Paragraph::new(Line::styled(sep, theme.separator)),
        chunks[1],
    );

    if names.is_empty() {
        let empty = Line::from(Span::styled("No subdirectories", theme.modal_muted));
        frame.render_widget(Paragraph::new(empty), chunks[2]);
        return;
    }

    let max_rows = chunks[2].height as usize;
    let cursor = picker.cursor();
    let scroll_offset = if cursor >= max_rows {
        cursor - max_rows + 1
    } else {
        0
    };

    let lines: Vec<Line> = names
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_rows)
        .map(|(i, name)| {
            let is_selected = i == cursor;
            let style = if is_selected {
                theme.highlight
            } else {
                Style::new().fg(theme.dim_text)
            };
            let prefix = if is_selected { " \u{203A} " } else { "   " };
            Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{}/", name), style),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), chunks[2]);
}
