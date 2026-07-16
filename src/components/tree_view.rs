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
