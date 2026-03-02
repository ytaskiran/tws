use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::widgets::Block;
use tui_tree_widget::{Tree, TreeState};

use crate::components::{confirm_modal, input_modal, tree_view};
use crate::core::persistence;
use crate::core::state::{AppState, SelectedItem};
use crate::event;
use crate::theme;
use crate::tmux::commands as tmux;
use crate::tui::{self, Tui};

/// What the input modal is being used for.
enum InputPurpose {
    AddCollection,
    AddProject { collection_idx: usize },
    RenameCollection { idx: usize },
    RenameProject { col_idx: usize, proj_idx: usize },
    NewSession { col_idx: usize, proj_idx: usize },
}

/// What the confirm modal is confirming.
enum ConfirmPurpose {
    DeleteCollection { idx: usize, name: String },
    DeleteProject { col_idx: usize, proj_idx: usize, name: String },
    KillSession { session_name: String },
    KillAllSessions { col_idx: usize, proj_idx: usize, project_name: String },
}

enum Mode {
    Normal,
    Input {
        purpose: InputPurpose,
        buffer: String,
    },
    Confirm {
        purpose: ConfirmPurpose,
    },
}

pub struct App {
    pub state: AppState,
    pub tree_state: TreeState<String>,
    pub running: bool,
    mode: Mode,
    last_refresh: Instant,
}

/// How often to poll tmux for session changes (seconds).
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

impl App {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tree_state: TreeState::default(),
            running: true,
            mode: Mode::Normal,
            last_refresh: Instant::now(),
        }
    }

    pub fn run(&mut self, terminal: &mut Tui) -> std::io::Result<()> {
        // Initial session refresh
        self.do_refresh_sessions();

        while self.running {
            // Periodic session refresh
            if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
                self.do_refresh_sessions();
            }

            self.draw(terminal)?;
            if let Some(key) = event::poll_key(Duration::from_millis(250))? {
                // Ctrl+C always quits
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('c')
                {
                    self.running = false;
                    continue;
                }

                match &self.mode {
                    Mode::Normal => self.handle_normal_key(key.code, terminal)?,
                    Mode::Input { .. } => self.handle_input_key(key.code, terminal)?,
                    Mode::Confirm { .. } => self.handle_confirm_key(key.code),
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self, terminal: &mut Tui) -> std::io::Result<()> {
        terminal.draw(|frame| {
            let area = frame.area();

            // Always draw the tree
            let items = tree_view::build_tree_items(&self.state);
            let tree = Tree::new(&items)
                .expect("collection IDs are unique")
                .block(
                    Block::bordered()
                        .title(" tws ")
                        .title_style(theme::TITLE_STYLE)
                        .border_style(theme::BORDER_STYLE),
                )
                .highlight_style(theme::HIGHLIGHT_STYLE)
                .highlight_symbol("▶ ")
                .node_closed_symbol("▸ ")
                .node_open_symbol("▾ ")
                .node_no_children_symbol("  ");

            frame.render_stateful_widget(tree, area, &mut self.tree_state);

            // Draw modal overlay if active
            match &self.mode {
                Mode::Normal => {}
                Mode::Input { purpose, buffer } => {
                    let title = match purpose {
                        InputPurpose::AddCollection => "New Collection",
                        InputPurpose::AddProject { .. } => "New Project",
                        InputPurpose::RenameCollection { .. } => "Rename Collection",
                        InputPurpose::RenameProject { .. } => "Rename Project",
                        InputPurpose::NewSession { .. } => "Session Name",
                    };
                    input_modal::render(frame, title, buffer, area);
                }
                Mode::Confirm { purpose } => {
                    let message = match purpose {
                        ConfirmPurpose::DeleteCollection { name, .. } => {
                            format!("Delete collection \"{}\"?", name)
                        }
                        ConfirmPurpose::DeleteProject { name, .. } => {
                            format!("Delete project \"{}\"?", name)
                        }
                        ConfirmPurpose::KillSession { session_name } => {
                            format!("Kill session \"{}\"?", session_name)
                        }
                        ConfirmPurpose::KillAllSessions { project_name, .. } => {
                            format!("Kill all sessions for \"{}\"?", project_name)
                        }
                    };
                    confirm_modal::render(frame, &message, area);
                }
            }
        })?;
        Ok(())
    }

    fn handle_normal_key(&mut self, code: KeyCode, terminal: &mut Tui) -> std::io::Result<()> {
        match code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => {
                self.tree_state.key_down();
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.tree_state.key_up();
            }
            KeyCode::Char('h') | KeyCode::Left => {
                self.tree_state.key_left();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.tree_state.key_right();
            }
            KeyCode::Char(' ') => {
                self.tree_state.toggle_selected();
            }
            KeyCode::Enter => {
                let selected = self.state.resolve_selection(self.tree_state.selected());
                match selected {
                    SelectedItem::Collection(..) => {
                        self.tree_state.toggle_selected();
                    }
                    SelectedItem::Project(col_idx, proj_idx) => {
                        self.mode = Mode::Input {
                            purpose: InputPurpose::NewSession { col_idx, proj_idx },
                            buffer: String::new(),
                        };
                    }
                    SelectedItem::Session(col_idx, proj_idx, sess_idx) => {
                        let sessions = self.state.sessions_for_project(
                            self.state.collections[col_idx].projects[proj_idx].id,
                        );
                        if let Some(session) = sessions.get(sess_idx) {
                            let name = session.tmux_session_name.clone();
                            self.attach_to_session(&name, terminal)?;
                        }
                    }
                    SelectedItem::None => {}
                }
            }
            KeyCode::Char('a') => self.start_add(),
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('d') => self.start_delete(),
            KeyCode::Char('x') => self.start_kill_session(),
            _ => {}
        }
        Ok(())
    }

    fn handle_input_key(&mut self, code: KeyCode, terminal: &mut Tui) -> std::io::Result<()> {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            KeyCode::Enter => {
                self.confirm_input(terminal)?;
            }
            KeyCode::Backspace => {
                if let Mode::Input { buffer, .. } = &mut self.mode {
                    buffer.pop();
                }
            }
            KeyCode::Char(c) => {
                if let Mode::Input { buffer, .. } = &mut self.mode {
                    buffer.push(c);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_confirm_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') | KeyCode::Enter => {
                self.execute_confirm();
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    fn start_add(&mut self) {
        let selected = self.state.resolve_selection(self.tree_state.selected());
        let purpose = match selected {
            SelectedItem::Collection(idx) | SelectedItem::Project(idx, _) | SelectedItem::Session(idx, _, _) => {
                InputPurpose::AddProject {
                    collection_idx: idx,
                }
            }
            SelectedItem::None => InputPurpose::AddCollection,
        };
        self.mode = Mode::Input {
            purpose,
            buffer: String::new(),
        };
    }

    fn start_rename(&mut self) {
        let selected = self.state.resolve_selection(self.tree_state.selected());
        let current_name = match self.state.selected_name(&selected) {
            Some(name) => name,
            None => return,
        };
        let purpose = match selected {
            SelectedItem::Collection(idx) => InputPurpose::RenameCollection { idx },
            SelectedItem::Project(col_idx, proj_idx) => {
                InputPurpose::RenameProject { col_idx, proj_idx }
            }
            // Can't rename sessions
            SelectedItem::Session(..) | SelectedItem::None => return,
        };
        self.mode = Mode::Input {
            purpose,
            buffer: current_name,
        };
    }

    fn start_delete(&mut self) {
        let selected = self.state.resolve_selection(self.tree_state.selected());
        let purpose = match &selected {
            SelectedItem::Collection(idx) => {
                let name = self.state.collections[*idx].name.clone();
                ConfirmPurpose::DeleteCollection { idx: *idx, name }
            }
            SelectedItem::Project(col_idx, proj_idx) => {
                let name = self.state.collections[*col_idx].projects[*proj_idx]
                    .name
                    .clone();
                ConfirmPurpose::DeleteProject {
                    col_idx: *col_idx,
                    proj_idx: *proj_idx,
                    name,
                }
            }
            // Use 'x' to kill sessions, not 'd'
            SelectedItem::Session(..) | SelectedItem::None => return,
        };
        self.mode = Mode::Confirm { purpose };
    }

    fn start_kill_session(&mut self) {
        let selected = self.state.resolve_selection(self.tree_state.selected());
        match selected {
            SelectedItem::Session(col_idx, proj_idx, sess_idx) => {
                let proj_id = self.state.collections[col_idx].projects[proj_idx].id;
                let sessions = self.state.sessions_for_project(proj_id);
                if let Some(session) = sessions.get(sess_idx) {
                    let name = session.tmux_session_name.clone();
                    self.mode = Mode::Confirm {
                        purpose: ConfirmPurpose::KillSession { session_name: name },
                    };
                }
            }
            SelectedItem::Project(col_idx, proj_idx) => {
                // If the project has active sessions, offer to kill all of them
                if self.state.has_active_session(col_idx, proj_idx) {
                    let project_name = self.state.collections[col_idx].projects[proj_idx]
                        .name
                        .clone();
                    self.mode = Mode::Confirm {
                        purpose: ConfirmPurpose::KillAllSessions {
                            col_idx,
                            proj_idx,
                            project_name,
                        },
                    };
                }
            }
            _ => {}
        }
    }

    fn confirm_input(&mut self, terminal: &mut Tui) -> std::io::Result<()> {
        // Take ownership of the mode to extract buffer and purpose
        let old_mode = std::mem::replace(&mut self.mode, Mode::Normal);
        if let Mode::Input { purpose, buffer } = old_mode {
            let trimmed = buffer.trim().to_string();
            if trimmed.is_empty() {
                return Ok(());
            }
            match purpose {
                InputPurpose::AddCollection => {
                    self.state.add_collection(trimmed);
                    self.save_state();
                }
                InputPurpose::AddProject { collection_idx } => {
                    self.state.add_project(collection_idx, trimmed);
                    // Auto-expand the collection so the new project is visible
                    let col_id = self.state.collections[collection_idx].id.to_string();
                    self.tree_state.open(vec![col_id]);
                    self.save_state();
                }
                InputPurpose::RenameCollection { idx } => {
                    self.state.rename_collection(idx, trimmed);
                    self.save_state();
                }
                InputPurpose::RenameProject { col_idx, proj_idx } => {
                    self.state.rename_project(col_idx, proj_idx, trimmed);
                    self.save_state();
                }
                InputPurpose::NewSession { col_idx, proj_idx } => {
                    if let Some(session_name) = self.state.make_session_name(col_idx, proj_idx, &trimmed) {
                        self.launch_session(&session_name, terminal)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn execute_confirm(&mut self) {
        let old_mode = std::mem::replace(&mut self.mode, Mode::Normal);
        if let Mode::Confirm { purpose } = old_mode {
            match purpose {
                ConfirmPurpose::DeleteCollection { idx, .. } => {
                    self.state.delete_collection(idx);
                    self.tree_state.select_first();
                    self.save_state();
                }
                ConfirmPurpose::DeleteProject { col_idx, proj_idx, .. } => {
                    self.state.delete_project(col_idx, proj_idx);
                    self.tree_state.select_first();
                    self.save_state();
                }
                ConfirmPurpose::KillSession { session_name } => {
                    let _ = tmux::kill_session(&session_name);
                    self.do_refresh_sessions();
                }
                ConfirmPurpose::KillAllSessions { col_idx, proj_idx, .. } => {
                    let proj_id = self.state.collections[col_idx].projects[proj_idx].id;
                    let names: Vec<String> = self
                        .state
                        .sessions_for_project(proj_id)
                        .iter()
                        .map(|s| s.tmux_session_name.clone())
                        .collect();
                    for name in names {
                        let _ = tmux::kill_session(&name);
                    }
                    self.do_refresh_sessions();
                }
            }
        }
    }

    /// Launch a new tmux session with the given name and attach to it.
    fn launch_session(&mut self, session_name: &str, terminal: &mut Tui) -> std::io::Result<()> {
        tmux::new_session(session_name)?;
        self.attach_to_session(session_name, terminal)
    }

    /// Attach or switch to a tmux session by name.
    fn attach_to_session(&mut self, session_name: &str, terminal: &mut Tui) -> std::io::Result<()> {
        if tmux::is_inside_tmux() {
            // Inside tmux: switch-client is non-blocking, TUI keeps running
            let _ = tmux::switch_client(session_name);
        } else {
            // Outside tmux: suspend TUI, attach (blocks), then resume TUI
            tui::restore()?;
            let _ = tmux::attach_session(session_name);
            *terminal = tui::init()?;
        }

        // Refresh sessions immediately after attach/switch
        self.do_refresh_sessions();
        Ok(())
    }

    fn do_refresh_sessions(&mut self) {
        let live = tmux::list_tws_sessions();
        self.state.refresh_sessions(&live);
        self.last_refresh = Instant::now();
    }

    fn save_state(&self) {
        if let Err(e) = persistence::save(&self.state.collections) {
            eprintln!("Failed to save state: {}", e);
        }
    }
}
