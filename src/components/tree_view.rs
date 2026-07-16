use ratatui::style::Modifier;
use ratatui::text::{Line, Span, Text};
use tui_tree_widget::TreeItem;

use crate::core::model::Thread;
use crate::core::state::AppState;
use crate::theme::Theme;

/// Converts the app state into TreeItems for rendering.
/// Collections -> Threads -> Sessions (3-level hierarchy).
/// Root threads (from the root collection) render at root level, not nested under a collection node.
pub fn build_tree_items<'a>(state: &'a AppState, theme: &Theme) -> Vec<TreeItem<'a, String>> {
    let mut items: Vec<TreeItem<'a, String>> = Vec::new();

    // Regular collections first
    for col in &state.collections {
        if col.is_root {
            continue;
        }
        let children: Vec<TreeItem<'a, String>> = col
            .threads
            .iter()
            .map(|thread| build_thread_item(state, thread, theme))
            .collect();

        items.push(
            TreeItem::new(
                col.id.to_string(),
                Text::styled(col.name.as_str(), theme.collection),
                children,
            )
            .expect("thread IDs are unique within a collection"),
        );
    }

    // Root threads at the bottom, rendered as root-level items
    for col in &state.collections {
        if !col.is_root {
            continue;
        }
        for thread in &col.threads {
            items.push(build_thread_item(state, thread, theme));
        }
    }

    items
}

/// Build a TreeItem for a single thread (shared between regular and root threads).
fn build_thread_item<'a>(
    state: &'a AppState,
    thread: &'a Thread,
    theme: &Theme,
) -> TreeItem<'a, String> {
    let session_children: Vec<TreeItem<'a, String>> = state
        .active_sessions
        .iter()
        .filter(|s| s.thread_id == thread.id)
        .map(|s| {
            let agents = state.agents_for_session(&s.tmux_session_name);
            if agents.is_empty() {
                TreeItem::new_leaf(
                    s.tmux_session_name.clone(),
                    Text::styled(&s.display_name, theme.session),
                )
            } else {
                let agent_children: Vec<TreeItem<'a, String>> = agents
                    .iter()
                    .map(|a| {
                        let label = Line::from(vec![
                            Span::styled("╰─ ", theme.agent_connector),
                            Span::styled(
                                a.agent_type.icon(),
                                theme.agent.add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(" \u{25CF} ", theme.agent_status_style(a.status)),
                            // Padded to the longest label so names stay column-aligned.
                            Span::styled(
                                format!("{:<7}  ", a.status.label()),
                                theme.agent_status_style(a.status),
                            ),
                            Span::styled(a.display_name.as_str(), theme.agent),
                        ]);
                        TreeItem::new_leaf(a.pane_id.clone(), label)
                    })
                    .collect();
                TreeItem::new(
                    s.tmux_session_name.clone(),
                    Text::styled(&s.display_name, theme.session),
                    agent_children,
                )
                .expect("pane IDs are unique within a session")
            }
        })
        .collect();

    let session_count = session_children.len();

    let thread_text = if session_count > 0 {
        Text::from(Line::from(vec![
            Span::styled(thread.name.as_str(), theme.thread),
            Span::styled(" \u{25CF} ", theme.badge_dot),
            Span::styled(session_count.to_string(), theme.badge_count),
        ]))
    } else {
        Text::styled(thread.name.as_str(), theme.thread_dim)
    };

    if session_children.is_empty() {
        TreeItem::new_leaf(thread.id.to_string(), thread_text)
    } else {
        TreeItem::new(thread.id.to_string(), thread_text, session_children)
            .expect("session names are unique within a thread")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::palette::Palette;
    use crate::core::model::{AgentSession, AgentStatus, AgentType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use tui_tree_widget::{Tree, TreeState};

    const SESSION: &str = "twsr_general_s1";

    fn agent(pane_id: &str, name: &str, status: AgentStatus) -> AgentSession {
        AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: SESSION.into(),
            window_index: 0,
            pane_id: pane_id.into(),
            display_name: name.into(),
            renamed: false,
            pin_slot: None,
            status,
        }
    }

    /// Render one session holding one agent per status, fully expanded, and
    /// return the resulting buffer.
    fn render_all_statuses(theme: &Theme) -> ratatui::buffer::Buffer {
        let mut state = AppState::new();
        state.ensure_general_thread();
        state.refresh_sessions(&[(SESSION.to_string(), 0)]);
        // Names deliberately share no substring with any status label, so a
        // search for "idle" can only ever match the label itself.
        state.agent_sessions = vec![
            agent("%1", "alpha", AgentStatus::Running),
            agent("%2", "bravo", AgentStatus::Waiting),
            agent("%3", "charlie", AgentStatus::Idle),
        ];

        let thread_id = state.collections[0].threads[0].id.to_string();
        let items = build_tree_items(&state, theme);

        let mut tree_state = TreeState::default();
        tree_state.open(vec![thread_id.clone()]);
        tree_state.open(vec![thread_id, SESSION.to_string()]);

        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal
            .draw(|frame| {
                let tree = Tree::new(&items).unwrap();
                frame.render_stateful_widget(tree, frame.area(), &mut tree_state);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Column/row of the first cell where `needle` starts.
    ///
    /// Matches cell-by-cell rather than searching a joined string: the tree is
    /// full of multi-byte glyphs (`╰`, `─`, `✳`, `●`), so a byte offset from
    /// `str::find` is not a column index and would silently read the wrong cell.
    fn cell_of(buf: &ratatui::buffer::Buffer, needle: &str) -> (u16, u16) {
        let width = buf.area.width;
        for y in 0..buf.area.height {
            let symbols: Vec<&str> = (0..width).map(|x| buf[(x, y)].symbol()).collect();
            for start in 0..width as usize {
                if symbols[start..].concat().starts_with(needle) {
                    return (start as u16, y);
                }
            }
        }
        panic!("{needle:?} not found in rendered tree:\n{buf:?}");
    }

    /// Foreground color of the first cell of `needle` in the buffer.
    fn fg_of(buf: &ratatui::buffer::Buffer, needle: &str) -> Color {
        let (x, y) = cell_of(buf, needle);
        buf[(x, y)].fg
    }

    #[test]
    fn tree_renders_a_status_label_for_every_agent() {
        let buf = render_all_statuses(&Theme::build(&Palette::default()));
        // Each status is nameable in the tree, not just via the agents view.
        // cell_of panics if the label is absent from the rendered buffer.
        for label in ["running", "waiting", "idle"] {
            cell_of(&buf, label);
        }
    }

    #[test]
    fn tree_status_labels_use_distinct_palette_colors() {
        let p = Palette::default();
        let buf = render_all_statuses(&Theme::build(&p));

        assert_eq!(fg_of(&buf, "running"), p.status_running);
        assert_eq!(fg_of(&buf, "waiting"), p.status_waiting);
        assert_eq!(fg_of(&buf, "idle"), p.status_idle);

        // The regression that prompted this: running and waiting must never
        // collapse into colors that read the same at a glance.
        assert_ne!(p.status_running, p.status_waiting);
    }

    #[test]
    fn tree_status_label_padding_keeps_agent_names_aligned() {
        let buf = render_all_statuses(&Theme::build(&Palette::default()));
        // Names start at the same column regardless of label length
        // ("idle" is 3 chars shorter than "running").
        let run_col = cell_of(&buf, "alpha").0;
        assert_eq!(run_col, cell_of(&buf, "bravo").0);
        assert_eq!(run_col, cell_of(&buf, "charlie").0);
    }
}
