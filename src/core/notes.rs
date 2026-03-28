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
    /// Used in tests; editing in production is done via `$EDITOR`.
    #[cfg(test)]
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

    pub fn note_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.md", key))
    }
}

/// Read-only note viewer. Holds loaded note content and scroll state
/// for the preview sidebar. All editing is done via `$EDITOR`.
pub struct NoteEditor {
    pub content: String,
    pub scroll_offset: usize,
    pub target_key: Option<String>,
}

impl NoteEditor {
    pub fn new() -> Self {
        Self {
            content: String::new(),
            scroll_offset: 0,
            target_key: None,
        }
    }

    pub fn load(&mut self, key: String, text: &str) {
        self.target_key = Some(key);
        self.content = text.to_string();
        self.scroll_offset = 0;
    }

    pub fn reload(&mut self, store: &NoteStore) {
        if let Some(key) = &self.target_key {
            self.content = store.get(key).unwrap_or_default();
            self.scroll_offset = 0;
        }
    }

    pub fn clear(&mut self) {
        self.target_key = None;
        self.content = String::new();
        self.scroll_offset = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    pub fn line_count(&self) -> usize {
        if self.content.is_empty() {
            0
        } else {
            self.content.split('\n').count()
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self, total_lines: usize, visible_height: usize) {
        let max_offset = total_lines.saturating_sub(visible_height);
        if self.scroll_offset < max_offset {
            self.scroll_offset += 1;
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
    fn new_editor_is_empty() {
        let e = NoteEditor::new();
        assert!(e.is_empty());
        assert!(e.target_key.is_none());
    }

    #[test]
    fn load_stores_content() {
        let e = editor_with("hello\nworld");
        assert_eq!(e.content, "hello\nworld");
        assert_eq!(e.line_count(), 2);
        assert_eq!(e.scroll_offset, 0);
    }

    #[test]
    fn load_empty_is_empty() {
        let e = editor_with("");
        assert!(e.is_empty());
        assert_eq!(e.line_count(), 0);
    }

    #[test]
    fn clear_resets_everything() {
        let mut e = editor_with("Hello\nWorld");
        e.scroll_offset = 5;
        e.clear();
        assert!(e.is_empty());
        assert!(e.target_key.is_none());
        assert_eq!(e.scroll_offset, 0);
    }

    #[test]
    fn scroll_up_decrements() {
        let mut e = editor_with("a\nb\nc");
        e.scroll_offset = 2;
        e.scroll_up();
        assert_eq!(e.scroll_offset, 1);
    }

    #[test]
    fn scroll_up_at_zero_is_noop() {
        let mut e = editor_with("a\nb\nc");
        e.scroll_up();
        assert_eq!(e.scroll_offset, 0);
    }

    #[test]
    fn scroll_down_increments() {
        let mut e = editor_with("a\nb\nc\nd\ne");
        e.scroll_down(5, 3); // 5 lines, 3 visible → max offset = 2
        assert_eq!(e.scroll_offset, 1);
    }

    #[test]
    fn scroll_down_stops_at_max() {
        let mut e = editor_with("a\nb\nc\nd\ne");
        e.scroll_offset = 2;
        e.scroll_down(5, 3); // already at max
        assert_eq!(e.scroll_offset, 2);
    }
}
