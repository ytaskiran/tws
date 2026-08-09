use uuid::Uuid;

use super::model::{
    AgentSession, Collection, Session, Thread, tmux_root_session_name_labeled,
    tmux_root_session_prefix, tmux_session_name_labeled, tmux_session_prefix,
};

pub struct AppState {
    pub collections: Vec<Collection>,
    /// Runtime-only; excluded from persistence.
    pub active_sessions: Vec<Session>,
    /// Runtime-only; excluded from persistence.
    pub agent_sessions: Vec<AgentSession>,
}

/// A single agent flattened out of the collection/thread/session hierarchy.
/// Carries both display strings and the index tuple needed to produce `SelectedItem::Agent`.
pub struct FlatAgent {
    pub col_idx: usize,
    pub thread_idx: usize,
    pub thread_name: String,
    pub sess_idx: usize,
    pub session_display_name: String,
    pub agent_idx: usize,
    pub agent_type: super::model::AgentType,
    pub agent_display_name: String,
    pub tmux_session_name: String,
    pub window_index: u32,
    pub pane_id: String,
    pub pin_slot: Option<u8>,
    pub status: super::model::AgentStatus,
}

pub enum SelectedItem {
    None,
    Collection(usize),
    Thread(usize, usize),
    Session(usize, usize, usize),
    Agent(usize, usize, usize, usize),
}

impl AppState {
    // Tree paths use UUIDs for collections and threads, tmux names for sessions,
    // and pane IDs for agents; root threads omit the collection segment.
    pub fn resolve_selection(&self, selected: &[String]) -> SelectedItem {
        match selected.len() {
            0 => SelectedItem::None,
            1 => {
                let id = &selected[0];
                if let Some(idx) = self.find_collection_idx(id) {
                    SelectedItem::Collection(idx)
                } else if let Some((col_idx, thread_idx)) = self.find_root_thread_by_uuid(id) {
                    SelectedItem::Thread(col_idx, thread_idx)
                } else {
                    SelectedItem::None
                }
            }
            2 => {
                let first = &selected[0];
                let second = &selected[1];
                if let Some(col_idx) = self.find_collection_idx(first)
                    && let Some(thread_idx) = self.find_thread_idx(col_idx, second)
                {
                    return SelectedItem::Thread(col_idx, thread_idx);
                }
                if let Some((col_idx, thread_idx)) = self.find_root_thread_by_uuid(first) {
                    let thread = &self.collections[col_idx].threads[thread_idx];
                    let sessions = self.sessions_for_thread(thread.id);
                    if let Some(sess_idx) =
                        sessions.iter().position(|s| s.tmux_session_name == *second)
                    {
                        return SelectedItem::Session(col_idx, thread_idx, sess_idx);
                    }
                }
                SelectedItem::None
            }
            3 => {
                if let Some(col_idx) = self.find_collection_idx(&selected[0])
                    && let Some(thread_idx) = self.find_thread_idx(col_idx, &selected[1])
                {
                    let thread = &self.collections[col_idx].threads[thread_idx];
                    let sessions = self.sessions_for_thread(thread.id);
                    if let Some(sess_idx) = sessions
                        .iter()
                        .position(|s| s.tmux_session_name == selected[2])
                    {
                        return SelectedItem::Session(col_idx, thread_idx, sess_idx);
                    } else {
                        return SelectedItem::Thread(col_idx, thread_idx);
                    }
                }
                if let Some((col_idx, thread_idx)) = self.find_root_thread_by_uuid(&selected[0]) {
                    let thread = &self.collections[col_idx].threads[thread_idx];
                    let sessions = self.sessions_for_thread(thread.id);
                    if let Some(sess_idx) = sessions
                        .iter()
                        .position(|s| s.tmux_session_name == selected[1])
                    {
                        let agents = self.agents_for_session(&selected[1]);
                        if let Some(agent_idx) =
                            agents.iter().position(|a| a.pane_id == selected[2])
                        {
                            return SelectedItem::Agent(col_idx, thread_idx, sess_idx, agent_idx);
                        }
                        return SelectedItem::Session(col_idx, thread_idx, sess_idx);
                    }
                }
                SelectedItem::None
            }
            4 => {
                if let Some(col_idx) = self.find_collection_idx(&selected[0])
                    && let Some(thread_idx) = self.find_thread_idx(col_idx, &selected[1])
                {
                    let thread = &self.collections[col_idx].threads[thread_idx];
                    let sessions = self.sessions_for_thread(thread.id);
                    if let Some(sess_idx) = sessions
                        .iter()
                        .position(|s| s.tmux_session_name == selected[2])
                    {
                        let agents = self.agents_for_session(&selected[2]);
                        if let Some(agent_idx) =
                            agents.iter().position(|a| a.pane_id == selected[3])
                        {
                            return SelectedItem::Agent(col_idx, thread_idx, sess_idx, agent_idx);
                        }
                        return SelectedItem::Session(col_idx, thread_idx, sess_idx);
                    }
                }
                SelectedItem::None
            }
            _ => SelectedItem::None,
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
        if let Some(col) = self.collections.get_mut(col_idx)
            && let Some(thread) = col.threads.get_mut(thread_idx)
        {
            thread.name = new_name;
        }
    }

    pub fn set_thread_working_dir(
        &mut self,
        col_idx: usize,
        thread_idx: usize,
        dir: Option<std::path::PathBuf>,
    ) {
        if let Some(col) = self.collections.get_mut(col_idx)
            && let Some(thread) = col.threads.get_mut(thread_idx)
        {
            thread.working_dir = dir;
        }
    }

    pub fn delete_collection(&mut self, idx: usize) {
        if idx < self.collections.len() {
            self.collections.remove(idx);
        }
    }

    pub fn delete_thread(&mut self, col_idx: usize, thread_idx: usize) {
        if let Some(col) = self.collections.get_mut(col_idx)
            && thread_idx < col.threads.len()
        {
            col.threads.remove(thread_idx);
        }
    }

    pub fn selected_name(&self, selected: &SelectedItem) -> Option<String> {
        match selected {
            SelectedItem::None => None,
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let thread_id = self.collections.get(*col_idx)?.threads.get(*thread_idx)?.id;
                let sessions = self.sessions_for_thread(thread_id);
                sessions.get(*sess_idx).map(|s| s.display_name.clone())
            }
            SelectedItem::Collection(idx) => self.collections.get(*idx).map(|c| c.name.clone()),
            SelectedItem::Thread(col_idx, thread_idx) => self
                .collections
                .get(*col_idx)
                .and_then(|c| c.threads.get(*thread_idx))
                .map(|p| p.name.clone()),
            SelectedItem::Agent(col_idx, thread_idx, sess_idx, agent_idx) => self
                .resolve_agent(*col_idx, *thread_idx, *sess_idx, *agent_idx)
                .map(|a| a.display_name.clone()),
        }
    }

    pub fn make_session_name(
        &self,
        col_idx: usize,
        thread_idx: usize,
        label: &str,
    ) -> Option<String> {
        let col = self.collections.get(col_idx)?;
        let thread = col.threads.get(thread_idx)?;
        if col.is_root {
            Some(tmux_root_session_name_labeled(&thread.name, label))
        } else {
            Some(tmux_session_name_labeled(&col.name, &thread.name, label))
        }
    }

    /// Assign the lowest free pin slot, preserving an existing assignment.
    pub fn pin_agent_auto(&mut self, pane_id: &str) -> Option<u8> {
        if let Some(slot) = self
            .agent_sessions
            .iter()
            .find(|a| a.pane_id == pane_id)
            .and_then(|a| a.pin_slot)
        {
            return Some(slot);
        }
        let used: std::collections::HashSet<u8> = self
            .agent_sessions
            .iter()
            .filter_map(|a| a.pin_slot)
            .collect();
        let slot = (0u8..=9).find(|s| !used.contains(s))?;
        if let Some(agent) = self
            .agent_sessions
            .iter_mut()
            .find(|a| a.pane_id == pane_id)
        {
            agent.pin_slot = Some(slot);
            Some(slot)
        } else {
            None
        }
    }

    /// Assign a pin slot, swapping or relocating an occupant as needed.
    pub fn pin_agent_to(&mut self, pane_id: &str, slot: u8) {
        let slot = slot.min(9);

        let moving_existing = self
            .agent_sessions
            .iter()
            .find(|a| a.pane_id == pane_id)
            .and_then(|a| a.pin_slot);

        if moving_existing == Some(slot) {
            return;
        }

        let occupant_pane_id: Option<String> = self
            .agent_sessions
            .iter()
            .find(|a| a.pin_slot == Some(slot))
            .map(|a| a.pane_id.clone());

        if let Some(agent) = self
            .agent_sessions
            .iter_mut()
            .find(|a| a.pane_id == pane_id)
        {
            agent.pin_slot = Some(slot);
        } else {
            return;
        }

        if let Some(occupant_id) = occupant_pane_id {
            if let Some(prev_slot) = moving_existing {
                if let Some(agent) = self
                    .agent_sessions
                    .iter_mut()
                    .find(|a| a.pane_id == occupant_id)
                {
                    agent.pin_slot = Some(prev_slot);
                }
            } else {
                if let Some(agent) = self
                    .agent_sessions
                    .iter_mut()
                    .find(|a| a.pane_id == occupant_id)
                {
                    agent.pin_slot = None;
                }
                self.pin_agent_auto(&occupant_id);
            }
        }
    }

    pub fn unpin_agent(&mut self, pane_id: &str) {
        if let Some(agent) = self
            .agent_sessions
            .iter_mut()
            .find(|a| a.pane_id == pane_id)
        {
            agent.pin_slot = None;
        }
    }

    pub fn agent_by_pin_slot(&self, slot: u8) -> Option<&AgentSession> {
        self.agent_sessions
            .iter()
            .find(|a| a.pin_slot == Some(slot))
    }

    pub fn agents_for_session(&self, tmux_session_name: &str) -> Vec<&AgentSession> {
        self.agent_sessions
            .iter()
            .filter(|a| a.tmux_session_name == tmux_session_name)
            .collect()
    }

    pub fn resolve_agent(
        &self,
        col_idx: usize,
        thread_idx: usize,
        sess_idx: usize,
        agent_idx: usize,
    ) -> Option<&AgentSession> {
        let thread_id = self.collections.get(col_idx)?.threads.get(thread_idx)?.id;
        let sessions = self.sessions_for_thread(thread_id);
        let session = sessions.get(sess_idx)?;
        let agents = self.agents_for_session(&session.tmux_session_name);
        agents.get(agent_idx).copied()
    }

    pub fn sessions_for_thread(&self, thread_id: Uuid) -> Vec<&Session> {
        self.active_sessions
            .iter()
            .filter(|s| s.thread_id == thread_id)
            .collect()
    }

    pub fn has_active_session(&self, col_idx: usize, thread_idx: usize) -> bool {
        if let Some(col) = self.collections.get(col_idx)
            && let Some(thread) = col.threads.get(thread_idx)
        {
            return self
                .active_sessions
                .iter()
                .any(|s| s.thread_id == thread.id);
        }
        false
    }

    // Prefix matching supports multiple labeled sessions per thread.
    pub fn refresh_sessions(&mut self, live_tmux_sessions: &[(String, i64)]) {
        self.active_sessions.clear();

        for col in &self.collections {
            for thread in &col.threads {
                let prefix = if col.is_root {
                    tmux_root_session_prefix(&thread.name)
                } else {
                    tmux_session_prefix(&col.name, &thread.name)
                };
                for (session_name, last_attached) in live_tmux_sessions {
                    if let Some(rest) = session_name.strip_prefix(&prefix)
                        && let Some(label) = rest.strip_prefix('_')
                        && !label.is_empty()
                    {
                        self.active_sessions.push(Session {
                            tmux_session_name: session_name.clone(),
                            display_name: label.to_string(),
                            thread_id: thread.id,
                            last_attached: *last_attached,
                        });
                    }
                }
            }
        }
    }

    pub fn session_display_path(&self, session: &Session) -> Option<String> {
        let (col_name, thread_name) = self.resolve_thread_path(session.thread_id)?;
        Some(match col_name {
            Some(c) => format!("{}/{}/{}", c, thread_name, session.display_name),
            None => format!("{}/{}", thread_name, session.display_name),
        })
    }

    pub fn all_threads_display(&self) -> Vec<(usize, usize, String)> {
        let mut result = Vec::new();
        for (col_idx, col) in self.collections.iter().enumerate() {
            for (thread_idx, thread) in col.threads.iter().enumerate() {
                let path = if col.is_root {
                    thread.name.clone()
                } else {
                    format!("{}/{}", col.name, thread.name)
                };
                result.push((col_idx, thread_idx, path));
            }
        }
        result
    }

    pub fn resolve_thread_path(&self, thread_id: Uuid) -> Option<(Option<String>, String)> {
        for col in &self.collections {
            for thread in &col.threads {
                if thread.id == thread_id {
                    let col_name = if col.is_root {
                        None
                    } else {
                        Some(col.name.clone())
                    };
                    return Some((col_name, thread.name.clone()));
                }
            }
        }
        None
    }

    pub fn recent_sessions(&self, n: usize) -> Vec<&Session> {
        let mut recent: Vec<&Session> = self
            .active_sessions
            .iter()
            .filter(|s| s.last_attached > 0)
            .collect();
        recent.sort_by_key(|s| std::cmp::Reverse(s.last_attached));
        recent.truncate(n);
        recent
    }

    pub fn session_tree_path(&self, session_name: &str) -> Option<Vec<String>> {
        let session = self
            .active_sessions
            .iter()
            .find(|s| s.tmux_session_name == session_name)?;
        for col in &self.collections {
            for thread in &col.threads {
                if thread.id == session.thread_id {
                    return if col.is_root {
                        Some(vec![thread.id.to_string(), session_name.to_string()])
                    } else {
                        Some(vec![
                            col.id.to_string(),
                            thread.id.to_string(),
                            session_name.to_string(),
                        ])
                    };
                }
            }
        }
        None
    }

    pub fn find_root_collection_idx(&self) -> Option<usize> {
        self.collections.iter().position(|c| c.is_root)
    }

    pub fn find_root_thread_by_uuid(&self, uuid_str: &str) -> Option<(usize, usize)> {
        let id: Uuid = uuid_str.parse().ok()?;
        let col_idx = self.find_root_collection_idx()?;
        let thread_idx = self.collections[col_idx]
            .threads
            .iter()
            .position(|t| t.id == id)?;
        Some((col_idx, thread_idx))
    }

    pub fn ensure_root_collection(&mut self) -> usize {
        if let Some(idx) = self.find_root_collection_idx() {
            idx
        } else {
            self.collections.push(Collection::new_root());
            self.collections.len() - 1
        }
    }

    pub fn ensure_general_thread(&mut self) -> (usize, usize) {
        let col_idx = self.ensure_root_collection();
        if let Some(thread_idx) = self.collections[col_idx]
            .threads
            .iter()
            .position(|t| t.name == "general")
        {
            (col_idx, thread_idx)
        } else {
            self.collections[col_idx]
                .threads
                .push(Thread::new("general"));
            (col_idx, self.collections[col_idx].threads.len() - 1)
        }
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

    pub fn all_agents_flat(&self) -> Vec<FlatAgent> {
        let mut result = Vec::new();
        for (col_idx, col) in self.collections.iter().enumerate() {
            for (thread_idx, thread) in col.threads.iter().enumerate() {
                let sessions = self.sessions_for_thread(thread.id);
                for (sess_idx, session) in sessions.iter().enumerate() {
                    let agents = self.agents_for_session(&session.tmux_session_name);
                    for (agent_idx, agent) in agents.iter().enumerate() {
                        result.push(FlatAgent {
                            col_idx,
                            thread_idx,
                            sess_idx,
                            agent_idx,
                            thread_name: thread.name.clone(),
                            session_display_name: session.display_name.clone(),
                            agent_type: agent.agent_type,
                            agent_display_name: agent.display_name.clone(),
                            tmux_session_name: agent.tmux_session_name.clone(),
                            window_index: agent.window_index,
                            pane_id: agent.pane_id.clone(),
                            pin_slot: agent.pin_slot,
                            status: agent.status,
                        });
                    }
                }
            }
        }
        // Pinned agents first, by slot ascending; unpinned keep original tree order.
        result.sort_by_key(|a| match a.pin_slot {
            Some(slot) => (0u8, slot),
            None => (1u8, 0u8),
        });
        result
    }
}

#[cfg(test)]
impl AppState {
    pub fn new() -> Self {
        Self {
            collections: Vec::new(),
            active_sessions: Vec::new(),
            agent_sessions: Vec::new(),
        }
    }

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
            agent_sessions: Vec::new(),
        }
    }

    pub fn session_prefix_for(&self, col_idx: usize, thread_idx: usize) -> Option<String> {
        let col = self.collections.get(col_idx)?;
        let thread = col.threads.get(thread_idx)?;
        if col.is_root {
            Some(tmux_root_session_prefix(&thread.name))
        } else {
            Some(tmux_session_prefix(&col.name, &thread.name))
        }
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

        let sess_name = "tws_work_edge-device-pipeline_hotfix".to_string();
        match state.resolve_selection(&[col_id, thread_id, sess_name]) {
            SelectedItem::Session(_, _, sess_idx) => assert_eq!(sess_idx, 1),
            _ => panic!("expected Session"),
        }
    }

    #[test]
    fn ensure_root_collection_creates_once() {
        let mut state = AppState::new();
        let idx1 = state.ensure_root_collection();
        let idx2 = state.ensure_root_collection();
        assert_eq!(idx1, idx2);
        assert_eq!(state.collections.len(), 1);
        assert!(state.collections[idx1].is_root);
    }

    #[test]
    fn ensure_general_thread_creates_once() {
        let mut state = AppState::new();
        let (c1, t1) = state.ensure_general_thread();
        let (c2, t2) = state.ensure_general_thread();
        assert_eq!((c1, t1), (c2, t2));
        assert_eq!(state.collections[c1].threads.len(), 1);
        assert_eq!(state.collections[c1].threads[t1].name, "general");
    }

    #[test]
    fn refresh_sessions_discovers_root_sessions() {
        let mut state = AppState::new();
        let (col_idx, _) = state.ensure_general_thread();
        state.add_thread(col_idx, "scratch".into());
        let live = vec![
            ("twsr_general_quick".to_string(), 100),
            ("twsr_scratch_dev".to_string(), 200),
        ];
        state.refresh_sessions(&live);
        assert_eq!(state.active_sessions.len(), 2);
        assert_eq!(state.active_sessions[0].display_name, "quick");
        assert_eq!(state.active_sessions[1].display_name, "dev");
    }

    #[test]
    fn resolve_root_thread_selection() {
        let mut state = AppState::new();
        state.ensure_general_thread();
        let thread_id = state.collections[0].threads[0].id.to_string();
        match state.resolve_selection(&[thread_id]) {
            SelectedItem::Thread(col_idx, thread_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(thread_idx, 0);
            }
            _ => panic!("expected Thread"),
        }
    }

    #[test]
    fn resolve_root_session_selection() {
        let mut state = AppState::new();
        state.ensure_general_thread();
        let live = vec![("twsr_general_quick".to_string(), 100)];
        state.refresh_sessions(&live);

        let thread_id = state.collections[0].threads[0].id.to_string();
        let sess_name = "twsr_general_quick".to_string();
        match state.resolve_selection(&[thread_id, sess_name]) {
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(thread_idx, 0);
                assert_eq!(sess_idx, 0);
            }
            _ => panic!("expected Session"),
        }
    }

    #[test]
    fn make_session_name_root() {
        let mut state = AppState::new();
        state.ensure_general_thread();
        let name = state.make_session_name(0, 0, "bugfix").unwrap();
        assert_eq!(name, "twsr_general_bugfix");
    }

    #[test]
    fn resolve_thread_path_root() {
        let mut state = AppState::new();
        state.ensure_general_thread();
        let thread_id = state.collections[0].threads[0].id;
        let (col_name, thread_name) = state.resolve_thread_path(thread_id).unwrap();
        assert!(col_name.is_none());
        assert_eq!(thread_name, "general");
    }

    #[test]
    fn session_display_path_regular() {
        let mut state = AppState::with_sample_data();
        let live = vec![("tws_work_edge-device-pipeline_bugfix".to_string(), 0)];
        state.refresh_sessions(&live);
        let path = state
            .session_display_path(&state.active_sessions[0])
            .unwrap();
        assert_eq!(path, "Work/Edge Device Pipeline/bugfix");
    }

    #[test]
    fn session_display_path_root() {
        let mut state = AppState::new();
        state.ensure_general_thread();
        let live = vec![("twsr_general_quick".to_string(), 0)];
        state.refresh_sessions(&live);
        let path = state
            .session_display_path(&state.active_sessions[0])
            .unwrap();
        assert_eq!(path, "general/quick");
    }

    #[test]
    fn resolve_selection_prefers_collection_over_root_thread() {
        let mut state = AppState::with_sample_data();
        state.ensure_general_thread();

        let col_id = state.collections[0].id.to_string();
        match state.resolve_selection(&[col_id]) {
            SelectedItem::Collection(idx) => assert_eq!(idx, 0),
            _ => panic!("expected Collection"),
        }
    }

    #[test]
    fn resolve_selection_prefers_regular_thread_over_root_session() {
        let mut state = AppState::with_sample_data();
        state.ensure_general_thread();
        let live = vec![("twsr_general_quick".to_string(), 0)];
        state.refresh_sessions(&live);

        let col_id = state.collections[0].id.to_string();
        let thread_id = state.collections[0].threads[0].id.to_string();
        match state.resolve_selection(&[col_id, thread_id]) {
            SelectedItem::Thread(col_idx, thread_idx) => {
                assert_eq!(col_idx, 0);
                assert_eq!(thread_idx, 0);
                assert_eq!(state.collections[col_idx].name, "Work");
            }
            _ => panic!("expected Thread"),
        }
    }

    #[test]
    fn refresh_sessions_ignores_bare_root_prefix() {
        let mut state = AppState::new();
        state.ensure_general_thread();
        let live = vec![("twsr_general".to_string(), 0)];
        state.refresh_sessions(&live);
        assert!(state.active_sessions.is_empty());
    }

    fn make_agent(pane_id: &str) -> super::AgentSession {
        use super::super::model::{AgentSession, AgentStatus, AgentType};
        AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: "tws_x_y_a".into(),
            window_index: 0,
            pane_id: pane_id.into(),
            display_name: "claude".into(),
            renamed: false,
            pin_slot: None,
            status: AgentStatus::Unknown,
            status_since: 0,
        }
    }

    #[test]
    fn pin_agent_auto_returns_slot_zero_on_first_pin() {
        let mut state = AppState::new();
        state.agent_sessions.push(make_agent("%1"));
        assert_eq!(state.pin_agent_auto("%1"), Some(0));
        assert_eq!(state.agent_sessions[0].pin_slot, Some(0));
    }

    #[test]
    fn pin_agent_to_empty_slot_assigns() {
        let mut state = AppState::new();
        state.agent_sessions.push(make_agent("%1"));
        state.pin_agent_to("%1", 3);
        assert_eq!(state.agent_sessions[0].pin_slot, Some(3));
    }

    #[test]
    fn pin_agent_to_same_slot_is_noop() {
        let mut state = AppState::new();
        let mut a = make_agent("%1");
        a.pin_slot = Some(3);
        state.agent_sessions.push(a);
        state.pin_agent_to("%1", 3);
        assert_eq!(state.agent_sessions[0].pin_slot, Some(3));
    }

    #[test]
    fn pin_agent_to_both_pinned_swaps_slots() {
        let mut state = AppState::new();
        let mut a = make_agent("%1");
        a.pin_slot = Some(2);
        let mut b = make_agent("%2");
        b.pin_slot = Some(5);
        state.agent_sessions.push(a);
        state.agent_sessions.push(b);

        state.pin_agent_to("%2", 2);
        assert_eq!(state.agent_sessions[0].pin_slot, Some(5));
        assert_eq!(state.agent_sessions[1].pin_slot, Some(2));
    }

    #[test]
    fn pin_agent_to_unpinned_into_occupied_re_auto_pins_occupant() {
        let mut state = AppState::new();
        for (i, slot) in [(1u32, 0u8), (2, 1), (3, 3)] {
            let mut a = make_agent(&format!("%{}", i));
            a.pin_slot = Some(slot);
            state.agent_sessions.push(a);
        }
        state.agent_sessions.push(make_agent("%4"));
        state.pin_agent_to("%4", 1);

        let by_id = |id: &str| {
            state
                .agent_sessions
                .iter()
                .find(|a| a.pane_id == id)
                .unwrap()
                .pin_slot
        };
        assert_eq!(by_id("%4"), Some(1));
        assert_eq!(by_id("%2"), Some(2));
    }

    #[test]
    fn unpin_agent_clears_slot() {
        let mut state = AppState::new();
        let mut a = make_agent("%1");
        a.pin_slot = Some(3);
        state.agent_sessions.push(a);
        state.unpin_agent("%1");
        assert_eq!(state.agent_sessions[0].pin_slot, None);
    }

    #[test]
    fn unpin_agent_noop_when_not_pinned() {
        let mut state = AppState::new();
        state.agent_sessions.push(make_agent("%1"));
        state.unpin_agent("%1");
        assert_eq!(state.agent_sessions[0].pin_slot, None);
    }

    #[test]
    fn pin_agent_auto_picks_lowest_free_slot() {
        let mut state = AppState::new();
        state.agent_sessions.push(make_agent("%1"));
        state.agent_sessions.push(make_agent("%2"));
        state.agent_sessions.push(make_agent("%3"));
        state.agent_sessions[0].pin_slot = Some(0);
        state.agent_sessions[1].pin_slot = Some(1);
        state.agent_sessions[2].pin_slot = Some(3);
        state.agent_sessions.push(make_agent("%4"));
        assert_eq!(state.pin_agent_auto("%4"), Some(2));
    }

    #[test]
    fn pin_agent_auto_returns_none_when_full() {
        let mut state = AppState::new();
        for i in 0..10 {
            let mut a = make_agent(&format!("%{}", i));
            a.pin_slot = Some(i as u8);
            state.agent_sessions.push(a);
        }
        state.agent_sessions.push(make_agent("%11"));
        assert_eq!(state.pin_agent_auto("%11"), None);
        assert_eq!(state.agent_sessions.last().unwrap().pin_slot, None);
    }

    #[test]
    fn pin_agent_auto_idempotent_for_already_pinned() {
        let mut state = AppState::new();
        let mut a = make_agent("%1");
        a.pin_slot = Some(5);
        state.agent_sessions.push(a);
        assert_eq!(state.pin_agent_auto("%1"), Some(5));
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
        assert_eq!(recent[0].display_name, "hotfix");
        assert_eq!(recent[1].display_name, "main");
        assert_eq!(recent[2].display_name, "bugfix");

        let recent2 = state.recent_sessions(2);
        assert_eq!(recent2.len(), 2);
        assert_eq!(recent2[0].display_name, "hotfix");
        assert_eq!(recent2[1].display_name, "main");
    }

    #[test]
    fn all_agents_flat_sorts_pinned_first_with_gaps_preserved() {
        let mut state = AppState::with_sample_data();
        let live = vec![("tws_work_edge-device-pipeline_one".to_string(), 0)];
        state.refresh_sessions(&live);

        use crate::core::model::{AgentSession, AgentStatus, AgentType};
        let mk = |id: &str, slot: Option<u8>| AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: "tws_work_edge-device-pipeline_one".into(),
            window_index: 0,
            pane_id: id.into(),
            display_name: id.into(),
            renamed: false,
            pin_slot: slot,
            status: AgentStatus::Unknown,
            status_since: 0,
        };
        state.agent_sessions.push(mk("%a", None));
        state.agent_sessions.push(mk("%b", Some(3)));
        state.agent_sessions.push(mk("%c", Some(0)));
        state.agent_sessions.push(mk("%d", None));

        let flat = state.all_agents_flat();
        let ids: Vec<&str> = flat.iter().map(|f| f.pane_id.as_str()).collect();
        assert_eq!(ids, vec!["%c", "%b", "%a", "%d"]);
    }

    #[test]
    fn pin_slot_survives_agent_list_rebuild() {
        use crate::core::model::{AgentSession, AgentStatus, AgentType};
        let mut state = AppState::new();
        state.agent_sessions.push(AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: "tws_x_y_a".into(),
            window_index: 0,
            pane_id: "%1".into(),
            display_name: "claude".into(),
            renamed: false,
            pin_slot: Some(2),
            status: AgentStatus::Unknown,
            status_since: 0,
        });

        let saved_pin = state
            .agent_sessions
            .iter()
            .find(|a| a.pane_id == "%1")
            .and_then(|a| a.pin_slot);

        state.agent_sessions.clear();
        state.agent_sessions.push(AgentSession {
            agent_type: AgentType::ClaudeCode,
            tmux_session_name: "tws_x_y_a".into(),
            window_index: 0,
            pane_id: "%1".into(),
            display_name: "claude".into(),
            renamed: false,
            pin_slot: None,
            status: AgentStatus::Unknown,
            status_since: 0,
        });

        if let Some(agent) = state.agent_sessions.iter_mut().find(|a| a.pane_id == "%1") {
            agent.pin_slot = saved_pin;
        }

        assert_eq!(state.agent_sessions[0].pin_slot, Some(2));
    }

    #[test]
    fn set_and_clear_thread_working_dir() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_thread(0, "Pipeline".into());

        let dir = std::path::PathBuf::from("/tmp/pipeline");
        state.set_thread_working_dir(0, 0, Some(dir.clone()));
        assert_eq!(state.collections[0].threads[0].working_dir, Some(dir));

        state.set_thread_working_dir(0, 0, None);
        assert!(state.collections[0].threads[0].working_dir.is_none());
    }

    #[test]
    fn set_thread_working_dir_ignores_out_of_range_indices() {
        let mut state = AppState::new();
        state.add_collection("Work".into());
        state.add_thread(0, "Pipeline".into());
        state.set_thread_working_dir(9, 9, Some(std::path::PathBuf::from("/tmp")));
        assert!(state.collections[0].threads[0].working_dir.is_none());
    }
}
