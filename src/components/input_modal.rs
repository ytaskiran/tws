use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

pub fn render(frame: &mut Frame, title: &str, input: &str, area: Rect) {
    let popup = centered_rect(50, 5, area);
    frame.render_widget(Clear, popup);

    let display_text = Line::from(vec![
        Span::raw(input),
        Span::styled("█", Style::new().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK)),
    ]);

    let block = Block::bordered()
        .title(format!(" {} ", title))
        .title_style(Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .border_style(Style::new().fg(Color::Cyan));

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
