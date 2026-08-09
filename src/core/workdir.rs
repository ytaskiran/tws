use std::path::{Path, PathBuf};

/// Falls back to `/` when the home directory cannot be determined, so path
/// navigation degrades to the filesystem root instead of panicking. Unlike
/// `persistence::config_dir`, this runs inside the render loop where a panic
/// would tear down the terminal mid-frame.
#[allow(dead_code)]
pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[allow(dead_code)]
pub fn expand_tilde(s: &str) -> PathBuf {
    if s == "~" {
        return home_dir();
    }
    match s.strip_prefix("~/") {
        Some(rest) => home_dir().join(rest),
        None => PathBuf::from(s),
    }
}

#[allow(dead_code)]
pub fn shorten_home(path: &Path) -> String {
    match path.strip_prefix(home_dir()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// Browses one directory at a time. Never walks the tree, so opening is O(size
/// of one directory) regardless of how large the filesystem is.
#[allow(dead_code)]
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

#[allow(dead_code)]
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
        if !self.filtered.is_empty() {
            self.cursor = (self.cursor + 1).min(self.filtered.len() - 1);
        }
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
        if self.cursor >= self.filtered.len() {
            self.cursor = self.filtered.len().saturating_sub(1);
        }
    }
}

/// An unreadable or missing directory yields an empty list rather than an
/// error: navigation must survive permission denials and directories deleted
/// mid-browse.
#[allow(dead_code)]
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
    fn expand_tilde_bare() {
        assert_eq!(expand_tilde("~"), home_dir());
    }

    #[test]
    fn expand_tilde_with_subpath() {
        assert_eq!(expand_tilde("~/projects"), home_dir().join("projects"));
    }

    #[test]
    fn expand_tilde_leaves_absolute_paths_alone() {
        assert_eq!(expand_tilde("/usr/local"), PathBuf::from("/usr/local"));
    }

    #[test]
    fn expand_tilde_does_not_expand_mid_string_tilde() {
        assert_eq!(expand_tilde("/tmp/~foo"), PathBuf::from("/tmp/~foo"));
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
    fn shorten_home_round_trips_with_expand_tilde() {
        let p = home_dir().join("a").join("b");
        assert_eq!(expand_tilde(&shorten_home(&p)), p);
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
    fn cursor_clamps_when_filter_shrinks_results() {
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
        assert_eq!(picker.cursor(), 1);
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
}
