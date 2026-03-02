use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, message: &str, area: Rect) {
    let popup = centered_rect(50, 5, area);
    frame.render_widget(Clear, popup);

    let text = vec![
        Line::from(message),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::raw("es / "),
            Span::styled("n", Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("o"),
        ]),
    ];

    let block = Block::bordered()
        .title(" Confirm ")
        .title_style(Style::new().fg(Color::Red).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(Color::Red));

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup);
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(area);
    let [area] = horizontal.areas(area);
    area
}
