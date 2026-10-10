//! Prototype v2: agents as status-first lines. The dot and the agent name are
//! the loud part; the workspace path trails dim. Pin slots sit in a quiet gutter.

use ratatui::layout::Alignment;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::agent_meta;
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

/// Cut `s` to at most `max` chars, and mark a cut with a trailing `…`.
fn fit(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    match max {
        0 => String::new(),
        _ => s.chars().take(max - 1).chain(['…']).collect(),
    }
}

/// The index of the agent drawn at screen cell (`x`, `y`) when `render` drew
/// into `area` with offset `scroll`. An agent owns its subagent line too.
pub fn agent_at(agents: &[FlatAgent], area: Rect, scroll: u16, x: u16, y: u16) -> Option<usize> {
    if !area.contains(Position { x, y }) {
        return None;
    }
    let line = usize::from(y - area.y) + usize::from(scroll);
    agents
        .iter()
        .enumerate()
        .flat_map(|(i, a)| std::iter::repeat_n(i, 1 + usize::from(a.subagents > 0)))
        .nth(line)
}

/// `prev_scroll` is the offset of the last frame; the return value is the
/// offset of this one.
pub fn render(
    frame: &mut Frame,
    agents: &[FlatAgent],
    cursor: usize,
    prev_scroll: u16,
    area: Rect,
    theme: &Theme,
) -> u16 {
    if agents.is_empty() {
        let top = area.height.saturating_sub(1) / 2;
        let mut lines: Vec<Line> = vec![Line::from(""); top as usize];
        lines.push(Line::from(Span::styled("no agents", theme.thread_idle)));
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
        return 0;
    }

    let width = area.width as usize;
    let name_width = agents
        .iter()
        .map(|a| a.agent_display_name.chars().count())
        .max()
        .unwrap_or(0);

    let now_ms = agent_meta::now_ms();
    let mut lines: Vec<Line<'static>> = Vec::new();
    // The first and last lines of the selected agent, so the scroll can keep
    // its subagent line on screen without hiding the agent line.
    let (mut cursor_first, mut cursor_last) = (0, 0);
    for (i, a) in agents.iter().enumerate() {
        let selected = i == cursor;
        if selected {
            cursor_first = lines.len();
        }
        lines.push({
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
                Span::styled(a.agent_display_name.clone(), theme.agent_name_loud),
                Span::raw(pad),
            ];
            // The harness name at the right edge, as in the sessions view. The path
            // gives way first, so the meta stays on screen in a narrow pane.
            let meta = agent_meta::kind_label(a.agent_type).to_string();
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let room = width.saturating_sub(used + meta.chars().count() + 3);
            spans.push(Span::styled(
                fit(
                    &format!("{} / {}", a.thread_name, a.session_display_name),
                    room,
                ),
                theme.path_dim,
            ));
            let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
            let gap = width.saturating_sub(used + meta.chars().count() + 2).max(1);
            spans.push(Span::raw(" ".repeat(gap)));
            spans.push(Span::styled(meta, theme.meta));
            item_line(spans, selected, width, theme)
        });
        // The subagent line is part of the agent: same bar, same highlight, and
        // the cursor does not stop on it. The spinner sits under the name.
        if a.subagents > 0 {
            let bar = if selected {
                Span::styled("▎", theme.selection_bar)
            } else {
                Span::raw(" ")
            };
            let text = format!(
                "{} {} working",
                agent_meta::spinner(now_ms),
                agent_meta::subagents(a.subagents)
            );
            let spans = vec![bar, Span::raw("      "), Span::styled(text, theme.path_dim)];
            lines.push(item_line(spans, selected, width, theme));
        }
        if selected {
            cursor_last = lines.len() - 1;
        }
    }

    let scroll = super::sticky_scroll(
        prev_scroll,
        cursor_first,
        cursor_last,
        lines.len(),
        area.height,
    );
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
    scroll
}

/// A selected line is padded to the full width, so its tint covers the row.
fn item_line(
    mut spans: Vec<Span<'static>>,
    selected: bool,
    width: usize,
    theme: &Theme,
) -> Line<'static> {
    if !selected {
        return Line::from(spans);
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if width > used {
        spans.push(Span::raw(" ".repeat(width - used)));
    }
    let mut line = Line::from(spans);
    line.style = theme.highlight;
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::palette::Palette;
    use crate::core::model::AgentType;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn agent(n: usize) -> FlatAgent {
        FlatAgent {
            thread_idx: 0,
            thread_name: "t".into(),
            sess_idx: 0,
            session_display_name: "s".into(),
            agent_idx: n,
            agent_display_name: format!("agent-{n:02}"),
            tmux_session_name: "s".into(),
            window_index: 0,
            pane_id: format!("%{n}"),
            pin_slot: None,
            status: AgentStatus::Working,
            agent_type: AgentType::ClaudeCode,
            subagents: 0,
        }
    }

    /// Agent 0 has a subagent line, so the list has 6 lines for 5 agents. On
    /// a scrolled frame, the hit test must name the agent drawn at each line.
    #[test]
    fn agent_at_agrees_with_a_scrolled_render() {
        let mut agents: Vec<FlatAgent> = (0..5).map(agent).collect();
        agents[0].subagents = 2;
        let theme = Theme::build(&Palette::default());
        let mut terminal = Terminal::new(TestBackend::new(40, 3)).unwrap();
        let mut scroll = 0;
        terminal
            .draw(|f| scroll = render(f, &agents, 4, 0, f.area(), &theme))
            .unwrap();
        assert_eq!(scroll, 3, "6 lines, 3 visible, the last agent selected");
        let area = Rect::new(0, 0, 40, 3);
        let top: String = (0..40)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
            .collect();
        assert!(top.contains("agent-02"), "top line is not agent 2: {top:?}");
        assert_eq!(agent_at(&agents, area, scroll, 0, 0), Some(2));
        assert_eq!(agent_at(&agents, area, scroll, 39, 2), Some(4));
        assert_eq!(agent_at(&agents, area, 0, 0, 1), Some(0), "subagent line");
        assert_eq!(agent_at(&agents, area, 0, 0, 2), Some(1));
        assert_eq!(
            agent_at(&agents, area, scroll, 0, 3),
            None,
            "below the area"
        );
        assert_eq!(
            agent_at(&agents, area, scroll, 40, 0),
            None,
            "right of the area"
        );
    }

    fn screen(agents: &[FlatAgent], cursor: usize, height: u16) -> String {
        let theme = Theme::build(&Palette::default());
        let mut terminal = Terminal::new(TestBackend::new(40, height)).unwrap();
        terminal
            .draw(|f| {
                render(f, agents, cursor, 0, f.area(), &theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn rows_show_the_harness_without_an_age() {
        let a = agent(0);
        let mut b = agent(1);
        b.agent_type = AgentType::Codex;
        let out = screen(&[a, b], 0, 3);
        assert!(out.contains("claude"), "missing kind:\n{out}");
        assert!(out.contains("codex"), "missing kind:\n{out}");
        assert!(!out.contains(" · "), "age shown:\n{out}");
    }

    #[test]
    fn long_path_is_cut_so_the_meta_stays_visible() {
        let mut a = agent(0);
        a.thread_name = "project-operations-analytics".into();
        a.session_display_name = "cortex-pr-123-review".into();
        let out = screen(&[a], 0, 1);
        assert!(out.contains('…'), "path not marked as cut:\n{out}");
        assert!(
            out.trim_end().ends_with("claude"),
            "meta pushed off:\n{out}"
        );
    }

    #[test]
    fn selected_agent_stays_visible_past_the_bottom_edge() {
        let agents: Vec<FlatAgent> = (0..10).map(agent).collect();
        let out = screen(&agents, 9, 3);
        assert!(out.contains("agent-09"), "cursor row scrolled off:\n{out}");
    }

    #[test]
    fn subagents_get_their_own_line_under_the_agent() {
        let mut a = agent(0);
        a.subagents = 3;
        let out = screen(&[a, agent(1)], 1, 3);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("agent-00"), "agent line:\n{out}");
        assert!(
            lines[1].contains("3 subagents working"),
            "subagent line:\n{out}"
        );
        // The spinner starts the text, in the column of the agent name.
        let name_col = lines[0].chars().position(|c| c == 'a').unwrap();
        let spinner = lines[1].chars().nth(name_col).unwrap();
        assert!(
            ('\u{2800}'..='\u{28FF}').contains(&spinner),
            "no spinner under the name:\n{out}"
        );
        assert!(lines[2].contains("agent-01"), "next agent:\n{out}");
        assert!(
            !lines[0].contains("subagent"),
            "count on the agent line:\n{out}"
        );
    }

    #[test]
    fn one_subagent_reads_singular() {
        let mut a = agent(0);
        a.subagents = 1;
        let out = screen(&[a], 0, 2);
        assert!(out.contains("1 subagent working"), "singular:\n{out}");
    }

    #[test]
    fn subagent_line_of_the_selected_agent_stays_visible() {
        let mut agents: Vec<FlatAgent> = (0..10).map(agent).collect();
        agents[9].subagents = 2;
        let out = screen(&agents, 9, 3);
        assert!(out.contains("agent-09"), "cursor row scrolled off:\n{out}");
        assert!(
            out.contains("2 subagents working"),
            "its line scrolled off:\n{out}"
        );
    }

    #[test]
    fn one_row_pane_shows_the_selected_agent_not_its_subagent_line() {
        let mut agents: Vec<FlatAgent> = (0..3).map(agent).collect();
        agents[2].subagents = 2;
        let out = screen(&agents, 2, 1);
        assert!(out.contains("agent-02"), "agent line hidden:\n{out}");
    }

    #[test]
    fn subagent_lines_above_push_the_cursor_down() {
        let mut agents: Vec<FlatAgent> = (0..4).map(agent).collect();
        for a in &mut agents[..3] {
            a.subagents = 1;
        }
        // Agent 3 sits on line 6, below a 3-line screen.
        let out = screen(&agents, 3, 3);
        assert!(out.contains("agent-03"), "cursor row scrolled off:\n{out}");
    }

    #[test]
    fn list_starts_at_the_top_when_the_cursor_fits() {
        let agents: Vec<FlatAgent> = (0..10).map(agent).collect();
        let out = screen(&agents, 1, 3);
        assert!(out.contains("agent-00"), "list scrolled too early:\n{out}");
    }
}
