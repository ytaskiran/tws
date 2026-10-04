//! The sessions view: threads as quiet groups of sessions and agents.
//!
//! Each thread with live sessions opens a group. A faint band marks where the
//! group starts and a dim guide line shows how far it runs. Threads with no
//! sessions sit below the groups, without markers.
//!
//! The view owns its row order. `row_paths` returns the selectable rows in
//! screen order, and `render` draws exactly those rows, so the cursor and the
//! screen cannot disagree. Paths use the identifiers that
//! `AppState::resolve_selection` expects.

use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

use super::agent_meta;
use crate::core::model::{AgentSession, AgentStatus, Collection, Session, Thread};
use crate::core::state::AppState;
use crate::core::status::status_glyph;
use crate::core::workdir::shorten_home;
use crate::theme::Theme;

/// One visual row. `Gap` is spacing and is never selectable.
enum Row<'a> {
    Collection(Vec<String>, &'a Collection),
    Thread(Vec<String>, &'a Thread),
    IdleThread(Vec<String>, &'a Thread),
    Session(Vec<String>, &'a Session),
    Agent(Vec<String>, &'a AgentSession),
    Gap,
}

impl Row<'_> {
    fn path(&self) -> Option<&[String]> {
        match self {
            Row::Collection(p, _)
            | Row::Thread(p, _)
            | Row::IdleThread(p, _)
            | Row::Session(p, _)
            | Row::Agent(p, _) => Some(p),
            Row::Gap => None,
        }
    }
}

/// Push the rows for one list of threads: live groups first, then idle threads.
fn push_threads<'a>(
    state: &'a AppState,
    prefix: &[String],
    threads: &'a [Thread],
    out: &mut Vec<Row<'a>>,
) {
    let mut idle = Vec::new();
    for thread in threads {
        let mut tpath = prefix.to_vec();
        tpath.push(thread.id.to_string());
        let sessions: Vec<&Session> = state
            .active_sessions
            .iter()
            .filter(|s| s.thread_id == thread.id)
            .collect();
        if sessions.is_empty() {
            idle.push(Row::IdleThread(tpath, thread));
            continue;
        }
        out.push(Row::Thread(tpath.clone(), thread));
        for s in sessions {
            let mut spath = tpath.clone();
            spath.push(s.tmux_session_name.clone());
            out.push(Row::Session(spath.clone(), s));
            for a in state.agents_for_session(&s.tmux_session_name) {
                let mut apath = spath.clone();
                apath.push(a.pane_id.clone());
                out.push(Row::Agent(apath, a));
            }
        }
        out.push(Row::Gap);
    }
    out.extend(idle);
}

/// All rows in screen order. Named collections come first, as in the rest of
/// the app; root threads follow.
fn rows(state: &AppState) -> Vec<Row<'_>> {
    let mut out = Vec::new();
    for col in state.collections.iter().filter(|c| !c.is_root) {
        let prefix = vec![col.id.to_string()];
        out.push(Row::Collection(prefix.clone(), col));
        push_threads(state, &prefix, &col.threads, &mut out);
        // A live group already ends with a gap; do not stack a second one.
        if !matches!(out.last(), Some(Row::Gap)) {
            out.push(Row::Gap);
        }
    }
    for col in state.collections.iter().filter(|c| c.is_root) {
        push_threads(state, &[], &col.threads, &mut out);
    }
    out
}

/// Selectable rows in screen order.
pub fn row_paths(state: &AppState) -> Vec<Vec<String>> {
    rows(state)
        .iter()
        .filter_map(|r| r.path().map(<[String]>::to_vec))
        .collect()
}

/// Move `delta` rows from `current`, clamped to the list. With no current
/// selection, start at the first row.
pub fn step(paths: &[Vec<String>], current: &[String], delta: isize) -> Vec<String> {
    let Some(last) = paths.len().checked_sub(1) else {
        return Vec::new();
    };
    let next = match paths.iter().position(|p| p.as_slice() == current) {
        Some(i) => i.saturating_add_signed(delta).min(last),
        None => 0,
    };
    paths[next].clone()
}

fn status_style(status: AgentStatus, theme: &Theme) -> Style {
    match status {
        AgentStatus::Working => theme.status_working,
        AgentStatus::Waiting | AgentStatus::Review => theme.status_waiting,
        AgentStatus::Idle | AgentStatus::Unknown => theme.status_idle,
    }
}

fn visible_len(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.chars().count()).sum()
}

/// Draw `left` and `right` across `width`. `base` paints the whole row (the
/// band); a selected row takes the selection tint and the accent bar instead.
/// Both need explicit padding, because a `Line` style paints only its spans.
fn line(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
    selected: Option<Style>,
    base: Option<Style>,
    theme: &Theme,
) -> Line<'static> {
    left.insert(
        0,
        match selected {
            Some(_) => Span::styled("▎", theme.selection_bar),
            None => Span::raw(" "),
        },
    );
    let gap = width
        .saturating_sub(visible_len(&left) + visible_len(&right) + 2)
        .max(1);
    left.push(Span::raw(" ".repeat(gap)));
    left.extend(right);
    let used = visible_len(&left);
    if width > used {
        left.push(Span::raw(" ".repeat(width - used)));
    }
    let mut l = Line::from(left);
    if let Some(style) = selected.or(base) {
        l.style = style;
    }
    l
}

/// `focused` is false while the notes pane has focus; the selection then
/// uses the quieter unfocused tint.
pub fn render(
    frame: &mut Frame,
    state: &AppState,
    selected: &[String],
    focused: bool,
    area: Rect,
    theme: &Theme,
) {
    let width = area.width as usize;
    let now = agent_meta::now();
    let tint = if focused {
        theme.highlight
    } else {
        theme.highlight_unfocused
    };
    let guide = || Span::styled(" │ ", theme.guide);
    let dir = |t: &Thread, is_sel: bool| -> Option<Span<'static>> {
        let style = if is_sel { theme.path_dim } else { theme.meta };
        t.working_dir
            .as_ref()
            .map(|d| Span::styled(format!("  {}", shorten_home(d)), style))
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selected_line = 0;

    for row in rows(state) {
        let is_sel = row.path().is_some_and(|p| p == selected);
        if is_sel {
            selected_line = lines.len();
        }
        let sel = is_sel.then_some(tint);
        let name_style = |normal: Style| {
            if is_sel {
                theme.session_name_selected
            } else {
                normal
            }
        };
        let l = match row {
            Row::Gap => Line::from(""),
            Row::Collection(_, c) => line(
                vec![Span::styled(format!(" {}", c.name), theme.collection)],
                vec![],
                width,
                sel,
                None,
                theme,
            ),
            Row::Thread(_, t) => {
                // No status summary here: each agent row carries its own dot.
                let mut left = vec![Span::styled(format!(" {}", t.name), theme.thread_name)];
                left.extend(dir(t, is_sel));
                line(left, vec![], width, sel, Some(theme.band), theme)
            }
            Row::IdleThread(_, t) => {
                let mut left = vec![Span::styled(
                    format!(" {}", t.name),
                    name_style(theme.thread_idle),
                )];
                left.extend(dir(t, is_sel));
                line(left, vec![], width, sel, None, theme)
            }
            Row::Session(_, s) => line(
                vec![
                    guide(),
                    Span::styled(s.display_name.clone(), name_style(theme.session_name)),
                ],
                vec![],
                width,
                sel,
                None,
                theme,
            ),
            Row::Agent(_, a) => {
                // No pin digit here: in this view 1-5 open recent sessions,
                // so a digit would promise a key that does something else.
                let left = vec![
                    guide(),
                    Span::raw("  "),
                    Span::styled(
                        format!("{} ", status_glyph(a.status)),
                        status_style(a.status, theme),
                    ),
                    Span::styled(a.display_name.clone(), name_style(theme.agent_name)),
                ];
                let meta = agent_meta::label(a.agent_type, a.status_since, a.subagents, now);
                line(
                    left,
                    vec![Span::styled(meta, theme.meta)],
                    width,
                    sel,
                    None,
                    theme,
                )
            }
        };
        lines.push(l);
    }

    let scroll = super::scroll_to_keep_visible(selected_line, area.height);
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::AgentType;
    use uuid::Uuid;

    fn thread(n: u128, name: &str) -> Thread {
        Thread {
            id: Uuid::from_u128(n),
            name: name.into(),
            description: None,
            working_dir: None,
        }
    }

    fn session(name: &str, thread: u128) -> Session {
        Session {
            tmux_session_name: name.into(),
            display_name: name.into(),
            thread_id: Uuid::from_u128(thread),
            last_attached: 0,
        }
    }

    fn agent(sess: &str, pane: &str) -> AgentSession {
        AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: sess.into(),
            window_index: 0,
            pane_id: pane.into(),
            display_name: pane.into(),
            renamed: false,
            pin_slot: None,
            status: AgentStatus::Working,
            status_since: 0,
            subagents: 0,
        }
    }

    /// Root threads: `a` (idle), `b` (one session, one agent), `c` (one session).
    fn fixture() -> AppState {
        let mut state = AppState::new();
        let mut root = Collection::new_root();
        root.threads = vec![thread(1, "a"), thread(2, "b"), thread(3, "c")];
        state.collections = vec![root];
        state.active_sessions = vec![session("s_b", 2), session("s_c", 3)];
        state.agent_sessions = vec![agent("s_b", "%1")];
        state
    }

    fn id(n: u128) -> String {
        Uuid::from_u128(n).to_string()
    }

    fn p(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn row_paths_list_live_groups_first_and_idle_threads_last() {
        let (a, b, c) = (id(1), id(2), id(3));
        assert_eq!(
            row_paths(&fixture()),
            vec![
                p(&[&b]),
                p(&[&b, "s_b"]),
                p(&[&b, "s_b", "%1"]),
                p(&[&c]),
                p(&[&c, "s_c"]),
                p(&[&a]),
            ]
        );
    }

    #[test]
    fn named_collections_come_first_with_prefixed_paths() {
        let mut state = fixture();
        let mut col = Collection::new("work");
        col.threads = vec![thread(9, "w")];
        let col_id = col.id.to_string();
        state.collections.insert(0, col);

        let paths = row_paths(&state);
        assert_eq!(paths[0], p(&[&col_id]));
        assert_eq!(paths[1], p(&[&col_id, &id(9)]));
        assert_eq!(paths[2], p(&[&id(2)]));
    }

    /// The first screen row of the fixture is the band of thread `b`, whose
    /// agent is working. The dot belongs on the agent row only.
    #[test]
    fn thread_band_shows_no_agent_status_summary() {
        use crate::config::palette::Palette;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let state = fixture();
        let theme = Theme::build(&Palette::default());
        let mut terminal = Terminal::new(TestBackend::new(60, 8)).unwrap();
        terminal
            .draw(|f| render(f, &state, &[], true, f.area(), &theme))
            .unwrap();
        let buf = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..60).map(|x| buf[(x, y)].symbol()).collect() };

        assert!(row(0).contains('b'), "row 0 is not the band: {:?}", row(0));
        assert!(
            !row(0).contains('●'),
            "band repeats agent status: {:?}",
            row(0)
        );
        assert!(row(2).contains('●'), "agent row lost its dot: {:?}", row(2));
    }

    #[test]
    fn step_moves_and_clamps_at_both_ends() {
        let paths = row_paths(&fixture());
        let last = paths.last().unwrap();
        assert_eq!(step(&paths, &[], 1), paths[0]);
        assert_eq!(step(&paths, &paths[0], 1), paths[1]);
        assert_eq!(step(&paths, &paths[0], -1), paths[0]);
        assert_eq!(&step(&paths, last, 1), last);
    }

    #[test]
    fn step_on_empty_list_clears_selection() {
        assert!(step(&[], &p(&["x"]), 1).is_empty());
    }
}
