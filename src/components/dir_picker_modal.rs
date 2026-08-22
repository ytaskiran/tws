use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use crate::components::centered_rect;
use crate::core::workdir::{DirPicker, shorten_home};
use crate::theme::Theme;

const MAX_VISIBLE: usize = 10;

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

    let cursor = picker.cursor();
    let on_path = picker.is_current_row();

    let row_style = |selected: bool| {
        if selected {
            theme.highlight
        } else {
            Style::new().fg(theme.dim_text)
        }
    };
    let prefix = |selected: bool| if selected { " \u{203A} " } else { "   " };

    // The path line is row 0 of a single navigable column: it carries the same
    // prefix column and highlight as the entries below, so the marker moves
    // between them without any text shifting.
    let mut base = shorten_home(picker.current());
    if !base.ends_with('/') {
        base.push('/');
    }
    let mut path_spans = vec![Span::styled(prefix(on_path), row_style(on_path))];
    if on_path {
        path_spans.push(Span::styled(base, theme.highlight));
        // Dimmed because the query filters the list below; Enter takes the
        // directory, not the typed text, so it must not read as a path segment.
        path_spans.push(Span::styled(
            picker.query(),
            theme.highlight.add_modifier(Modifier::DIM),
        ));
        path_spans.push(Span::styled("\u{2588}", theme.cursor));
    } else {
        path_spans.push(Span::styled(base, theme.modal_muted));
        path_spans.push(Span::styled(picker.query(), row_style(false)));
    }
    frame.render_widget(Paragraph::new(Line::from(path_spans)), chunks[0]);

    let sep = "\u{2500}".repeat(chunks[1].width as usize);
    frame.render_widget(
        Paragraph::new(Line::styled(sep, theme.separator)),
        chunks[1],
    );

    if names.is_empty() {
        let label = if picker.query().is_empty() {
            "   No subdirectories"
        } else {
            "   No matches"
        };
        let empty = Line::from(Span::styled(label, theme.modal_muted));
        frame.render_widget(Paragraph::new(empty), chunks[2]);
        return;
    }

    // Cursor 0 is the path line, so entry `i` is selected at cursor `i + 1`.
    let max_rows = chunks[2].height as usize;
    let entry_row = cursor.saturating_sub(1);
    let scroll_offset = if entry_row >= max_rows {
        entry_row - max_rows + 1
    } else {
        0
    };

    let rows: Vec<Line> = names
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(max_rows)
        .map(|(i, name)| {
            let selected = i + 1 == cursor;
            let style = row_style(selected);
            Line::from(vec![
                Span::styled(prefix(selected), style),
                Span::styled(format!("{}/", name), style),
            ])
        })
        .collect();

    frame.render_widget(Paragraph::new(rows), chunks[2]);
}
