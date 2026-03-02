use ratatui::text::Text;
use tui_tree_widget::TreeItem;

use crate::core::state::AppState;
use crate::theme;

/// Converts the app state into TreeItems for rendering.
/// Collections → Projects → Sessions (3-level hierarchy).
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

                    if session_children.is_empty() {
                        TreeItem::new_leaf(
                            proj.id.to_string(),
                            Text::styled(&proj.name, theme::PROJECT_STYLE),
                        )
                    } else {
                        TreeItem::new(
                            proj.id.to_string(),
                            Text::styled(&proj.name, theme::PROJECT_STYLE),
                            session_children,
                        )
                        .expect("session names are unique within a project")
                    }
                })
                .collect();

            TreeItem::new(
                col.id.to_string(),
                Text::styled(&col.name, theme::COLLECTION_STYLE),
                children,
            )
            .expect("project IDs are unique within a collection")
        })
        .collect()
}
