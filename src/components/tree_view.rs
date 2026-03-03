use ratatui::text::{Line, Span, Text};
use tui_tree_widget::TreeItem;

use crate::core::state::AppState;
use crate::theme;

/// Converts the app state into TreeItems for rendering.
/// Collections -> Projects -> Sessions (3-level hierarchy).
pub fn build_tree_items<'a>(state: &'a AppState) -> Vec<TreeItem<'a, String>> {
    state
        .collections
        .iter()
        .map(|col| {
            let children: Vec<TreeItem<'a, String>> = col
                .projects
                .iter()
                .map(|proj| {
                    // Find active sessions for this project
                    let session_children: Vec<TreeItem<'a, String>> = state
                        .active_sessions
                        .iter()
                        .filter(|s| s.project_id == proj.id && s.alive)
                        .map(|s| {
                            TreeItem::new_leaf(
                                s.tmux_session_name.clone(),
                                Text::styled(&s.display_name, theme::SESSION_STYLE),
                            )
                        })
                        .collect();

                    let session_count = session_children.len();

                    // Build project display with optional session badge
                    let project_text = if session_count > 0 {
                        Text::from(Line::from(vec![
                            Span::styled(proj.name.as_str(), theme::PROJECT_STYLE),
                            Span::styled(" \u{25CF} ", theme::BADGE_DOT_STYLE),
                            Span::styled(session_count.to_string(), theme::BADGE_COUNT_STYLE),
                        ]))
                    } else {
                        Text::styled(proj.name.as_str(), theme::PROJECT_DIM_STYLE)
                    };

                    if session_children.is_empty() {
                        TreeItem::new_leaf(proj.id.to_string(), project_text)
                    } else {
                        TreeItem::new(proj.id.to_string(), project_text, session_children)
                            .expect("session names are unique within a project")
                    }
                })
                .collect();

            TreeItem::new(
                col.id.to_string(),
                Text::from(vec![
                    Line::from(""),
                    Line::from(Span::styled(col.name.as_str(), theme::COLLECTION_STYLE)),
                ]),
                children,
            )
            .expect("project IDs are unique within a collection")
        })
        .collect()
}
