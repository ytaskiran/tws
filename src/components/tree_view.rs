use ratatui::text::{Line, Span, Text};
use tui_tree_widget::TreeItem;

use crate::core::state::AppState;
use crate::theme;

/// Converts the app state into TreeItems for rendering.
/// Collections -> Threads -> Sessions (3-level hierarchy).
pub fn build_tree_items<'a>(state: &'a AppState) -> Vec<TreeItem<'a, String>> {
    state
        .collections
        .iter()
        .map(|col| {
            let children: Vec<TreeItem<'a, String>> = col
                .threads
                .iter()
                .map(|thread| {
                    // Find active sessions for this thread
                    let session_children: Vec<TreeItem<'a, String>> = state
                        .active_sessions
                        .iter()
                        .filter(|s| s.thread_id == thread.id)
                        .map(|s| {
                            TreeItem::new_leaf(
                                s.tmux_session_name.clone(),
                                Text::styled(&s.display_name, theme::SESSION_STYLE),
                            )
                        })
                        .collect();

                    let session_count = session_children.len();

                    // Build thread display with optional session badge
                    let thread_text = if session_count > 0 {
                        Text::from(Line::from(vec![
                            Span::styled(thread.name.as_str(), theme::THREAD_STYLE),
                            Span::styled(" \u{25CF} ", theme::BADGE_DOT_STYLE),
                            Span::styled(session_count.to_string(), theme::BADGE_COUNT_STYLE),
                        ]))
                    } else {
                        Text::styled(thread.name.as_str(), theme::THREAD_DIM_STYLE)
                    };

                    if session_children.is_empty() {
                        TreeItem::new_leaf(thread.id.to_string(), thread_text)
                    } else {
                        TreeItem::new(thread.id.to_string(), thread_text, session_children)
                            .expect("session names are unique within a thread")
                    }
                })
                .collect();

            TreeItem::new(
                col.id.to_string(),
                Text::styled(col.name.as_str(), theme::COLLECTION_STYLE),
                children,
            )
            .expect("thread IDs are unique within a collection")
        })
        .collect()
}
