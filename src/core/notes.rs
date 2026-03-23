use std::fs;
use std::path::PathBuf;

use super::persistence;

/// File-based note storage. Each note is a `.md` file in `~/.config/tws/notes/`.
///
/// Keys are UUID strings (for collections/threads) or tmux session names (for sessions).
pub struct NoteStore {
    dir: PathBuf,
}

impl NoteStore {
    /// Create a new NoteStore, ensuring the notes directory exists.
    pub fn new() -> Self {
        let dir = persistence::config_dir().join("notes");
        fs::create_dir_all(&dir).expect("could not create notes directory");
        Self { dir }
    }

    /// Get the note text for a key. Returns None if no note exists or the file is empty.
    pub fn get(&self, key: &str) -> Option<String> {
        let path = self.note_path(key);
        match fs::read_to_string(&path) {
            Ok(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    /// Set the note text for a key. If text is empty, the file is deleted.
    pub fn set(&self, key: &str, text: &str) {
        let path = self.note_path(key);
        if text.trim().is_empty() {
            let _ = fs::remove_file(&path);
        } else {
            let _ = fs::write(&path, text);
        }
    }

    /// Remove a single note file.
    pub fn remove(&self, key: &str) {
        let _ = fs::remove_file(self.note_path(key));
    }

    /// Remove multiple note files at once.
    pub fn remove_all(&self, keys: &[String]) {
        for key in keys {
            self.remove(key);
        }
    }

    fn note_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.md", key))
    }
}

/// Multiline text editor for a single note.
///
/// Tracks cursor position and scroll offset. The note content is stored as
/// a `Vec<String>` (one entry per line) for efficient line-based operations.
pub struct NoteEditor {
    pub lines: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll_offset: usize,
    /// Which note is currently loaded (UUID string or tmux session name).
    pub target_key: Option<String>,
    /// Whether the content has changed since last save/load.
    pub dirty: bool,
}

impl NoteEditor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_offset: 0,
            target_key: None,
            dirty: false,
        }
    }

    /// Load a note's text into the editor, resetting cursor and scroll.
    pub fn load(&mut self, key: String, text: &str) {
        self.target_key = Some(key);
        self.lines = if text.is_empty() {
            vec![String::new()]
        } else {
            text.split('\n').map(|s| s.to_string()).collect()
        };
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.dirty = false;
    }

    /// Clear the editor (no note loaded).
    pub fn clear(&mut self) {
        self.target_key = None;
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_offset = 0;
        self.dirty = false;
    }

    /// Join lines back into a single string.
    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Whether the editor has any meaningful content.
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Insert a character at the cursor position.
    pub fn insert_char(&mut self, c: char) {
        let line = &mut self.lines[self.cursor_row];
        // Clamp cursor_col in case the line shrank
        self.cursor_col = self.cursor_col.min(line.len());
        line.insert(self.cursor_col, c);
        self.cursor_col += 1;
        self.dirty = true;
    }

    /// Delete the character before the cursor (backspace).
    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_row];
            self.cursor_col -= 1;
            line.remove(self.cursor_col);
            self.dirty = true;
        } else if self.cursor_row > 0 {
            // Merge current line into the previous one
            let current = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&current);
            self.dirty = true;
        }
    }

    /// Delete the character at the cursor position.
    pub fn delete_char(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            self.lines[self.cursor_row].remove(self.cursor_col);
            self.dirty = true;
        } else if self.cursor_row + 1 < self.lines.len() {
            // Merge next line into current
            let next = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next);
            self.dirty = true;
        }
    }

    /// Insert a newline at the cursor, splitting the current line.
    pub fn newline(&mut self) {
        let line = &mut self.lines[self.cursor_row];
        self.cursor_col = self.cursor_col.min(line.len());
        let rest = line[self.cursor_col..].to_string();
        line.truncate(self.cursor_col);
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, rest);
        self.cursor_col = 0;
        self.dirty = true;
    }

    pub fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.clamp_col();
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.clamp_col();
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
    }

    pub fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    /// Adjust scroll_offset so the cursor row is visible within the given height.
    pub fn ensure_visible(&mut self, visible_height: usize) {
        if visible_height == 0 {
            return;
        }
        if self.cursor_row < self.scroll_offset {
            self.scroll_offset = self.cursor_row;
        } else if self.cursor_row >= self.scroll_offset + visible_height {
            self.scroll_offset = self.cursor_row - visible_height + 1;
        }
    }

    /// Clamp cursor_col to the current line's length.
    fn clamp_col(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col > line_len {
            self.cursor_col = line_len;
        }
    }
}

#[cfg(test)]
impl NoteStore {
    /// Create a NoteStore backed by a custom directory (for testing).
    fn with_dir(dir: PathBuf) -> Self {
        fs::create_dir_all(&dir).unwrap();
        Self { dir }
    }
}

#[cfg(test)]
mod note_store_tests {
    use super::*;
    use std::env;

    fn temp_store() -> (NoteStore, PathBuf) {
        let dir = env::temp_dir().join(format!("tws_notes_test_{}", uuid::Uuid::new_v4()));
        let store = NoteStore::with_dir(dir.clone());
        (store, dir)
    }

    #[test]
    fn get_missing_returns_none() {
        let (store, dir) = temp_store();
        assert!(store.get("nonexistent").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_and_get_round_trip() {
        let (store, dir) = temp_store();
        store.set("abc-123", "Hello, world!");
        assert_eq!(store.get("abc-123").unwrap(), "Hello, world!");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_empty_deletes_file() {
        let (store, dir) = temp_store();
        store.set("key1", "some content");
        assert!(store.get("key1").is_some());

        store.set("key1", "");
        assert!(store.get("key1").is_none());
        assert!(!store.note_path("key1").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_whitespace_only_deletes_file() {
        let (store, dir) = temp_store();
        store.set("key1", "content");
        store.set("key1", "   \n  ");
        assert!(store.get("key1").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_deletes_file() {
        let (store, dir) = temp_store();
        store.set("key1", "hello");
        store.remove("key1");
        assert!(store.get("key1").is_none());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let (store, dir) = temp_store();
        store.remove("does-not-exist"); // should not panic
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn remove_all_deletes_multiple() {
        let (store, dir) = temp_store();
        store.set("a", "note a");
        store.set("b", "note b");
        store.set("c", "note c");

        store.remove_all(&["a".into(), "b".into()]);
        assert!(store.get("a").is_none());
        assert!(store.get("b").is_none());
        assert!(store.get("c").is_some());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multiline_content_preserved() {
        let (store, dir) = temp_store();
        let content = "# Heading\n\n- item 1\n- item 2\n\nParagraph text.";
        store.set("md-test", content);
        assert_eq!(store.get("md-test").unwrap(), content);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwrite_existing_note() {
        let (store, dir) = temp_store();
        store.set("key1", "first version");
        store.set("key1", "second version");
        assert_eq!(store.get("key1").unwrap(), "second version");
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod note_editor_tests {
    use super::*;

    fn editor_with(text: &str) -> NoteEditor {
        let mut e = NoteEditor::new();
        e.load("test-key".into(), text);
        e
    }

    #[test]
    fn new_editor_has_one_empty_line() {
        let e = NoteEditor::new();
        assert_eq!(e.lines, vec![""]);
        assert!(e.is_empty());
        assert!(e.target_key.is_none());
    }

    #[test]
    fn load_splits_lines() {
        let e = editor_with("hello\nworld");
        assert_eq!(e.lines, vec!["hello", "world"]);
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 0);
        assert!(!e.dirty);
    }

    #[test]
    fn load_empty_gives_one_empty_line() {
        let e = editor_with("");
        assert_eq!(e.lines, vec![""]);
        assert!(e.is_empty());
    }

    #[test]
    fn to_text_joins_lines() {
        let e = editor_with("a\nb\nc");
        assert_eq!(e.to_text(), "a\nb\nc");
    }

    #[test]
    fn insert_char_at_start() {
        let mut e = editor_with("ello");
        e.insert_char('H');
        assert_eq!(e.lines[0], "Hello");
        assert_eq!(e.cursor_col, 1);
        assert!(e.dirty);
    }

    #[test]
    fn insert_char_at_end() {
        let mut e = editor_with("Hi");
        e.cursor_col = 2;
        e.insert_char('!');
        assert_eq!(e.lines[0], "Hi!");
        assert_eq!(e.cursor_col, 3);
    }

    #[test]
    fn backspace_mid_line() {
        let mut e = editor_with("Hello");
        e.cursor_col = 3;
        e.backspace();
        assert_eq!(e.lines[0], "Helo");
        assert_eq!(e.cursor_col, 2);
    }

    #[test]
    fn backspace_at_line_start_merges() {
        let mut e = editor_with("Hello\nWorld");
        e.cursor_row = 1;
        e.cursor_col = 0;
        e.backspace();
        assert_eq!(e.lines, vec!["HelloWorld"]);
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 5);
    }

    #[test]
    fn backspace_at_start_of_first_line_is_noop() {
        let mut e = editor_with("Hello");
        e.cursor_col = 0;
        e.backspace();
        assert_eq!(e.lines[0], "Hello");
        assert!(!e.dirty);
    }

    #[test]
    fn delete_char_mid_line() {
        let mut e = editor_with("Hello");
        e.cursor_col = 1;
        e.delete_char();
        assert_eq!(e.lines[0], "Hllo");
    }

    #[test]
    fn delete_char_at_end_merges_next_line() {
        let mut e = editor_with("Hello\nWorld");
        e.cursor_col = 5; // end of "Hello"
        e.delete_char();
        assert_eq!(e.lines, vec!["HelloWorld"]);
    }

    #[test]
    fn delete_char_at_end_of_last_line_is_noop() {
        let mut e = editor_with("Hello");
        e.cursor_col = 5;
        e.delete_char();
        assert_eq!(e.lines[0], "Hello");
        assert!(!e.dirty);
    }

    #[test]
    fn newline_splits_line() {
        let mut e = editor_with("HelloWorld");
        e.cursor_col = 5;
        e.newline();
        assert_eq!(e.lines, vec!["Hello", "World"]);
        assert_eq!(e.cursor_row, 1);
        assert_eq!(e.cursor_col, 0);
    }

    #[test]
    fn newline_at_start() {
        let mut e = editor_with("Hello");
        e.cursor_col = 0;
        e.newline();
        assert_eq!(e.lines, vec!["", "Hello"]);
        assert_eq!(e.cursor_row, 1);
        assert_eq!(e.cursor_col, 0);
    }

    #[test]
    fn newline_at_end() {
        let mut e = editor_with("Hello");
        e.cursor_col = 5;
        e.newline();
        assert_eq!(e.lines, vec!["Hello", ""]);
        assert_eq!(e.cursor_row, 1);
    }

    #[test]
    fn move_up_down() {
        let mut e = editor_with("aaa\nbbb\nccc");
        e.cursor_row = 1;
        e.cursor_col = 2;

        e.move_up();
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 2);

        e.move_down();
        assert_eq!(e.cursor_row, 1);
    }

    #[test]
    fn move_up_at_top_is_noop() {
        let mut e = editor_with("Hello");
        e.move_up();
        assert_eq!(e.cursor_row, 0);
    }

    #[test]
    fn move_down_at_bottom_is_noop() {
        let mut e = editor_with("Hello");
        e.move_down();
        assert_eq!(e.cursor_row, 0);
    }

    #[test]
    fn move_up_clamps_col_to_shorter_line() {
        let mut e = editor_with("Hi\nHello");
        e.cursor_row = 1;
        e.cursor_col = 4; // position in "Hello"
        e.move_up();
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 2); // clamped to "Hi" length
    }

    #[test]
    fn move_left_wraps_to_prev_line() {
        let mut e = editor_with("abc\ndef");
        e.cursor_row = 1;
        e.cursor_col = 0;
        e.move_left();
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 3);
    }

    #[test]
    fn move_right_wraps_to_next_line() {
        let mut e = editor_with("abc\ndef");
        e.cursor_row = 0;
        e.cursor_col = 3;
        e.move_right();
        assert_eq!(e.cursor_row, 1);
        assert_eq!(e.cursor_col, 0);
    }

    #[test]
    fn move_left_at_start_is_noop() {
        let mut e = editor_with("Hello");
        e.move_left();
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 0);
    }

    #[test]
    fn move_right_at_end_is_noop() {
        let mut e = editor_with("Hello");
        e.cursor_col = 5;
        e.move_right();
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 5);
    }

    #[test]
    fn ensure_visible_scrolls_down() {
        let mut e = editor_with("a\nb\nc\nd\ne\nf\ng\nh");
        e.cursor_row = 5;
        e.scroll_offset = 0;
        e.ensure_visible(3);
        assert_eq!(e.scroll_offset, 3); // cursor_row(5) - height(3) + 1
    }

    #[test]
    fn ensure_visible_scrolls_up() {
        let mut e = editor_with("a\nb\nc\nd\ne");
        e.cursor_row = 1;
        e.scroll_offset = 3;
        e.ensure_visible(3);
        assert_eq!(e.scroll_offset, 1);
    }

    #[test]
    fn ensure_visible_noop_when_in_view() {
        let mut e = editor_with("a\nb\nc\nd\ne");
        e.cursor_row = 2;
        e.scroll_offset = 1;
        e.ensure_visible(3);
        assert_eq!(e.scroll_offset, 1); // cursor is within [1, 4), no change
    }

    #[test]
    fn clear_resets_everything() {
        let mut e = editor_with("Hello\nWorld");
        e.cursor_row = 1;
        e.cursor_col = 3;
        e.dirty = true;
        e.clear();
        assert!(e.is_empty());
        assert!(e.target_key.is_none());
        assert!(!e.dirty);
        assert_eq!(e.cursor_row, 0);
        assert_eq!(e.cursor_col, 0);
    }
}
