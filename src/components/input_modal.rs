use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};
use ratatui::Frame;

use crate::theme;

pub fn render(frame: &mut Frame, title: &str, input: &str, area: Rect) {
    let popup = centered_rect(50, 7, area);
    frame.render_widget(Clear, popup);

    let display_text = Line::from(vec![
        Span::raw(input),
        Span::styled(
            "\u{2588}",
            Style::new()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .title(format!(" {} ", title))
        .title_style(theme::MODAL_TITLE_STYLE)
        .border_style(theme::MODAL_BORDER_STYLE)
        .padding(Padding::new(1, 1, 1, 0));

    let paragraph = Paragraph::new(display_text).block(block);
    frame.render_widget(paragraph, popup);
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
