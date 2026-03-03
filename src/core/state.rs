use uuid::Uuid;

use super::model::{Collection, Project, Session, tmux_session_name_labeled, tmux_session_prefix};

pub struct AppState {
    pub collections: Vec<Collection>,
    /// Runtime-only: live tmux sessions managed by tws. Never persisted.
    pub active_sessions: Vec<Session>,
}

/// What the current tree selection points to.
pub enum SelectedItem {
    /// Nothing is selected.
    None,
    /// A collection is selected (index into collections vec).
    Collection(usize),
    /// A project is selected (collection index, project index).
    Project(usize, usize),
    /// A session is selected (collection index, project index, session index within active_sessions for that project).
    Session(usize, usize, usize),
}

impl AppState {
    pub fn new() -> Self {
        Self {
            collections: Vec::new(),
            active_sessions: Vec::new(),
        }
    }

    /// Creates sample data for development/testing.
    pub fn with_sample_data() -> Self {
        let mut work = Collection::new("Work");
        work.projects.push(Project::new("Edge Device Pipeline"));
        work.projects.push(Project::new("Model Training Infra"));
        work.projects.push(Project::new("CI/CD Overhaul"));

        let mut learning = Collection::new("Learning");
        learning.projects.push(Project::new("Rust Book"));
        learning.projects.push(Project::new("Ratatui Experiments"));

        let mut podcast = Collection::new("Derin Notlar Podcast");
        podcast.projects.push(Project::new("Episode 12"));
        podcast.projects.push(Project::new("Episode 13 - Planning"));

        let personal = Collection::new("Personal");

        Self {
            collections: vec![work, learning, podcast, personal],
            active_sessions: Vec::new(),
        }
    }

    /// Resolve a tree selection path (from TreeState::selected()) to a SelectedItem.
    pub fn resolve_selection(&self, selected: &[String]) -> SelectedItem {
        match selected.len() {
            0 => SelectedItem::None,
            1 => {
                let id = &selected[0];
                if let Some(idx) = self.find_collection_idx(id) {
                    SelectedItem::Collection(idx)
                } else {
                    SelectedItem::None
                }
            }
            2 => {
                let col_id = &selected[0];
                let proj_id = &selected[1];
                if let Some(col_idx) = self.find_collection_idx(col_id) {
                    if let Some(proj_idx) = self.find_project_idx(col_idx, proj_id) {
                        SelectedItem::Project(col_idx, proj_idx)
                    } else {
                        SelectedItem::None
                    }
                } else {
                    SelectedItem::None
                }
            }
            _ => {
                // Depth 3: collection / project / session
                let col_id = &selected[0];
                let proj_id = &selected[1];
                let sess_name = &selected[2];
                if let Some(col_idx) = self.find_collection_idx(col_id) {
                    if let Some(proj_idx) = self.find_project_idx(col_idx, proj_id) {
                        let proj = &self.collections[col_idx].projects[proj_idx];
                        let sessions = self.sessions_for_project(proj.id);
                        if let Some(sess_idx) = sessions.iter().position(|s| s.tmux_session_name == *sess_name) {
                            SelectedItem::Session(col_idx, proj_idx, sess_idx)
                        } else {
                            SelectedItem::Project(col_idx, proj_idx)
                        }
                    } else {
                        SelectedItem::None
                    }
                } else {
                    SelectedItem::None
                }
            }
        }
    }

    pub fn add_collection(&mut self, name: String) {
        self.collections.push(Collection::new(name));
    }

    pub fn add_project(&mut self, collection_idx: usize, name: String) {
        if let Some(col) = self.collections.get_mut(collection_idx) {
            col.projects.push(Project::new(name));
        }
    }

    pub fn rename_collection(&mut self, idx: usize, new_name: String) {
        if let Some(col) = self.collections.get_mut(idx) {
            col.name = new_name;
        }
    }

    pub fn rename_project(&mut self, col_idx: usize, proj_idx: usize, new_name: String) {
        if let Some(col) = self.collections.get_mut(col_idx) {
            if let Some(proj) = col.projects.get_mut(proj_idx) {
                proj.name = new_name;
            }
        }
    }

    pub fn delete_collection(&mut self, idx: usize) {
        if idx < self.collections.len() {
            self.collections.remove(idx);
        }
    }

    pub fn delete_project(&mut self, col_idx: usize, proj_idx: usize) {
        if let Some(col) = self.collections.get_mut(col_idx) {
            if proj_idx < col.projects.len() {
                col.projects.remove(proj_idx);
            }
        }
    }

    /// Get the name of a selected item (for pre-filling rename input).
    pub fn selected_name(&self, selected: &SelectedItem) -> Option<String> {
        match selected {
            SelectedItem::None => None,
            SelectedItem::Session(col_idx, proj_idx, sess_idx) => {
                let proj_id = self.collections.get(*col_idx)?.projects.get(*proj_idx)?.id;
                let sessions = self.sessions_for_project(proj_id);
                sessions.get(*sess_idx).map(|s| s.display_name.clone())
            }
            SelectedItem::Collection(idx) => {
                self.collections.get(*idx).map(|c| c.name.clone())
            }
            SelectedItem::Project(col_idx, proj_idx) => self
                .collections
                .get(*col_idx)
                .and_then(|c| c.projects.get(*proj_idx))
                .map(|p| p.name.clone()),
        }
    }

    /// Generate the session prefix for a given collection/project index pair.
    pub fn session_prefix_for(&self, col_idx: usize, proj_idx: usize) -> Option<String> {
        let col = self.collections.get(col_idx)?;
        let proj = col.projects.get(proj_idx)?;
        Some(tmux_session_prefix(&col.name, &proj.name))
    }

    /// Generate a labeled session name for a project using the user-provided label.
    pub fn make_session_name(&self, col_idx: usize, proj_idx: usize, label: &str) -> Option<String> {
        let col = self.collections.get(col_idx)?;
        let proj = col.projects.get(proj_idx)?;
        Some(tmux_session_name_labeled(&col.name, &proj.name, label))
    }

    /// Get all active sessions belonging to a given project.
    pub fn sessions_for_project(&self, project_id: Uuid) -> Vec<&Session> {
        self.active_sessions
            .iter()
            .filter(|s| s.project_id == project_id && s.alive)
            .collect()
    }

    /// Check whether a project has any active sessions.
    pub fn has_active_session(&self, col_idx: usize, proj_idx: usize) -> bool {
        if let Some(col) = self.collections.get(col_idx) {
            if let Some(proj) = col.projects.get(proj_idx) {
                return self.active_sessions.iter().any(|s| s.project_id == proj.id && s.alive);
            }
        }
        false
    }

    /// Refresh active_sessions by matching live tmux session names against
    /// our collection/project hierarchy. Matches by prefix to support
    /// multiple labeled sessions per project (e.g. `tws_work_pipeline_bugfix`).
    pub fn refresh_sessions(&mut self, live_tmux_sessions: &[String]) {
        self.active_sessions.clear();

        for col in &self.collections {
            for proj in &col.projects {
                let prefix = tmux_session_prefix(&col.name, &proj.name);
                for session_name in live_tmux_sessions {
                    // Match "prefix_label" where label is any non-empty suffix
                    if let Some(rest) = session_name.strip_prefix(&prefix) {
                        if let Some(label) = rest.strip_prefix('_') {
                            if !label.is_empty() {
                                self.active_sessions.push(Session {
                                    tmux_session_name: session_name.clone(),
                                    display_name: label.to_string(),
                                    project_id: proj.id,
                                    alive: true,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn find_collection_idx(&self, uuid_str: &str) -> Option<usize> {
        let id: Uuid = uuid_str.parse().ok()?;
        self.collections.iter().position(|c| c.id == id)
    }

    fn find_project_idx(&self, col_idx: usize, uuid_str: &str) -> Option<usize> {
        let id: Uuid = uuid_str.parse().ok()?;
        self.collections
            .get(col_idx)?
            .projects
            .iter()
            .position(|p| p.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_find_collection() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        assert_eq!(state.collections.len(), 1);
        assert_eq!(state.collections[0].name, "Work");
    }

    #[test]
    fn add_project_to_collection() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_project(0, "Pipeline".into());
        assert_eq!(state.collections[0].projects.len(), 1);
        assert_eq!(state.collections[0].projects[0].name, "Pipeline");
    }

    #[test]
    fn rename_collection() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.rename_collection(0, "Job".into());
        assert_eq!(state.collections[0].name, "Job");
    }

    #[test]
    fn rename_project() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_project(0, "Old".into());
        state.rename_project(0, 0, "New".into());
        assert_eq!(state.collections[0].projects[0].name, "New");
    }

    #[test]
    fn delete_collection() {
        let mut state = AppState::new();
        state.add_collection("A".into());
        state.add_collection("B".into());
        state.delete_collection(0);
        assert_eq!(state.collections.len(), 1);
        assert_eq!(state.collections[0].name, "B");
    }

    #[test]
    fn delete_project() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_project(0, "A".into());
        state.add_project(0, "B".into());
        state.delete_project(0, 0);
        assert_eq!(state.collections[0].projects.len(), 1);
        assert_eq!(state.collections[0].projects[0].name, "B");
    }

    #[test]
    fn resolve_collection_selection() {
        let state = AppState::with_sample_data();
        let id = state.collections[0].id.to_string();
        match state.resolve_selection(&[id]) {
            SelectedItem::Collection(idx) => assert_eq!(idx, 0),
            _ => panic!("expected Collection"),
        }
    }

    #[test]
    fn resolve_project_selection() {
        let state = AppState::with_sample_data();
        let col_id = state.collections[0].id.to_string();
        let proj_id = state.collections[0].projects[1].id.to_string();
        match state.resolve_selection(&[col_id, proj_id]) {
            SelectedItem::Project(col_idx, proj_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(proj_idx, 1);
            }
            _ => panic!("expected Project"),
        }
    }

    #[test]
    fn resolve_empty_selection() {
        let state = AppState::new();
        match state.resolve_selection(&[]) {
            SelectedItem::None => {}
            _ => panic!("expected None"),
        }
    }

    #[test]
    fn session_prefix_for_valid() {
        let state = AppState::with_sample_data();
        let prefix = state.session_prefix_for(0, 0).unwrap();
        assert_eq!(prefix, "tws_work_edge-device-pipeline");
    }

    #[test]
    fn make_session_name_labeled() {
        let state = AppState::with_sample_data();
        let name = state.make_session_name(0, 0, "bugfix").unwrap();
        assert_eq!(name, "tws_work_edge-device-pipeline_bugfix");
    }

    #[test]
    fn make_session_name_slugifies_label() {
        let state = AppState::with_sample_data();
        let name = state.make_session_name(0, 0, "Hot Fix 2").unwrap();
        assert_eq!(name, "tws_work_edge-device-pipeline_hot-fix-2");
    }

    #[test]
    fn refresh_sessions_discovers_labeled() {
        let mut state = AppState::with_sample_data();
        let live = vec![
            "tws_work_edge-device-pipeline_bugfix".to_string(),
            "tws_work_edge-device-pipeline_hotfix".to_string(),
        ];
        state.refresh_sessions(&live);
        assert_eq!(state.active_sessions.len(), 2);
        assert_eq!(state.active_sessions[0].display_name, "bugfix");
        assert_eq!(state.active_sessions[1].display_name, "hotfix");
        assert_eq!(
            state.active_sessions[0].project_id,
            state.collections[0].projects[0].id
        );
    }

    #[test]
    fn refresh_sessions_ignores_non_matching() {
        let mut state = AppState::with_sample_data();
        let live = vec!["some-other-session".to_string()];
        state.refresh_sessions(&live);
        assert!(state.active_sessions.is_empty());
    }

    #[test]
    fn refresh_sessions_ignores_bare_prefix() {
        let mut state = AppState::with_sample_data();
        // The bare prefix without _label should NOT match
        let live = vec!["tws_work_edge-device-pipeline".to_string()];
        state.refresh_sessions(&live);
        assert!(state.active_sessions.is_empty());
    }

    #[test]
    fn has_active_session_works() {
        let mut state = AppState::with_sample_data();
        assert!(!state.has_active_session(0, 0));
        let live = vec!["tws_work_edge-device-pipeline_bugfix".to_string()];
        state.refresh_sessions(&live);
        assert!(state.has_active_session(0, 0));
        assert!(!state.has_active_session(0, 1));
    }

    #[test]
    fn resolve_session_selection() {
        let mut state = AppState::with_sample_data();
        let live = vec!["tws_work_edge-device-pipeline_bugfix".to_string()];
        state.refresh_sessions(&live);

        let col_id = state.collections[0].id.to_string();
        let proj_id = state.collections[0].projects[0].id.to_string();
        let sess_name = "tws_work_edge-device-pipeline_bugfix".to_string();
        match state.resolve_selection(&[col_id, proj_id, sess_name]) {
            SelectedItem::Session(col_idx, proj_idx, sess_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(proj_idx, 0);
                assert_eq!(sess_idx, 0);
            }
            _ => panic!("expected Session"),
        }
    }

    #[test]
    fn resolve_session_selection_multiple() {
        let mut state = AppState::with_sample_data();
        let live = vec![
            "tws_work_edge-device-pipeline_bugfix".to_string(),
            "tws_work_edge-device-pipeline_hotfix".to_string(),
        ];
        state.refresh_sessions(&live);

        let col_id = state.collections[0].id.to_string();
        let proj_id = state.collections[0].projects[0].id.to_string();

        // Select the second session
        let sess_name = "tws_work_edge-device-pipeline_hotfix".to_string();
        match state.resolve_selection(&[col_id, proj_id, sess_name]) {
            SelectedItem::Session(_, _, sess_idx) => assert_eq!(sess_idx, 1),
            _ => panic!("expected Session"),
        }
    }
}
