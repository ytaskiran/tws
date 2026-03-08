use uuid::Uuid;

use super::model::{Collection, Thread, Session, tmux_session_name_labeled, tmux_session_prefix};

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
    /// A thread is selected (collection index, thread index).
    Thread(usize, usize),
    /// A session is selected (collection index, thread index, session index within active_sessions for that thread).
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
        work.threads.push(Thread::new("Edge Device Pipeline"));
        work.threads.push(Thread::new("Model Training Infra"));
        work.threads.push(Thread::new("CI/CD Overhaul"));

        let mut learning = Collection::new("Learning");
        learning.threads.push(Thread::new("Rust Book"));
        learning.threads.push(Thread::new("Ratatui Experiments"));

        let mut podcast = Collection::new("Derin Notlar Podcast");
        podcast.threads.push(Thread::new("Episode 12"));
        podcast.threads.push(Thread::new("Episode 13 - Planning"));

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
                let thread_id = &selected[1];
                if let Some(col_idx) = self.find_collection_idx(col_id) {
                    if let Some(thread_idx) = self.find_thread_idx(col_idx, thread_id) {
                        SelectedItem::Thread(col_idx, thread_idx)
                    } else {
                        SelectedItem::None
                    }
                } else {
                    SelectedItem::None
                }
            }
            _ => {
                // Depth 3: collection / thread / session
                let col_id = &selected[0];
                let thread_id = &selected[1];
                let sess_name = &selected[2];
                if let Some(col_idx) = self.find_collection_idx(col_id) {
                    if let Some(thread_idx) = self.find_thread_idx(col_idx, thread_id) {
                        let thread = &self.collections[col_idx].threads[thread_idx];
                        let sessions = self.sessions_for_thread(thread.id);
                        if let Some(sess_idx) = sessions.iter().position(|s| s.tmux_session_name == *sess_name) {
                            SelectedItem::Session(col_idx, thread_idx, sess_idx)
                        } else {
                            SelectedItem::Thread(col_idx, thread_idx)
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

    pub fn add_thread(&mut self, collection_idx: usize, name: String) {
        if let Some(col) = self.collections.get_mut(collection_idx) {
            col.threads.push(Thread::new(name));
        }
    }

    pub fn rename_collection(&mut self, idx: usize, new_name: String) {
        if let Some(col) = self.collections.get_mut(idx) {
            col.name = new_name;
        }
    }

    pub fn rename_thread(&mut self, col_idx: usize, thread_idx: usize, new_name: String) {
        if let Some(col) = self.collections.get_mut(col_idx) {
            if let Some(thread) = col.threads.get_mut(thread_idx) {
                thread.name = new_name;
            }
        }
    }

    pub fn delete_collection(&mut self, idx: usize) {
        if idx < self.collections.len() {
            self.collections.remove(idx);
        }
    }

    pub fn delete_thread(&mut self, col_idx: usize, thread_idx: usize) {
        if let Some(col) = self.collections.get_mut(col_idx) {
            if thread_idx < col.threads.len() {
                col.threads.remove(thread_idx);
            }
        }
    }

    /// Get the name of a selected item (for pre-filling rename input).
    pub fn selected_name(&self, selected: &SelectedItem) -> Option<String> {
        match selected {
            SelectedItem::None => None,
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let thread_id = self.collections.get(*col_idx)?.threads.get(*thread_idx)?.id;
                let sessions = self.sessions_for_thread(thread_id);
                sessions.get(*sess_idx).map(|s| s.display_name.clone())
            }
            SelectedItem::Collection(idx) => {
                self.collections.get(*idx).map(|c| c.name.clone())
            }
            SelectedItem::Thread(col_idx, thread_idx) => self
                .collections
                .get(*col_idx)
                .and_then(|c| c.threads.get(*thread_idx))
                .map(|p| p.name.clone()),
        }
    }

    /// Generate the session prefix for a given collection/thread index pair.
    pub fn session_prefix_for(&self, col_idx: usize, thread_idx: usize) -> Option<String> {
        let col = self.collections.get(col_idx)?;
        let thread = col.threads.get(thread_idx)?;
        Some(tmux_session_prefix(&col.name, &thread.name))
    }

    /// Generate a labeled session name for a thread using the user-provided label.
    pub fn make_session_name(&self, col_idx: usize, thread_idx: usize, label: &str) -> Option<String> {
        let col = self.collections.get(col_idx)?;
        let thread = col.threads.get(thread_idx)?;
        Some(tmux_session_name_labeled(&col.name, &thread.name, label))
    }

    /// Get all active sessions belonging to a given thread.
    pub fn sessions_for_thread(&self, thread_id: Uuid) -> Vec<&Session> {
        self.active_sessions
            .iter()
            .filter(|s| s.thread_id == thread_id && s.alive)
            .collect()
    }

    /// Check whether a thread has any active sessions.
    pub fn has_active_session(&self, col_idx: usize, thread_idx: usize) -> bool {
        if let Some(col) = self.collections.get(col_idx) {
            if let Some(thread) = col.threads.get(thread_idx) {
                return self.active_sessions.iter().any(|s| s.thread_id == thread.id && s.alive);
            }
        }
        false
    }

    /// Refresh active_sessions by matching live tmux session names against
    /// our collection/thread hierarchy. Matches by prefix to support
    /// multiple labeled sessions per thread (e.g. `tws_work_pipeline_bugfix`).
    ///
    /// Each entry is `(session_name, last_attached_timestamp)`.
    pub fn refresh_sessions(&mut self, live_tmux_sessions: &[(String, i64)]) {
        self.active_sessions.clear();

        for col in &self.collections {
            for thread in &col.threads {
                let prefix = tmux_session_prefix(&col.name, &thread.name);
                for (session_name, last_attached) in live_tmux_sessions {
                    // Match "prefix_label" where label is any non-empty suffix
                    if let Some(rest) = session_name.strip_prefix(&prefix) {
                        if let Some(label) = rest.strip_prefix('_') {
                            if !label.is_empty() {
                                self.active_sessions.push(Session {
                                    tmux_session_name: session_name.clone(),
                                    display_name: label.to_string(),
                                    thread_id: thread.id,
                                    alive: true,
                                    last_attached: *last_attached,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// Given a thread ID, find its collection and thread names.
    /// Returns `(collection_name, thread_name)`.
    pub fn resolve_thread_path(&self, thread_id: Uuid) -> Option<(String, String)> {
        for col in &self.collections {
            for thread in &col.threads {
                if thread.id == thread_id {
                    return Some((col.name.clone(), thread.name.clone()));
                }
            }
        }
        None
    }

    /// Returns the `n` most recently attached alive sessions, sorted by
    /// recency (most recent first). Sessions with `last_attached == 0` are excluded.
    pub fn recent_sessions(&self, n: usize) -> Vec<&Session> {
        let mut recent: Vec<&Session> = self
            .active_sessions
            .iter()
            .filter(|s| s.alive && s.last_attached > 0)
            .collect();
        recent.sort_by(|a, b| b.last_attached.cmp(&a.last_attached));
        recent.truncate(n);
        recent
    }

    fn find_collection_idx(&self, uuid_str: &str) -> Option<usize> {
        let id: Uuid = uuid_str.parse().ok()?;
        self.collections.iter().position(|c| c.id == id)
    }

    fn find_thread_idx(&self, col_idx: usize, uuid_str: &str) -> Option<usize> {
        let id: Uuid = uuid_str.parse().ok()?;
        self.collections
            .get(col_idx)?
            .threads
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
    fn add_thread_to_collection() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_thread(0, "Pipeline".into());
        assert_eq!(state.collections[0].threads.len(), 1);
        assert_eq!(state.collections[0].threads[0].name, "Pipeline");
    }

    #[test]
    fn rename_collection() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.rename_collection(0, "Job".into());
        assert_eq!(state.collections[0].name, "Job");
    }

    #[test]
    fn rename_thread() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_thread(0, "Old".into());
        state.rename_thread(0, 0, "New".into());
        assert_eq!(state.collections[0].threads[0].name, "New");
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
    fn delete_thread() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_thread(0, "A".into());
        state.add_thread(0, "B".into());
        state.delete_thread(0, 0);
        assert_eq!(state.collections[0].threads.len(), 1);
        assert_eq!(state.collections[0].threads[0].name, "B");
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
    fn resolve_thread_selection() {
        let state = AppState::with_sample_data();
        let col_id = state.collections[0].id.to_string();
        let thread_id = state.collections[0].threads[1].id.to_string();
        match state.resolve_selection(&[col_id, thread_id]) {
            SelectedItem::Thread(col_idx, thread_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(thread_idx, 1);
            }
            _ => panic!("expected Thread"),
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
            ("tws_work_edge-device-pipeline_bugfix".to_string(), 0),
            ("tws_work_edge-device-pipeline_hotfix".to_string(), 0),
        ];
        state.refresh_sessions(&live);
        assert_eq!(state.active_sessions.len(), 2);
        assert_eq!(state.active_sessions[0].display_name, "bugfix");
        assert_eq!(state.active_sessions[1].display_name, "hotfix");
        assert_eq!(
            state.active_sessions[0].thread_id,
            state.collections[0].threads[0].id
        );
    }

    #[test]
    fn refresh_sessions_ignores_non_matching() {
        let mut state = AppState::with_sample_data();
        let live = vec![("some-other-session".to_string(), 0)];
        state.refresh_sessions(&live);
        assert!(state.active_sessions.is_empty());
    }

    #[test]
    fn refresh_sessions_ignores_bare_prefix() {
        let mut state = AppState::with_sample_data();
        // The bare prefix without _label should NOT match
        let live = vec![("tws_work_edge-device-pipeline".to_string(), 0)];
        state.refresh_sessions(&live);
        assert!(state.active_sessions.is_empty());
    }

    #[test]
    fn has_active_session_works() {
        let mut state = AppState::with_sample_data();
        assert!(!state.has_active_session(0, 0));
        let live = vec![("tws_work_edge-device-pipeline_bugfix".to_string(), 0)];
        state.refresh_sessions(&live);
        assert!(state.has_active_session(0, 0));
        assert!(!state.has_active_session(0, 1));
    }

    #[test]
    fn resolve_session_selection() {
        let mut state = AppState::with_sample_data();
        let live = vec![("tws_work_edge-device-pipeline_bugfix".to_string(), 0)];
        state.refresh_sessions(&live);

        let col_id = state.collections[0].id.to_string();
        let thread_id = state.collections[0].threads[0].id.to_string();
        let sess_name = "tws_work_edge-device-pipeline_bugfix".to_string();
        match state.resolve_selection(&[col_id, thread_id, sess_name]) {
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(thread_idx, 0);
                assert_eq!(sess_idx, 0);
            }
            _ => panic!("expected Session"),
        }
    }

    #[test]
    fn resolve_session_selection_multiple() {
        let mut state = AppState::with_sample_data();
        let live = vec![
            ("tws_work_edge-device-pipeline_bugfix".to_string(), 0),
            ("tws_work_edge-device-pipeline_hotfix".to_string(), 0),
        ];
        state.refresh_sessions(&live);

        let col_id = state.collections[0].id.to_string();
        let thread_id = state.collections[0].threads[0].id.to_string();

        // Select the second session
        let sess_name = "tws_work_edge-device-pipeline_hotfix".to_string();
        match state.resolve_selection(&[col_id, thread_id, sess_name]) {
            SelectedItem::Session(_, _, sess_idx) => assert_eq!(sess_idx, 1),
            _ => panic!("expected Session"),
        }
    }

    #[test]
    fn recent_sessions_sorted_by_recency() {
        let mut state = AppState::with_sample_data();
        let live = vec![
            ("tws_work_edge-device-pipeline_bugfix".to_string(), 1000),
            ("tws_work_edge-device-pipeline_hotfix".to_string(), 3000),
            ("tws_work_model-training-infra_main".to_string(), 2000),
        ];
        state.refresh_sessions(&live);

        let recent = state.recent_sessions(5);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].display_name, "hotfix");       // ts 3000
        assert_eq!(recent[1].display_name, "main");          // ts 2000
        assert_eq!(recent[2].display_name, "bugfix");        // ts 1000

        // Truncation works
        let recent2 = state.recent_sessions(2);
        assert_eq!(recent2.len(), 2);
        assert_eq!(recent2[0].display_name, "hotfix");
        assert_eq!(recent2[1].display_name, "main");
    }
}
