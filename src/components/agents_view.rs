//! Prototype v2: agents as status-first lines. The dot and the agent name are
//! the loud part; the workspace path trails dim. Pin slots sit in a quiet gutter.

use ratatui::layout::Alignment;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use crate::core::model::AgentStatus;
use crate::core::state::FlatAgent;
use crate::core::status::status_glyph;
use crate::theme::Theme;

fn status_style(status: AgentStatus, theme: &Theme) -> Style {
    match status {
        AgentStatus::Working => theme.status_working,
        AgentStatus::Waiting | AgentStatus::Review => theme.status_waiting,
        _ => theme.status_idle,
    }
}

pub fn render(frame: &mut Frame, agents: &[FlatAgent], cursor: usize, area: Rect, theme: &Theme) {
    if agents.is_empty() {
        let top = area.height.saturating_sub(1) / 2;
        let mut lines: Vec<Line> = vec![Line::from(""); top as usize];
        lines.push(Line::from(Span::styled("no agents", theme.thread_idle)));
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
        return;
    }

    let width = area.width as usize;
    let name_width = agents
        .iter()
        .map(|a| a.agent_display_name.chars().count())
        .max()
        .unwrap_or(0);

    let lines: Vec<Line<'static>> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let selected = i == cursor;
            let bar = if selected {
                Span::styled("▎", theme.selection_bar)
            } else {
                Span::raw(" ")
            };
            let pin = match a.pin_slot {
                Some(slot) => Span::styled(format!(" {}  ", slot), theme.pin_digit),
                None => Span::raw("    "),
            };
            let pad = " ".repeat(name_width - a.agent_display_name.chars().count() + 6);
            let mut spans = vec![
                bar,
                pin,
                Span::styled(
                    format!("{} ", status_glyph(a.status)),
                    status_style(a.status, theme),
                ),
                Span::styled(
                    a.agent_display_name.clone(),
                    if selected {
                        theme.session_name_selected
                    } else {
                        theme.agent_name_loud
                    },
                ),
                Span::raw(pad),
                Span::styled(
                    format!("{} / {}", a.thread_name, a.session_display_name),
                    theme.path_dim,
                ),
            ];
            if selected {
                let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
                if width > used {
                    spans.push(Span::raw(" ".repeat(width - used)));
                }
                let mut line = Line::from(spans);
                line.style = theme.highlight;
                line
            } else {
                Line::from(spans)
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}
