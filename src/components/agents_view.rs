use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};

use crate::core::model::AgentStatus;
use crate::core::state::FlatAgent;
use crate::core::status::status_glyph;
use crate::theme::Theme;

fn status_style(status: AgentStatus, theme: &Theme) -> Style {
    match status {
        AgentStatus::Working => theme.status_working,
        AgentStatus::Waiting => theme.status_waiting,
        AgentStatus::Idle => theme.status_idle,
        AgentStatus::Unknown => theme.status_idle,
    }
}

pub fn render(frame: &mut Frame, agents: &[FlatAgent], cursor: usize, area: Rect, theme: &Theme) {
    if agents.is_empty() {
        let available_height = area.height.saturating_sub(2);
        let top_padding = available_height.saturating_sub(1) / 2;
        let mut lines: Vec<Line> = vec![Line::from(""); top_padding as usize];
        lines.push(Line::from(Span::styled(
            "No active agents",
            theme.thread_dim,
        )));
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
        return;
    }

    let (list_area, legend_area) = {
        let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
        (chunks[0], chunks[1])
    };

    // The flat list is already sorted: pinned first, then unpinned.
    // Find the split point: index of the first unpinned agent.
    let split = agents
        .iter()
        .position(|a| a.pin_slot.is_none())
        .unwrap_or(agents.len());
    let has_pinned = split > 0;
    let has_unpinned = split < agents.len();

    let agent_to_item = |a: &FlatAgent| -> ListItem<'static> {
        let badge: Span = match a.pin_slot {
            Some(slot) => Span::styled(format!("[{}] ", slot), theme.pin_badge),
            None => Span::raw("    "),
        };
        let line = Line::from(vec![
            Span::styled(
                format!("{} ", status_glyph(a.status)),
                status_style(a.status, theme),
            ),
            badge,
            Span::styled(a.thread_name.clone(), theme.thread),
            Span::styled(" : ", theme.badge_dot),
            Span::styled(a.session_display_name.clone(), theme.session),
            Span::styled(" : ", theme.badge_dot),
            Span::styled(
                a.agent_type.icon().to_string(),
                theme.agent.add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {}", a.agent_display_name), theme.agent),
        ]);
        ListItem::new(line)
    };

    let mut items: Vec<ListItem<'static>> = Vec::with_capacity(agents.len() + 1);
    let mut adjusted_cursor = cursor;

    for (idx, agent) in agents.iter().enumerate() {
        if idx == split && has_pinned && has_unpinned {
            // Insert separator row between pinned and unpinned blocks
            let sep_width = list_area.width.saturating_sub(2) as usize;
            let sep = "─".repeat(sep_width);
            items.push(ListItem::new(Line::from(Span::styled(
                sep,
                theme.separator,
            ))));
            if cursor >= split {
                adjusted_cursor = cursor + 1;
            }
        }
        items.push(agent_to_item(agent));
    }

    let mut list_state = ListState::default();
    list_state.select(Some(adjusted_cursor));

    let list = List::new(items).highlight_style(theme.highlight);
    frame.render_stateful_widget(list, list_area, &mut list_state);

    let legend = Line::from(vec![
        Span::styled("● ", theme.status_working),
        Span::styled("working   ", theme.statusbar_desc),
        Span::styled("◐ ", theme.status_waiting),
        Span::styled("waiting   ", theme.statusbar_desc),
        Span::styled("○ ", theme.status_idle),
        Span::styled("idle", theme.statusbar_desc),
    ]);
    frame.render_widget(
        Paragraph::new(legend).alignment(Alignment::Center),
        legend_area,
    );
}
