use ratatui::layout::Alignment;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::core::state::FlatAgent;
use crate::theme;

pub fn render(frame: &mut Frame, agents: &[FlatAgent], cursor: usize, area: Rect) {
    if agents.is_empty() {
        let available_height = area.height.saturating_sub(2);
        let top_padding = available_height.saturating_sub(1) / 2;
        let mut lines: Vec<Line> = vec![Line::from(""); top_padding as usize];
        lines.push(Line::from(Span::styled("No active agents", theme::THREAD_DIM_STYLE)));
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = agents
        .iter()
        .map(|a| {
            let line = Line::from(vec![
                Span::raw("  "),
                Span::styled(a.thread_name.as_str(), theme::THREAD_STYLE),
                Span::styled(" : ", theme::BADGE_DOT_STYLE),
                Span::styled(a.session_display_name.as_str(), theme::SESSION_STYLE),
                Span::styled(" : ", theme::BADGE_DOT_STYLE),
                Span::styled(a.agent_type.icon(), theme::AGENT_STYLE.add_modifier(Modifier::BOLD)),
                Span::styled(format!(" {}", a.agent_display_name), theme::AGENT_STYLE),
            ]);
            ListItem::new(line)
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(cursor));

    let list = List::new(items).highlight_style(theme::HIGHLIGHT_STYLE);
    frame.render_stateful_widget(list, area, &mut list_state);
}
