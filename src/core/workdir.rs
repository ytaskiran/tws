use std::path::{Path, PathBuf};

/// Falls back to `/` when the home directory cannot be determined, so path
/// navigation degrades to the filesystem root instead of panicking. Unlike
/// `persistence::config_dir`, this runs inside the render loop where a panic
/// would tear down the terminal mid-frame.
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn shorten_home(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Resolves where a new session should start. The bool is true when the thread
/// had a directory configured but it no longer exists — the caller warns, but a
/// stale path never blocks a launch.
pub fn resolve_launch_dir(working_dir: Option<&Path>) -> (PathBuf, bool) {
    match working_dir {
        Some(dir) if dir.is_dir() => (dir.to_path_buf(), false),
        Some(_) => (home_dir(), true),
        None => (home_dir(), false),
    }
}

/// Browses one directory at a time. Never walks the tree, so opening is O(size
/// of one directory) regardless of how large the filesystem is.
pub struct DirPicker {
    current: PathBuf,
    query: String,
    /// Every subdirectory of `current`, including hidden ones. Hiding is a
    /// filter concern so that typing a leading `.` can reveal them without a
    /// re-read.
    entries: Vec<String>,
    filtered: Vec<usize>,
    cursor: usize,
}

impl DirPicker {
    pub fn open(start: PathBuf) -> Self {
        let mut picker = Self {
            current: start,
            query: String::new(),
            entries: Vec::new(),
            filtered: Vec::new(),
            cursor: 0,
        };
        picker.reload();
        picker
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn filtered_names(&self) -> Vec<&str> {
        self.filtered
            .iter()
            .map(|&i| self.entries[i].as_str())
            .collect()
    }

    pub fn push_char(&mut self, c: char) {
        self.query.push(c);
        self.cursor = 0;
        self.refilter();
    }

    pub fn move_down(&mut self) {
        self.cursor = (self.cursor + 1).min(self.filtered.len());
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn reload(&mut self) {
        self.entries = list_subdirs(&self.current);
        self.query.clear();
        self.cursor = 0;
        self.refilter();
    }

    fn refilter(&mut self) {
        let q = self.query.to_lowercase();
        let show_hidden = self.query.starts_with('.');
        self.filtered = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, name)| show_hidden || !name.starts_with('.'))
            .filter(|(_, name)| q.is_empty() || name.to_lowercase().contains(&q))
            .map(|(i, _)| i)
            .collect();
    }

    /// Descends one level. From row 0 this completes into the first match, so
    /// typing a segment then tabbing walks downward the way a shell does; from
    /// a subdirectory row it enters that row. The query is cleared by `reload`,
    /// since it described a match in the directory we just left.
    pub fn complete(&mut self) {
        let target = match self.cursor {
            0 => self.filtered.first().map(|&i| self.entries[i].clone()),
            _ => self.highlighted().map(str::to_string),
        };
        let Some(name) = target else {
            return;
        };
        self.current = self.current.join(name);
        self.reload();
    }

    pub fn ascend(&mut self) {
        let Some(parent) = self.current.parent().map(Path::to_path_buf) else {
            return;
        };
        self.current = parent;
        self.reload();
    }

    /// Editing the query takes priority; only an already-empty query walks up a
    /// level, so backspace never skips characters the user typed.
    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.cursor = 0;
            self.refilter();
        } else {
            self.ascend();
        }
    }

    /// Row 0 is the directory being browsed, so confirming there always yields
    /// `current` — a partial filter is a transient search, not part of the path.
    pub fn selection(&self) -> PathBuf {
        match self.highlighted() {
            Some(name) => self.current.join(name),
            None => self.current.clone(),
        }
    }

    /// True when the cursor sits on row 0, the "use this directory" row.
    pub fn is_current_row(&self) -> bool {
        self.cursor == 0
    }

    /// Row 0 has no entry behind it; rows below are offset by one.
    fn highlighted(&self) -> Option<&str> {
        let row = self.cursor.checked_sub(1)?;
        let idx = *self.filtered.get(row)?;
        Some(self.entries[idx].as_str())
    }
}

/// An unreadable or missing directory yields an empty list rather than an
/// error: navigation must survive permission denials and directories deleted
/// mid-browse.
fn list_subdirs(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Builds a throwaway directory tree and returns its root.
    /// Each call gets a unique root so tests can run in parallel.
    fn fixture(subdirs: &[&str], files: &[&str]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tws_workdir_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        for d in subdirs {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in files {
            fs::write(root.join(f), b"x").unwrap();
        }
        root
    }

    #[test]
    fn shorten_home_replaces_home_prefix() {
        let p = home_dir().join("projects").join("tws");
        assert_eq!(shorten_home(&p), "~/projects/tws");
    }

    #[test]
    fn shorten_home_of_home_itself() {
        assert_eq!(shorten_home(&home_dir()), "~");
    }

    #[test]
    fn shorten_home_leaves_outside_paths_alone() {
        assert_eq!(shorten_home(Path::new("/usr/local")), "/usr/local");
    }

    #[test]
    fn lists_subdirectories_only() {
        let root = fixture(&["alpha", "beta"], &["notes.txt", "README.md"]);
        let picker = DirPicker::open(root.clone());
        assert_eq!(picker.filtered_names(), vec!["alpha", "beta"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sorts_case_insensitively() {
        let root = fixture(&["Zebra", "apple", "Mango"], &[]);
        let picker = DirPicker::open(root.clone());
        assert_eq!(picker.filtered_names(), vec!["apple", "Mango", "Zebra"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn hides_dotdirs_by_default() {
        let root = fixture(&[".git", ".cache", "src"], &[]);
        let picker = DirPicker::open(root.clone());
        assert_eq!(picker.filtered_names(), vec!["src"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn shows_dotdirs_when_query_starts_with_dot() {
        let root = fixture(&[".git", ".cache", "src"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('.');
        assert_eq!(picker.filtered_names(), vec![".cache", ".git"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn filters_by_case_insensitive_substring() {
        let root = fixture(&["edge-pipeline", "Edge-Firmware", "docs"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('E');
        picker.push_char('D');
        picker.push_char('G');
        // Sorted case-insensitively, so "Edge-Firmware" precedes "edge-pipeline".
        assert_eq!(
            picker.filtered_names(),
            vec!["Edge-Firmware", "edge-pipeline"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn nonexistent_directory_lists_nothing() {
        let missing = std::env::temp_dir().join("tws_definitely_not_here_9e1f");
        let picker = DirPicker::open(missing);
        assert!(picker.filtered_names().is_empty());
    }

    #[test]
    fn push_char_resets_cursor_to_top() {
        let root = fixture(&["aa", "ab", "ac"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.move_down();
        picker.move_down();
        assert_eq!(picker.cursor(), 2);
        picker.push_char('a');
        picker.push_char('b');
        assert_eq!(picker.filtered_names(), vec!["ab"]);
        assert_eq!(picker.cursor(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cursor_stays_zero_when_nothing_matches() {
        let root = fixture(&["aa", "ab"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('z');
        assert!(picker.filtered_names().is_empty());
        assert_eq!(picker.cursor(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn move_down_stops_at_last_entry() {
        let root = fixture(&["one", "two"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.move_down();
        picker.move_down();
        picker.move_down();
        // Rows are: 0 = current directory, 1 = "one", 2 = "two".
        assert_eq!(picker.cursor(), 2);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn move_up_stops_at_first_entry() {
        let root = fixture(&["one", "two"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.move_up();
        assert_eq!(picker.cursor(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn selection_joins_highlighted_entry() {
        let root = fixture(&["alpha", "beta"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.move_down();
        assert_eq!(picker.selection(), root.join("alpha"));
        picker.move_down();
        assert_eq!(picker.selection(), root.join("beta"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn selection_at_row_zero_is_current_even_when_nothing_matches() {
        let root = fixture(&["alpha"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('z');
        assert!(picker.filtered_names().is_empty());
        assert_eq!(picker.selection(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn selection_at_row_zero_is_current_with_no_subdirs() {
        let root = fixture(&[], &["only-a-file.txt"]);
        let picker = DirPicker::open(root.clone());
        assert_eq!(picker.selection(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cursor_starts_on_the_current_directory_row() {
        let root = fixture(&["alpha", "beta"], &[]);
        let picker = DirPicker::open(root.clone());
        assert_eq!(picker.cursor(), 0);
        assert!(picker.is_current_row());
        assert_eq!(picker.selection(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn is_current_row_only_at_cursor_zero() {
        let root = fixture(&["alpha"], &[]);
        let mut picker = DirPicker::open(root.clone());
        assert!(picker.is_current_row());
        picker.move_down();
        assert!(!picker.is_current_row());
        picker.move_up();
        assert!(picker.is_current_row());
        fs::remove_dir_all(&root).unwrap();
    }

    /// Enter on row 0 confirms where you are standing, discarding a partial
    /// filter: row 0 means one thing regardless of what has been typed.
    #[test]
    fn selection_at_row_zero_ignores_a_partial_filter() {
        let root = fixture(&["edge-pipeline"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('e');
        picker.push_char('d');
        assert_eq!(picker.filtered_names(), vec!["edge-pipeline"]);
        assert!(picker.is_current_row());
        assert_eq!(picker.selection(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    /// Tab on row 0 acts as shell-style completion into the first match, so
    /// typing a path segment then tabbing walks downward.
    #[test]
    fn complete_from_row_zero_descends_into_the_first_match() {
        let root = fixture(&["alpha", "beta"], &[]);
        fs::create_dir_all(root.join("beta").join("inner")).unwrap();
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('b');
        assert!(picker.is_current_row());
        picker.complete();
        assert_eq!(picker.current(), root.join("beta"));
        assert_eq!(picker.filtered_names(), vec!["inner"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn complete_from_a_subdirectory_row_descends_into_that_row() {
        let root = fixture(&["alpha", "beta"], &[]);
        fs::create_dir_all(root.join("beta").join("inner")).unwrap();
        let mut picker = DirPicker::open(root.clone());
        picker.move_down();
        picker.move_down();
        picker.complete();
        assert_eq!(picker.current(), root.join("beta"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn descending_returns_the_cursor_to_row_zero() {
        let root = fixture(&["alpha"], &[]);
        fs::create_dir_all(root.join("alpha").join("inner")).unwrap();
        let mut picker = DirPicker::open(root.clone());
        picker.move_down();
        picker.complete();
        assert_eq!(picker.cursor(), 0);
        assert!(picker.is_current_row());
        assert_eq!(picker.selection(), root.join("alpha"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn complete_descends_and_clears_query() {
        let root = fixture(&["projects"], &[]);
        fs::create_dir_all(root.join("projects").join("tws")).unwrap();
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('p');
        picker.complete();
        assert_eq!(picker.current(), root.join("projects"));
        assert_eq!(picker.query(), "");
        assert_eq!(picker.filtered_names(), vec!["tws"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn complete_is_a_noop_when_no_entry_matches() {
        let root = fixture(&["alpha"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('z');
        picker.complete();
        assert_eq!(picker.current(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ascend_moves_to_parent() {
        let root = fixture(&["projects"], &[]);
        let mut picker = DirPicker::open(root.join("projects"));
        picker.ascend();
        assert_eq!(picker.current(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn ascend_at_filesystem_root_is_a_noop() {
        let mut picker = DirPicker::open(PathBuf::from("/"));
        picker.ascend();
        assert_eq!(picker.current(), Path::new("/"));
    }

    #[test]
    fn backspace_deletes_a_query_char_before_ascending() {
        let root = fixture(&["alpha"], &[]);
        let mut picker = DirPicker::open(root.clone());
        picker.push_char('a');
        picker.backspace();
        assert_eq!(picker.query(), "");
        assert_eq!(picker.current(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn backspace_on_empty_query_ascends() {
        let root = fixture(&["projects"], &[]);
        let mut picker = DirPicker::open(root.join("projects"));
        picker.backspace();
        assert_eq!(picker.current(), root);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_launch_dir_uses_existing_directory() {
        let root = fixture(&[], &[]);
        let (dir, missing) = resolve_launch_dir(Some(root.as_path()));
        assert_eq!(dir, root);
        assert!(!missing);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_launch_dir_falls_back_when_directory_is_gone() {
        let gone = std::env::temp_dir().join("tws_gone_4c7a");
        let (dir, missing) = resolve_launch_dir(Some(gone.as_path()));
        assert_eq!(dir, home_dir());
        assert!(missing);
    }

    #[test]
    fn resolve_launch_dir_falls_back_when_unset() {
        let (dir, missing) = resolve_launch_dir(None);
        assert_eq!(dir, home_dir());
        assert!(!missing);
    }

    #[test]
    fn resolve_launch_dir_flags_a_file_as_missing() {
        let root = fixture(&[], &["notes.txt"]);
        let (dir, missing) = resolve_launch_dir(Some(root.join("notes.txt").as_path()));
        assert_eq!(dir, home_dir());
        assert!(missing);
        fs::remove_dir_all(&root).unwrap();
    }
}
