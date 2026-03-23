use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use tui_tree_widget::{Tree, TreeState};

use crate::components::status_bar::{self, StatusContext};
use crate::components::{confirm_modal, finder_modal, input_modal, notes_sidebar, recent_bar, tree_view};
use crate::core::notes::{NoteEditor, NoteStore};
use crate::core::persistence;
use crate::core::state::{AppState, SelectedItem};
use crate::event;
use crate::theme;
use crate::tmux::commands as tmux;
use crate::tui::{self, Tui};

/// What the input modal is being used for.
enum InputPurpose {
    AddCollection,
    AddThread { collection_idx: usize },
    RenameCollection { idx: usize },
    RenameThread { col_idx: usize, thread_idx: usize },
    NewSession { col_idx: usize, thread_idx: usize },
    RenameSession {
        col_idx: usize,
        thread_idx: usize,
        old_tmux_name: String,
    },
}

/// What the confirm modal is confirming.
enum ConfirmPurpose {
    DeleteCollection { idx: usize, name: String },
    DeleteThread { col_idx: usize, thread_idx: usize, name: String },
    KillSession { session_name: String },
    KillAllSessions { col_idx: usize, thread_idx: usize, thread_name: String },
}

struct FinderState {
    query: String,
    /// (tmux_session_name, "Collection/Thread/session_label"), sorted by recency.
    all_entries: Vec<(String, String)>,
    /// Indices into all_entries matching current query.
    filtered: Vec<usize>,
    /// Cursor position within filtered.
    cursor: usize,
}

impl FinderState {
    fn new(entries: Vec<(String, String)>) -> Self {
        let filtered = (0..entries.len()).collect();
        Self {
            query: String::new(),
            all_entries: entries,
            filtered,
            cursor: 0,
        }
    }

    fn update_filter(&mut self) {
        let q = self.query.to_lowercase();
        self.filtered = if q.is_empty() {
            (0..self.all_entries.len()).collect()
        } else {
            self.all_entries
                .iter()
                .enumerate()
                .filter(|(_, (_, path))| path.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect()
        };
        if self.cursor >= self.filtered.len() {
            self.cursor = self.filtered.len().saturating_sub(1);
        }
    }
}

/// Which pane has keyboard focus during normal mode.
enum Focus {
    Tree,
    Notes,
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
    Finder {
        state: FinderState,
    },
}

pub struct App {
    pub state: AppState,
    pub tree_state: TreeState<String>,
    pub running: bool,
    mode: Mode,
    focus: Focus,
    notes: NoteStore,
    note_editor: NoteEditor,
    last_refresh: Instant,
    flash: Option<(String, Instant)>,
}

/// How often to poll tmux for session changes (seconds).
const REFRESH_INTERVAL: Duration = Duration::from_secs(30);

impl App {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            tree_state: TreeState::default(),
            running: true,
            mode: Mode::Normal,
            focus: Focus::Tree,
            notes: NoteStore::new(),
            note_editor: NoteEditor::new(),
            last_refresh: Instant::now(),
            flash: None,
        }
    }

    fn set_flash(&mut self, msg: &str) {
        self.flash = Some((msg.to_string(), Instant::now()));
    }

    pub fn run(&mut self, terminal: &mut Tui, ui_state: persistence::UiState) -> std::io::Result<()> {
        // Initial session refresh (must run first so session children exist in the tree)
        self.do_refresh_sessions();

        // Restore expansion state
        for path in ui_state.open_nodes {
            self.tree_state.open(path);
        }
        // Restore last selection
        if let Some(sel) = ui_state.selected {
            self.tree_state.select(sel);
        }

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
                    Mode::Normal => {
                        self.handle_normal_mode(key.code, key.modifiers, terminal)?;
                    }
                    Mode::Input { .. } => self.handle_input_key(key.code, terminal)?,
                    Mode::Confirm { .. } => self.handle_confirm_key(key.code),
                    Mode::Finder { .. } => {
                        self.handle_finder_key(key.code, key.modifiers, terminal)?
                    }
                }
            }
        }
        self.flush_note_editor();
        self.save_ui_state();
        Ok(())
    }

    fn draw(&mut self, terminal: &mut Tui) -> std::io::Result<()> {
        // Compute flash outside the closure — we need &mut self to expire it,
        // but the closure also borrows self mutably via render_stateful_widget.
        let flash_msg: Option<String> = match &self.flash {
            Some((msg, t)) if t.elapsed() < Duration::from_secs(2) => Some(msg.clone()),
            Some(_) => {
                self.flash = None;
                None
            }
            None => None,
        };

        // Pre-compute recent sessions data outside the closure for readability.
        // (Only flash_msg *must* be outside — it mutates self.flash on expiry.)
        // Only show the bar in Normal mode when there are recent sessions.
        let is_normal = matches!(self.mode, Mode::Normal);
        let recent_data: Vec<(String, String)> = if is_normal {
            self.state
                .recent_sessions(5)
                .iter()
                .filter_map(|s| {
                    let path = self.state.session_display_path(s)?;
                    Some((s.tmux_session_name.clone(), path))
                })
                .collect()
        } else {
            Vec::new()
        };
        let recent_count = recent_data.len() as u16;
        let show_recent = !recent_data.is_empty();

        // Pre-compute sidebar data: resolve which note to display.
        let selected_item = self.state.resolve_selection(self.tree_state.selected());
        let sidebar_info: Option<String> = match &selected_item {
            SelectedItem::None => None,
            SelectedItem::Collection(idx) => {
                Some(self.state.collections[*idx].name.clone())
            }
            SelectedItem::Thread(col_idx, thread_idx) => {
                Some(self.state.collections[*col_idx].threads[*thread_idx].name.clone())
            }
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let thread = &self.state.collections[*col_idx].threads[*thread_idx];
                let sessions = self.state.sessions_for_thread(thread.id);
                sessions.get(*sess_idx).map(|s| s.display_name.clone())
            }
        };
        let show_sidebar = sidebar_info.is_some() && is_normal;
        let sidebar_title = sidebar_info.unwrap_or_default();
        let notes_focused = matches!(self.focus, Focus::Notes);

        // Approximate visible height; actual sidebar size isn't known until layout
        self.note_editor.ensure_visible(20);

        // Clone editor lines to avoid borrow conflicts (closure needs &mut self.tree_state)
        let editor_lines = self.note_editor.lines.clone();
        let editor_cursor = (self.note_editor.cursor_row, self.note_editor.cursor_col);
        let editor_scroll = self.note_editor.scroll_offset;
        let editor_is_empty = self.note_editor.is_empty();
        let editor_has_target = self.note_editor.target_key.is_some();

        terminal.draw(|frame| {
            let area = frame.area();

            // Build layout: tree, [separator, recent bar], separator, status bar
            let constraints = if show_recent {
                vec![
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(recent_count),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ]
            } else {
                vec![
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ]
            };
            let chunks = Layout::vertical(constraints).split(area);

            // Index variables for separator and status bar positions
            let (recent_sep_idx, recent_idx, sep_idx, status_idx) = if show_recent {
                (Some(1), Some(2), 3, 4)
            } else {
                (None, None, 1, 2)
            };

            // Split content area horizontally if sidebar should show
            let content_area = chunks[0];
            let (tree_area, sidebar_area) = if show_sidebar {
                let horiz = Layout::horizontal([
                    Constraint::Percentage(60),
                    Constraint::Percentage(40),
                ])
                .split(content_area);
                (horiz[0], Some(horiz[1]))
            } else {
                (content_area, None)
            };

            // Tree area or empty state
            let block = Block::default();

            let items = tree_view::build_tree_items(&self.state);
            if items.is_empty() {
                let available_height = tree_area.height.saturating_sub(2);
                let content_height = 4u16;
                let top_padding = (available_height.saturating_sub(content_height)) / 2;

                let mut lines: Vec<Line> = vec![Line::from(""); top_padding as usize];
                lines.push(Line::from(""));
                lines.push(Line::from(vec![
                    Span::raw("Welcome to "),
                    Span::styled("tws", theme::EMPTY_TITLE_STYLE),
                ]));
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    "Press Enter for a quick session, or a to add a thread.",
                    theme::EMPTY_HINT_STYLE,
                )));

                let paragraph = Paragraph::new(lines)
                    .block(block)
                    .alignment(Alignment::Center);
                frame.render_widget(paragraph, tree_area);
            } else {
                let tree = Tree::new(&items)
                    .expect("collection IDs are unique")
                    .block(block)
                    .highlight_style(theme::HIGHLIGHT_STYLE)
                    .highlight_symbol("  ")
                    .node_closed_symbol("\u{203A} ")
                    .node_open_symbol("\u{2304} ")
                    .node_no_children_symbol("  ");

                frame.render_stateful_widget(tree, tree_area, &mut self.tree_state);
            }

            // Notes sidebar
            if let Some(sb_area) = sidebar_area {
                let title = format!("Notes: {}", sidebar_title);
                notes_sidebar::render(
                    frame,
                    &editor_lines,
                    editor_cursor,
                    editor_scroll,
                    editor_is_empty,
                    &title,
                    notes_focused && editor_has_target,
                    sb_area,
                );
            }

            // Separator between tree and recent bar
            if let Some(idx) = recent_sep_idx {
                let sep = "\u{2500}".repeat(chunks[idx].width as usize);
                frame.render_widget(
                    Paragraph::new(Line::styled(sep, theme::SEPARATOR_STYLE)),
                    chunks[idx],
                );
            }

            // Recent sessions bar (only in Normal mode with active sessions)
            if let Some(idx) = recent_idx {
                recent_bar::render(frame, &recent_data, chunks[idx]);
            }

            // Separator line
            let separator = "\u{2500}".repeat(chunks[sep_idx].width as usize);
            frame.render_widget(
                Paragraph::new(Line::styled(separator, theme::SEPARATOR_STYLE)),
                chunks[sep_idx],
            );

            // Status bar
            let active_count = self.state.active_sessions.len();
            let status_ctx = self.status_context(&selected_item);
            status_bar::render(frame, status_ctx, chunks[status_idx], active_count, flash_msg.as_deref());

            // Draw modal overlay if active (over full area so it centers properly)
            match &self.mode {
                Mode::Normal => {}
                Mode::Input { purpose, buffer } => {
                    let title = match purpose {
                        InputPurpose::AddCollection => "New Collection",
                        InputPurpose::AddThread { .. } => "New Thread",
                        InputPurpose::RenameCollection { .. } => "Rename Collection",
                        InputPurpose::RenameThread { .. } => "Rename Thread",
                        InputPurpose::NewSession { .. } => "Session Name",
                        InputPurpose::RenameSession { .. } => "Rename Session",
                    };
                    input_modal::render(frame, title, buffer, area);
                }
                Mode::Confirm { purpose } => {
                    let message = match purpose {
                        ConfirmPurpose::DeleteCollection { name, .. } => {
                            format!("Delete collection \"{}\"?", name)
                        }
                        ConfirmPurpose::DeleteThread { name, .. } => {
                            format!("Delete thread \"{}\"?", name)
                        }
                        ConfirmPurpose::KillSession { session_name } => {
                            format!("Kill session \"{}\"?", session_name)
                        }
                        ConfirmPurpose::KillAllSessions { thread_name, .. } => {
                            format!("Kill all sessions for \"{}\"?", thread_name)
                        }
                    };
                    confirm_modal::render(frame, &message, area);
                }
                Mode::Finder { state } => {
                    finder_modal::render(
                        frame,
                        &state.query,
                        &state.all_entries,
                        &state.filtered,
                        state.cursor,
                        area,
                    );
                }
            }
        })?;
        Ok(())
    }

    /// Build a `StatusContext` from the current mode and already-resolved selection.
    fn status_context(&self, selected: &SelectedItem) -> StatusContext {
        match &self.mode {
            Mode::Input { .. } => StatusContext::Input,
            Mode::Confirm { .. } => StatusContext::Confirm,
            Mode::Finder { .. } => StatusContext::Finder,
            Mode::Normal => {
                if matches!(self.focus, Focus::Notes) {
                    return StatusContext::Notes;
                }
                match selected {
                    SelectedItem::None => StatusContext::NormalNone,
                    SelectedItem::Collection(_) => StatusContext::NormalCollection,
                    SelectedItem::Thread(_, _) => StatusContext::NormalThread,
                    SelectedItem::Session(_, _, _) => StatusContext::NormalSession,
                }
            }
        }
    }

    /// Top-level handler for Normal mode: checks focus-switching keys first,
    /// then dispatches to the tree or notes handler based on current focus.
    fn handle_normal_mode(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        terminal: &mut Tui,
    ) -> std::io::Result<()> {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);

        // Focus switching: Tab toggles, Ctrl+Arrow for directional switch
        let is_focus_switch = code == KeyCode::Tab
            || (ctrl && code == KeyCode::Left)
            || (ctrl && code == KeyCode::Right);

        if is_focus_switch {
            let wants_notes = code == KeyCode::Tab && matches!(self.focus, Focus::Tree)
                || (ctrl && code == KeyCode::Right);
            let wants_tree = code == KeyCode::Tab && matches!(self.focus, Focus::Notes)
                || (ctrl && code == KeyCode::Left);

            if wants_notes {
                let has_selection = !matches!(
                    self.state.resolve_selection(self.tree_state.selected()),
                    SelectedItem::None
                );
                if has_selection {
                    self.sync_note_editor();
                    self.focus = Focus::Notes;
                }
            } else if wants_tree {
                self.flush_note_editor();
                self.focus = Focus::Tree;
            }
            return Ok(());
        }

        match self.focus {
            Focus::Tree => {
                self.handle_normal_key(code, terminal)?;
                // After tree navigation, sync note editor if selection changed
                self.sync_note_editor();
            }
            Focus::Notes => self.handle_notes_key(code),
        }
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
            KeyCode::Enter => self.start_enter(terminal)?,
            KeyCode::Esc => {
                self.tree_state.select(Vec::new());
            }
            KeyCode::Char('a') => self.start_add(),
            KeyCode::Char('A') => {
                self.mode = Mode::Input {
                    purpose: InputPurpose::AddCollection,
                    buffer: String::new(),
                };
            }
            KeyCode::Char('r') => self.start_rename(),
            KeyCode::Char('d') => self.start_delete(),
            KeyCode::Char('x') => self.start_kill_session(),
            KeyCode::Char('/') => {
                if self.state.active_sessions.is_empty() {
                    self.set_flash("No active sessions");
                    return Ok(());
                }
                self.start_finder();
            }
            KeyCode::Char(c @ '1'..='5') => {
                let recent = self.state.recent_sessions(5);
                if let Some(session) = recent.get((c as usize) - ('1' as usize)) {
                    let name = session.tmux_session_name.clone();
                    self.attach_to_session(&name, terminal)?;
                }
            }
            KeyCode::Char('e') => self.toggle_expand_all(),
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
            SelectedItem::Collection(idx) | SelectedItem::Thread(idx, _) | SelectedItem::Session(idx, _, _) => {
                InputPurpose::AddThread {
                    collection_idx: idx,
                }
            }
            SelectedItem::None => {
                let col_idx = self.state.ensure_root_collection();
                InputPurpose::AddThread { collection_idx: col_idx }
            }
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
            SelectedItem::Thread(col_idx, thread_idx) => {
                InputPurpose::RenameThread { col_idx, thread_idx }
            }
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let thread_id = self.state.collections[col_idx].threads[thread_idx].id;
                let sessions = self.state.sessions_for_thread(thread_id);
                match sessions.get(sess_idx) {
                    Some(session) => InputPurpose::RenameSession {
                        col_idx,
                        thread_idx,
                        old_tmux_name: session.tmux_session_name.clone(),
                    },
                    None => return,
                }
            }
            SelectedItem::None => return,
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
            SelectedItem::Thread(col_idx, thread_idx) => {
                let name = self.state.collections[*col_idx].threads[*thread_idx]
                    .name
                    .clone();
                ConfirmPurpose::DeleteThread {
                    col_idx: *col_idx,
                    thread_idx: *thread_idx,
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
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let thread_id = self.state.collections[col_idx].threads[thread_idx].id;
                let sessions = self.state.sessions_for_thread(thread_id);
                if let Some(session) = sessions.get(sess_idx) {
                    let name = session.tmux_session_name.clone();
                    self.mode = Mode::Confirm {
                        purpose: ConfirmPurpose::KillSession { session_name: name },
                    };
                }
            }
            SelectedItem::Thread(col_idx, thread_idx) => {
                // If the thread has active sessions, offer to kill all of them
                if self.state.has_active_session(col_idx, thread_idx) {
                    let thread_name = self.state.collections[col_idx].threads[thread_idx]
                        .name
                        .clone();
                    self.mode = Mode::Confirm {
                        purpose: ConfirmPurpose::KillAllSessions {
                            col_idx,
                            thread_idx,
                            thread_name,
                        },
                    };
                }
            }
            _ => {}
        }
    }

    fn start_enter(&mut self, terminal: &mut Tui) -> std::io::Result<()> {
        let selected = self.state.resolve_selection(self.tree_state.selected());
        match selected {
            SelectedItem::Collection(..) => {}
            SelectedItem::Thread(col_idx, thread_idx) => {
                self.mode = Mode::Input {
                    purpose: InputPurpose::NewSession { col_idx, thread_idx },
                    buffer: String::new(),
                };
            }
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let sessions = self.state.sessions_for_thread(
                    self.state.collections[col_idx].threads[thread_idx].id,
                );
                if let Some(session) = sessions.get(sess_idx) {
                    let name = session.tmux_session_name.clone();
                    self.attach_to_session(&name, terminal)?;
                }
            }
            SelectedItem::None => {
                let (col_idx, thread_idx) = self.state.ensure_general_thread();
                self.mode = Mode::Input {
                    purpose: InputPurpose::NewSession { col_idx, thread_idx },
                    buffer: String::new(),
                };
            }
        }
        Ok(())
    }

    fn start_finder(&mut self) {
        let mut sessions: Vec<_> = self.state.active_sessions.iter().collect();
        sessions.sort_by(|a, b| b.last_attached.cmp(&a.last_attached));

        let entries: Vec<(String, String)> = sessions
            .iter()
            .filter_map(|s| {
                let path = self.state.session_display_path(s)?;
                Some((s.tmux_session_name.clone(), path))
            })
            .collect();

        self.mode = Mode::Finder {
            state: FinderState::new(entries),
        };
    }

    fn handle_finder_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        terminal: &mut Tui,
    ) -> std::io::Result<()> {
        let ctrl = modifiers.contains(KeyModifiers::CONTROL);
        let nav_down = code == KeyCode::Down || (ctrl && code == KeyCode::Char('j'));
        let nav_up = code == KeyCode::Up || (ctrl && code == KeyCode::Char('k'));

        if nav_down {
            if let Mode::Finder { state } = &mut self.mode {
                if !state.filtered.is_empty() {
                    state.cursor = (state.cursor + 1).min(state.filtered.len() - 1);
                }
            }
        } else if nav_up {
            if let Mode::Finder { state } = &mut self.mode {
                state.cursor = state.cursor.saturating_sub(1);
            }
        } else {
            match code {
                KeyCode::Esc => {
                    self.mode = Mode::Normal;
                }
                KeyCode::Enter => {
                    let old_mode = std::mem::replace(&mut self.mode, Mode::Normal);
                    if let Mode::Finder { state } = old_mode {
                        if let Some(&idx) = state.filtered.get(state.cursor) {
                            let name = state.all_entries[idx].0.clone();
                            self.attach_to_session(&name, terminal)?;
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Mode::Finder { state } = &mut self.mode {
                        state.query.pop();
                        state.update_filter();
                    }
                }
                KeyCode::Char(c) => {
                    if let Mode::Finder { state } = &mut self.mode {
                        state.query.push(c);
                        state.update_filter();
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_notes_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.flush_note_editor();
                self.focus = Focus::Tree;
            }
            KeyCode::Char(c) => self.note_editor.insert_char(c),
            KeyCode::Backspace => self.note_editor.backspace(),
            KeyCode::Delete => self.note_editor.delete_char(),
            KeyCode::Enter => self.note_editor.newline(),
            KeyCode::Up => self.note_editor.move_up(),
            KeyCode::Down => self.note_editor.move_down(),
            KeyCode::Left => self.note_editor.move_left(),
            KeyCode::Right => self.note_editor.move_right(),
            _ => {}
        }
    }

    /// Derive the note key for the currently selected tree item.
    fn selected_note_key(&self) -> Option<String> {
        let selected = self.state.resolve_selection(self.tree_state.selected());
        match selected {
            SelectedItem::None => None,
            SelectedItem::Collection(idx) => {
                Some(self.state.collections[idx].id.to_string())
            }
            SelectedItem::Thread(col_idx, thread_idx) => {
                Some(self.state.collections[col_idx].threads[thread_idx].id.to_string())
            }
            SelectedItem::Session(col_idx, thread_idx, sess_idx) => {
                let thread = &self.state.collections[col_idx].threads[thread_idx];
                let sessions = self.state.sessions_for_thread(thread.id);
                sessions.get(sess_idx).map(|s| s.tmux_session_name.clone())
            }
        }
    }

    /// Sync the note editor with the current tree selection.
    /// If the selection changed, flush dirty content and load the new note.
    fn sync_note_editor(&mut self) {
        let new_key = self.selected_note_key();
        if new_key == self.note_editor.target_key {
            return; // same item, nothing to do
        }

        // Flush the old note if dirty
        self.flush_note_editor();

        // Load the new note
        match new_key {
            Some(key) => {
                let text = self.notes.get(&key).unwrap_or_default();
                self.note_editor.load(key, &text);
            }
            None => {
                self.note_editor.clear();
                // Also reset focus to tree if nothing is selected
                self.focus = Focus::Tree;
            }
        }
    }

    /// Write the editor's content to disk if dirty.
    fn flush_note_editor(&mut self) {
        if self.note_editor.dirty {
            if let Some(key) = &self.note_editor.target_key {
                let text = self.note_editor.to_text();
                self.notes.set(key, &text);
                self.note_editor.dirty = false;
            }
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
                    self.set_flash("Collection created");
                }
                InputPurpose::AddThread { collection_idx } => {
                    self.state.add_thread(collection_idx, trimmed);
                    // Auto-expand the collection so the new thread is visible
                    let col_id = self.state.collections[collection_idx].id.to_string();
                    self.tree_state.open(vec![col_id]);
                    self.save_state();
                    self.set_flash("Thread added");
                }
                InputPurpose::RenameCollection { idx } => {
                    // Collect old tmux session names before the rename changes the prefix.
                    let old_sessions: Vec<(String, String, usize)> = self.state.collections[idx]
                        .threads
                        .iter()
                        .enumerate()
                        .flat_map(|(pi, thread)| {
                            self.state.sessions_for_thread(thread.id)
                                .into_iter()
                                .map(move |s| (s.tmux_session_name.clone(), s.display_name.clone(), pi))
                        })
                        .collect();
                    self.state.rename_collection(idx, trimmed);
                    for (old_name, label, thread_idx) in &old_sessions {
                        if let Some(new_name) = self.state.make_session_name(idx, *thread_idx, label) {
                            let _ = tmux::rename_session(old_name, &new_name);
                        }
                    }
                    self.do_refresh_sessions();
                    self.save_state();
                    self.set_flash("Collection renamed");
                }
                InputPurpose::RenameThread { col_idx, thread_idx } => {
                    // Collect old tmux session names before the rename changes the prefix.
                    let old_sessions: Vec<(String, String)> = self.state.collections[col_idx]
                        .threads.get(thread_idx)
                        .map(|thread| {
                            self.state.sessions_for_thread(thread.id)
                                .into_iter()
                                .map(|s| (s.tmux_session_name.clone(), s.display_name.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    self.state.rename_thread(col_idx, thread_idx, trimmed);
                    for (old_name, label) in &old_sessions {
                        if let Some(new_name) = self.state.make_session_name(col_idx, thread_idx, label) {
                            let _ = tmux::rename_session(old_name, &new_name);
                        }
                    }
                    self.do_refresh_sessions();
                    self.save_state();
                    self.set_flash("Thread renamed");
                }
                InputPurpose::NewSession { col_idx, thread_idx } => {
                    if let Some(session_name) = self.state.make_session_name(col_idx, thread_idx, &trimmed) {
                        self.save_state();
                        self.launch_session(&session_name, terminal)?;
                        self.set_flash("Session launched");
                    }
                }
                InputPurpose::RenameSession { col_idx, thread_idx, old_tmux_name } => {
                    if let Some(new_tmux_name) = self.state.make_session_name(col_idx, thread_idx, &trimmed) {
                        let _ = tmux::rename_session(&old_tmux_name, &new_tmux_name);
                        self.do_refresh_sessions();
                        self.set_flash("Session renamed");
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
                    // Refresh first so active_sessions reflects any sessions created
                    // since the last 2-second tick.
                    self.do_refresh_sessions();
                    // Collect note keys and session names before deletion
                    let col = &self.state.collections[idx];
                    let mut note_keys: Vec<String> = vec![col.id.to_string()];
                    let mut session_names: Vec<String> = Vec::new();
                    for thread in &col.threads {
                        note_keys.push(thread.id.to_string());
                        for s in self.state.sessions_for_thread(thread.id) {
                            note_keys.push(s.tmux_session_name.clone());
                            session_names.push(s.tmux_session_name.clone());
                        }
                    }
                    for name in &session_names {
                        let _ = tmux::kill_session(name);
                    }
                    self.notes.remove_all(&note_keys);
                    self.state.delete_collection(idx);
                    // Select the item that slid into this position, or the one before
                    // it, rather than always jumping to the first collection.
                    let new_sel = self.state.collections.get(idx)
                        .or_else(|| self.state.collections.last())
                        .map(|c| vec![c.id.to_string()])
                        .unwrap_or_default();
                    self.tree_state.select(new_sel);
                    self.save_state();
                    self.do_refresh_sessions();
                    self.set_flash("Collection deleted");
                    self.sync_note_editor();
                }
                ConfirmPurpose::DeleteThread { col_idx, thread_idx, .. } => {
                    // Refresh first so active_sessions is current.
                    self.do_refresh_sessions();
                    let thread_id = self.state.collections[col_idx].threads[thread_idx].id;
                    let mut note_keys: Vec<String> = vec![thread_id.to_string()];
                    let session_names: Vec<String> = self.state.sessions_for_thread(thread_id)
                        .iter()
                        .map(|s| {
                            note_keys.push(s.tmux_session_name.clone());
                            s.tmux_session_name.clone()
                        })
                        .collect();
                    for name in session_names {
                        let _ = tmux::kill_session(&name);
                    }
                    self.notes.remove_all(&note_keys);
                    self.state.delete_thread(col_idx, thread_idx);
                    // Select the thread that slid into this position, or the one
                    // before it, falling back to the collection itself.
                    let col = &self.state.collections[col_idx];
                    let new_sel = col.threads.get(thread_idx)
                        .or_else(|| col.threads.last())
                        .map(|p| vec![col.id.to_string(), p.id.to_string()])
                        .unwrap_or_else(|| vec![col.id.to_string()]);
                    self.tree_state.select(new_sel);
                    self.save_state();
                    self.do_refresh_sessions();
                    self.set_flash("Thread deleted");
                    self.sync_note_editor();
                }
                ConfirmPurpose::KillSession { session_name } => {
                    let _ = tmux::kill_session(&session_name);
                    self.notes.remove(&session_name);
                    self.do_refresh_sessions();
                    self.set_flash("Session killed");
                    self.sync_note_editor();
                }
                ConfirmPurpose::KillAllSessions { col_idx, thread_idx, .. } => {
                    let thread_id = self.state.collections[col_idx].threads[thread_idx].id;
                    let names: Vec<String> = self
                        .state
                        .sessions_for_thread(thread_id)
                        .iter()
                        .map(|s| s.tmux_session_name.clone())
                        .collect();
                    for name in &names {
                        let _ = tmux::kill_session(name);
                    }
                    self.notes.remove_all(&names);
                    self.do_refresh_sessions();
                    self.set_flash("All sessions killed");
                    self.sync_note_editor();
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
            let _ = tmux::switch_client(session_name);
            self.running = false;
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
        let live = tmux::list_tws_sessions_with_timestamps();
        self.state.refresh_sessions(&live);
        self.last_refresh = Instant::now();
    }

    fn toggle_expand_all(&mut self) {
        let mut all_paths: Vec<Vec<String>> = Vec::new();

        for col in &self.state.collections {
            if col.is_root {
                for thread in &col.threads {
                    if self.state.active_sessions.iter().any(|s| s.thread_id == thread.id) {
                        all_paths.push(vec![thread.id.to_string()]);
                    }
                }
            } else {
                all_paths.push(vec![col.id.to_string()]);
                for thread in &col.threads {
                    if self.state.active_sessions.iter().any(|s| s.thread_id == thread.id) {
                        all_paths.push(vec![col.id.to_string(), thread.id.to_string()]);
                    }
                }
            }
        }

        let all_open = !all_paths.is_empty()
            && all_paths.iter().all(|p| self.tree_state.opened().contains(p));

        if all_open {
            self.tree_state.close_all();
        } else {
            for path in all_paths {
                self.tree_state.open(path);
            }
        }
    }

    fn save_state(&self) {
        if let Err(e) = persistence::save(&self.state.collections) {
            eprintln!("Failed to save state: {}", e);
        }
    }

    fn save_ui_state(&self) {
        let open_nodes: Vec<Vec<String>> = self.tree_state.opened().iter().cloned().collect();
        let selected = {
            let sel = self.tree_state.selected();
            if sel.is_empty() { None } else { Some(sel.to_vec()) }
        };
        let ui = persistence::UiState { open_nodes, selected };
        if let Err(e) = persistence::save_ui(&ui) {
            eprintln!("Failed to save UI state: {}", e);
        }
    }
}
