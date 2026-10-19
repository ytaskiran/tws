//! Agents as cards in triage lanes: a tinted surface with a status bar on the
//! left, the name and a status chip on the first line, and the path and a live
//! harness sigil under it. Structure comes from the surfaces, not from borders.
//! A card that starts to need you gets a sweep of light.

use std::collections::HashMap;

use ratatui::layout::Alignment;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use super::agent_meta;
use crate::core::model::{AgentStatus, AgentType};
use crate::core::state::FlatAgent;
use crate::theme::{Theme, darken_toward};

/// Rows of a card: a half pad, the title, the path, and a half pad. The two
/// half pads of neighbor cards make one blank row between them.
const CARD_HEIGHT: usize = 4;
pub const SWEEP_MS: u128 = 900;
const SWEEP_BAND: f32 = 16.0;

/// Live data the cards show beside the agent list.
pub struct Live<'a> {
    /// Sweep start time in ms by pane id.
    pub sweeps: &'a HashMap<String, u128>,
    pub now_ms: u128,
}

const LANE_NAMES: [&str; 3] = ["needs you", "working", "quiet"];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Heading(u8, usize),
    /// Agent index, row inside the card.
    Card(usize, usize),
    Gap,
}

/// The screen rows top to bottom. `render` and `agent_at` both use it, so the
/// draw and the hit test cannot disagree. `agents` comes sorted by lane.
fn rows(agents: &[FlatAgent]) -> Vec<Row> {
    let mut out = Vec::new();
    for (i, a) in agents.iter().enumerate() {
        let l = a.status.lane();
        if i == 0 || agents[i - 1].status.lane() != l {
            // Half pads leave only half a row above a heading; a gap keeps
            // the heading nearer its own lane.
            if i > 0 {
                out.push(Row::Gap);
            }
            let count = agents.iter().filter(|b| b.status.lane() == l).count();
            out.push(Row::Heading(l, count));
        }
        out.extend((0..CARD_HEIGHT).map(|k| Row::Card(i, k)));
    }
    out
}

fn status_style(status: AgentStatus, theme: &Theme) -> Style {
    match status {
        AgentStatus::Working => theme.status_working,
        AgentStatus::Waiting | AgentStatus::Review => theme.status_waiting,
        _ => theme.status_idle,
    }
}

/// The status chip, such as ` waiting `. Working, waiting, and review get a
/// filled chip; idle is plain text, so the quiet lane stays quiet.
fn chip(status: AgentStatus, theme: &Theme) -> Option<Span<'static>> {
    let (word, style) = match status {
        AgentStatus::Working => ("working", theme.chip_working),
        AgentStatus::Waiting => ("waiting", theme.chip_waiting),
        AgentStatus::Review => ("review", theme.chip_waiting),
        AgentStatus::Idle => ("idle", theme.status_idle),
        AgentStatus::Unknown => return None,
    };
    Some(Span::styled(format!(" {word} "), style))
}

/// The harness sigil: the harness icon, animated from the clock while the
/// agent works. Claude cycles its own spinner glyphs, the Codex cursor blinks,
/// and the other icons pulse bold, so a new harness needs no change here.
fn sigil(kind: AgentType, working: bool, now_ms: u128) -> (&'static str, bool) {
    const CLAUDE: [&str; 6] = ["·", "✢", "✳", "✶", "✻", "✽"];
    let tick = (now_ms / 250) as usize;
    if !working {
        return (kind.icon(), false);
    }
    match kind {
        AgentType::ClaudeCode => {
            // Up the glyphs and back down, as Claude Code does.
            let i = tick % 10;
            (CLAUDE[if i < 6 { i } else { 10 - i }], false)
        }
        AgentType::Codex => (if tick % 4 < 2 { ">_" } else { "> " }, false),
        _ => (kind.icon(), tick % 4 < 2),
    }
}

/// Cut `s` to at most `max` cells, and mark a cut with a trailing `…`.
fn fit(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    // Measure each prefix as a string: a char by char sum misses sequences
    // such as `❤️`, which is 2 cells wide but 1 + 0 by chars.
    let end = s
        .char_indices()
        .map(|(i, c)| i + c.len_utf8())
        .take_while(|&end| s[..end].width() < max)
        .last()
        .unwrap_or(0);
    match max {
        0 => String::new(),
        _ => format!("{}…", &s[..end]),
    }
}

fn width_of(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// One card row: `left` at the start, `right` two cells from the end, and the
/// card surface under the full width.
fn card_line(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    surface: Style,
    width: usize,
) -> Line<'static> {
    let used = width_of(&left) + width_of(&right);
    let margin = if right.is_empty() { 0 } else { 2 };
    left.push(Span::raw(" ".repeat(width.saturating_sub(used + margin))));
    left.extend(right);
    left.push(Span::raw(
        " ".repeat(margin.min(width.saturating_sub(used))),
    ));
    Line::from(left).style(surface)
}

/// The index of the agent drawn at screen cell (`x`, `y`) when `render` drew
/// into `area` with offset `scroll`. Gaps and lane headings belong to no agent.
pub fn agent_at(agents: &[FlatAgent], area: Rect, scroll: u16, x: u16, y: u16) -> Option<usize> {
    if !area.contains(Position { x, y }) {
        return None;
    }
    let line = usize::from(y - area.y) + usize::from(scroll);
    match rows(agents).get(line) {
        Some(&Row::Card(i, _)) => Some(i),
        _ => None,
    }
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
    live: &Live,
) -> u16 {
    if agents.is_empty() {
        let top = area.height.saturating_sub(1) / 2;
        let mut lines: Vec<Line> = vec![Line::from(""); top as usize];
        lines.push(Line::from(Span::styled("no agents", theme.thread_idle)));
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), area);
        return 0;
    }

    let width = area.width as usize;
    let now_ms = live.now_ms;
    let layout = rows(agents);
    let surface = |i: usize| {
        if i == cursor {
            theme.card_selected
        } else {
            theme.band
        }
    };

    let lines: Vec<Line<'static>> = layout
        .iter()
        .map(|row| match *row {
            Row::Gap => Line::from(""),
            Row::Heading(l, count) => {
                let style = if l == 0 {
                    theme.status_waiting.bold()
                } else {
                    theme.path_dim
                };
                Line::from(vec![
                    Span::raw(" "),
                    Span::styled(LANE_NAMES[usize::from(l)], style),
                    Span::styled(format!("  {count}"), theme.meta),
                ])
            }
            Row::Card(i, k) => {
                let a = &agents[i];
                let bar = || Span::styled("▌ ", status_style(a.status, theme));
                let indent = || Span::raw(" ");
                // A half pad: the surface fills the lower (or upper) half of
                // the cell, so the card edge sits mid-row.
                let half_pad = |glyph: &str| {
                    let fg = surface(i).bg.unwrap_or(Color::Reset);
                    Line::from(Span::styled(glyph.repeat(width), Style::default().fg(fg)))
                };
                match k {
                    0 => half_pad("▄"),
                    1 => {
                        let right: Vec<Span> = chip(a.status, theme).into_iter().collect();
                        let mut meta: Vec<Span> = a
                            .pin_slot
                            .map(|slot| Span::styled(format!("  {slot}"), theme.pin_digit))
                            .into_iter()
                            .collect();
                        if a.subagents > 0 {
                            let sub = Span::styled(
                                format!(
                                    "   {} {}",
                                    agent_meta::spinner(now_ms),
                                    agent_meta::subagents(a.subagents)
                                ),
                                theme.path_dim,
                            );
                            // The subagent count gives way before the name.
                            let left = width.saturating_sub(
                                6 + width_of(&right) + width_of(&meta) + sub.width(),
                            );
                            if left >= a.agent_display_name.width().min(12) {
                                meta.push(sub);
                            }
                        }
                        let room = width.saturating_sub(3 + width_of(&right) + 3 + width_of(&meta));
                        let mut title = vec![
                            bar(),
                            indent(),
                            Span::styled(
                                fit(&a.agent_display_name, room),
                                theme.agent_name_loud.bold(),
                            ),
                        ];
                        title.extend(meta);
                        card_line(title, right, surface(i), width)
                    }
                    2 => {
                        // The path gives way first, so the harness stays on screen.
                        let working = a.status == AgentStatus::Working;
                        let (glyph, bold) = sigil(a.agent_type, working, now_ms);
                        let mut style = if working {
                            theme.status_working
                        } else {
                            theme.path_dim
                        };
                        if bold {
                            style = style.bold();
                        }
                        let right = vec![
                            Span::styled(format!("{glyph} "), style),
                            Span::styled(agent_meta::kind_label(a.agent_type), theme.meta),
                        ];
                        let room = width.saturating_sub(3 + width_of(&right) + 3);
                        let path = fit(
                            &format!("{} / {}", a.thread_name, a.session_display_name),
                            room,
                        );
                        card_line(
                            vec![bar(), indent(), Span::styled(path, theme.path_dim)],
                            right,
                            surface(i),
                            width,
                        )
                    }
                    _ => half_pad("▀"),
                }
            }
        })
        .collect();

    let first = layout
        .iter()
        .position(|r| *r == Row::Card(cursor, 0))
        .unwrap_or(0);
    // A lane heading over the first card of its lane scrolls in with it,
    // when the pane has room for both.
    let first = match first.checked_sub(1).map(|j| layout[j]) {
        Some(Row::Heading(..)) if usize::from(area.height) > CARD_HEIGHT => first - 1,
        _ => first,
    };
    let last = layout
        .iter()
        .position(|r| *r == Row::Card(cursor, CARD_HEIGHT - 1))
        .unwrap_or(first);
    let scroll = super::sticky_scroll(prev_scroll, first, last, lines.len(), area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);

    sweep(
        frame,
        agents,
        &layout,
        scroll,
        area,
        theme,
        live.sweeps,
        now_ms,
    );
    scroll
}

/// A band of accent light that crosses each card whose sweep runs. It only
/// tints the cell backgrounds, so the text stays as drawn.
#[allow(clippy::too_many_arguments)]
fn sweep(
    frame: &mut Frame,
    agents: &[FlatAgent],
    layout: &[Row],
    scroll: u16,
    area: Rect,
    theme: &Theme,
    sweeps: &HashMap<String, u128>,
    now_ms: u128,
) {
    let Some(light) = theme.status_waiting.fg else {
        return;
    };
    let buf = frame.buffer_mut();
    for (line, row) in layout.iter().enumerate().skip(usize::from(scroll)) {
        let y = area.y as usize + line - usize::from(scroll);
        if y >= usize::from(area.bottom()) {
            break;
        }
        let Row::Card(i, k) = *row else { continue };
        let pad = k == 0 || k == CARD_HEIGHT - 1;
        let Some(&start) = sweeps.get(&agents[i].pane_id) else {
            continue;
        };
        let Some(t) = now_ms.checked_sub(start).filter(|&t| t < SWEEP_MS) else {
            continue;
        };
        let p = t as f32 / SWEEP_MS as f32;
        // Ease out, so the band slows as it leaves.
        let p = 1.0 - (1.0 - p).powi(2);
        let travel = f32::from(area.width) + 2.0 * SWEEP_BAND;
        let center = f32::from(area.x) - SWEEP_BAND + p * travel;
        for x in area.left()..area.right() {
            let d = (f32::from(x) - center).abs() / SWEEP_BAND;
            if d < 1.0 {
                let cell = &mut buf[(x, y as u16)];
                let strength = 0.32 * (1.0 - d * d);
                if pad {
                    cell.fg = darken_toward(cell.fg, light, strength);
                } else {
                    cell.bg = darken_toward(cell.bg, light, strength);
                }
            }
        }
    }
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

    /// No sweeps, at time 0.
    fn still() -> Live<'static> {
        static NONE: std::sync::LazyLock<HashMap<String, u128>> =
            std::sync::LazyLock::new(HashMap::new);
        Live {
            sweeps: &NONE,
            now_ms: 0,
        }
    }

    fn screen(agents: &[FlatAgent], cursor: usize, width: u16, height: u16) -> Vec<String> {
        let theme = Theme::build(&Palette::default());
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|f| {
                render(f, agents, cursor, 0, f.area(), &theme, &still());
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    /// A lane heading, then cards of 4 rows. A 7-row pane scrolled to the
    /// last card shows the end of card 2, then card 3. The half pads belong to
    /// their card, and the hit test must agree.
    #[test]
    fn agent_at_agrees_with_a_scrolled_render() {
        let agents: Vec<FlatAgent> = (0..4).map(agent).collect();
        let theme = Theme::build(&Palette::default());
        let mut terminal = Terminal::new(TestBackend::new(40, 7)).unwrap();
        let mut scroll = 0;
        terminal
            .draw(|f| scroll = render(f, &agents, 3, 0, f.area(), &theme, &still()))
            .unwrap();
        assert_eq!(scroll, 10, "card 3 ends on line 16, the last of 17");
        let area = Rect::new(0, 0, 40, 7);
        let row = |y: u16| -> String {
            (0..40)
                .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                .collect()
        };
        assert!(row(1).contains("t / s"), "{:?}", row(1));
        assert!(row(4).contains("agent-03"), "{:?}", row(4));
        assert_eq!(agent_at(&agents, area, scroll, 0, 0), Some(2));
        assert_eq!(agent_at(&agents, area, scroll, 0, 2), Some(2), "bottom pad");
        assert_eq!(agent_at(&agents, area, scroll, 0, 3), Some(3), "top pad");
        assert_eq!(agent_at(&agents, area, scroll, 39, 6), Some(3));
        assert_eq!(
            agent_at(&agents, area, scroll, 40, 0),
            None,
            "right of area"
        );
        assert_eq!(
            agent_at(&agents[..1], area, 0, 0, 5),
            None,
            "past the last card"
        );
    }

    #[test]
    fn card_shows_name_chip_path_and_harness() {
        let mut a = agent(0);
        a.status = AgentStatus::Waiting;
        a.pin_slot = Some(3);
        a.agent_type = AgentType::Codex;
        let out = screen(&[a], 0, 40, 5);
        assert!(out[0].contains("needs you  1"), "lane: {out:?}");
        assert_eq!(out[1], "▄".repeat(40), "top pad: {out:?}");
        assert_eq!(out[4], "▀".repeat(40), "bottom pad: {out:?}");
        assert!(out[2].contains("agent-00  3"), "name and pin: {out:?}");
        assert!(out[2].trim_end().ends_with("waiting"), "chip: {out:?}");
        assert!(out[3].contains("t / s"), "path: {out:?}");
        assert!(out[3].trim_end().ends_with(">_ codex"), "harness: {out:?}");
    }

    #[test]
    fn unknown_status_has_no_chip() {
        let mut a = agent(0);
        a.status = AgentStatus::Unknown;
        let out = screen(&[a], 0, 40, 5);
        assert!(out[2].trim_end().ends_with("agent-00"), "{out:?}");
    }

    #[test]
    fn long_path_is_cut_so_the_harness_stays_visible() {
        let mut a = agent(0);
        a.thread_name = "project-operations-analytics".into();
        a.session_display_name = "cortex-pr-123-review".into();
        let out = screen(&[a], 0, 40, 5);
        assert!(out[3].contains('…'), "path not marked as cut: {out:?}");
        assert!(
            out[3].trim_end().ends_with("claude"),
            "meta pushed off: {out:?}"
        );
    }

    #[test]
    fn subagents_show_on_the_title_row() {
        let mut a = agent(0);
        a.subagents = 3;
        let out = screen(&[a, agent(1)], 1, 40, 10);
        assert!(out[2].contains("agent-00"), "{out:?}");
        assert!(out[2].contains("3 subagents"), "{out:?}");
        assert!(out[2].trim_end().ends_with("working"), "chip: {out:?}");
        let one = {
            let mut a = agent(0);
            a.subagents = 1;
            screen(&[a], 0, 40, 5)
        };
        assert!(one[2].contains("1 subagent "), "{one:?}");
        assert!(out[6].contains("agent-01"), "next card: {out:?}");
    }

    #[test]
    fn selected_card_stays_visible_past_the_bottom_edge() {
        let agents: Vec<FlatAgent> = (0..10).map(agent).collect();
        let out = screen(&agents, 9, 40, 4);
        assert!(out[1].contains("agent-09"), "{out:?}");
    }

    #[test]
    fn one_row_pane_shows_the_top_of_the_selected_card() {
        let agents: Vec<FlatAgent> = (0..3).map(agent).collect();
        let out = screen(&agents, 2, 40, 1);
        assert!(out[0].starts_with('▄'), "{out:?}");
        assert_eq!(agent_at(&agents, Rect::new(0, 0, 40, 1), 9, 0, 0), Some(2));
    }

    #[test]
    fn list_starts_at_the_top_when_the_cursor_fits() {
        let agents: Vec<FlatAgent> = (0..10).map(agent).collect();
        let out = screen(&agents, 1, 40, 11);
        assert!(
            out[2].contains("agent-00"),
            "list scrolled too early: {out:?}"
        );
    }

    /// Each lane gets a heading; the hit test skips the headings.
    #[test]
    fn lanes_add_headings_the_hit_test_skips() {
        let mut agents: Vec<FlatAgent> = (0..3).map(agent).collect();
        agents[0].status = AgentStatus::Waiting;
        agents[2].status = AgentStatus::Idle;
        let layout = rows(&agents);
        assert_eq!(layout[0], Row::Heading(0, 1));
        assert_eq!(layout[1], Row::Card(0, 0));
        assert_eq!(layout[6], Row::Heading(1, 1));
        let area = Rect::new(0, 0, 40, 40);
        assert_eq!(agent_at(&agents, area, 0, 0, 0), None, "heading");
        assert_eq!(agent_at(&agents, area, 0, 0, 1), Some(0));
        assert_eq!(agent_at(&agents, area, 0, 0, 7), Some(1));
    }

    #[test]
    fn fit_counts_cells_not_chars() {
        assert_eq!(fit("日本語テスト", 7), "日本語…");
        assert_eq!(fit("日本語", 6), "日本語");
        assert_eq!(fit("abc", 0), "");
        let hearts = "❤\u{FE0F}".repeat(4);
        for max in 1..8 {
            assert!(fit(&hearts, max).width() <= max, "max {max}");
        }
    }

    #[test]
    fn wide_name_keeps_the_chip_on_the_card() {
        let mut a = agent(0);
        a.status = AgentStatus::Waiting;
        a.agent_display_name = "日本語のエージェント名前テスト長い名前です".into();
        a.thread_name = "スレッド名前がとても長いです".into();
        let out = screen(&[a], 0, 40, 5);
        assert!(out[2].trim_end().ends_with("waiting"), "{out:?}");
        assert!(out[3].trim_end().ends_with("claude"), "{out:?}");
    }

    /// In a narrow card the subagent count goes before the name does.
    #[test]
    fn subagents_give_way_to_the_name() {
        let mut a = agent(0);
        a.pin_slot = Some(1);
        a.subagents = 3;
        let out = screen(&[a], 0, 40, 5);
        assert!(out[2].contains("agent-00  1"), "{out:?}");
        assert!(!out[2].contains("subagents"), "{out:?}");
        assert!(out[2].trim_end().ends_with("working"), "{out:?}");
    }

    /// A pane too short for a heading and a card shows the card.
    #[test]
    fn one_row_pane_shows_the_card_not_its_lane_heading() {
        let mut agents: Vec<FlatAgent> = (0..3).map(agent).collect();
        agents[0].status = AgentStatus::Waiting;
        let theme = Theme::build(&Palette::default());
        let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let mut scroll = 0;
        terminal
            .draw(|f| scroll = render(f, &agents, 1, 0, f.area(), &theme, &still()))
            .unwrap();
        let area = Rect::new(0, 0, 40, 1);
        assert_eq!(agent_at(&agents, area, scroll, 0, 0), Some(1));
    }

    #[test]
    fn claude_sigil_goes_up_the_glyphs_and_back_down() {
        let frames: String = (0..10)
            .map(|t| sigil(AgentType::ClaudeCode, true, t * 250).0)
            .collect();
        assert_eq!(frames, "·✢✳✶✻✽✻✶✳✢");
        assert_eq!(sigil(AgentType::ClaudeCode, false, 999), ("✳", false));
    }
}
