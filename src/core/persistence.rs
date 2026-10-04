use std::fs;
use std::io;
use std::path::PathBuf;

use super::model::Thread;

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct UiState {
    pub open_nodes: Vec<Vec<String>>,
    pub selected: Option<Vec<String>>,
    #[serde(default)]
    pub agents_view_active: bool,
    #[serde(default)]
    pub agent_list_cursor: usize,
    /// Persisted pin assignments: `(pane_id, slot)`. Reapplied on first scan after startup;
    /// entries whose pane_id is no longer live are silently dropped.
    ///
    /// Note: tmux recycles pane ids (%1, %2, …) after a server restart, so a restored pin
    /// could theoretically attach to an unrelated agent that inherited the id. This is
    /// inherently fuzzy for cross-restart persistence; within a single tmux server lifetime
    /// the pane_id is stable and the mapping is exact.
    #[serde(default)]
    pub pins: Vec<(String, u8)>,
}

fn ui_state_file() -> PathBuf {
    config_dir().join("ui-state.json")
}

pub fn load_ui() -> UiState {
    let path = ui_state_file();
    if !path.exists() {
        return UiState::default();
    }
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return UiState::default(),
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save_ui(ui: &UiState) -> io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let data = serde_json::to_string_pretty(ui)?;
    fs::write(ui_state_file(), data)?;
    Ok(())
}

pub(crate) fn config_dir() -> PathBuf {
    dirs::home_dir()
        .expect("could not determine home directory")
        .join(".config")
        .join("tws")
}

fn state_file() -> PathBuf {
    config_dir().join("state.json")
}

pub fn load() -> io::Result<Vec<Thread>> {
    let path = state_file();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(&path)?;
    serde_json::from_str(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub fn save(threads: &[Thread]) -> io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    save_to(&state_file(), threads)
}

/// Writes a new file and renames it over the old one, so a reader such as
/// `tws bar where` never sees a half-written file. A symlinked state.json
/// (a dotfiles repo, for example) is written at its target, so the link stays.
fn save_to(path: &std::path::Path, threads: &[Thread]) -> io::Result<()> {
    let data = serde_json::to_string_pretty(threads)?;
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    crate::core::status::write_atomic(&target, &data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn round_trip_threads() {
        let mut t = Thread::new("api");
        t.working_dir = Some("/tmp".into());
        let json = serde_json::to_string_pretty(&vec![t.clone()]).unwrap();
        let loaded: Vec<Thread> = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, t.id);
        assert_eq!(loaded[0].working_dir, t.working_dir);
    }

    #[test]
    fn old_collection_file_does_not_load() {
        // A file from before threads were top level. Loading it as threads
        // would drop the nested threads, and the next save would delete them.
        let json = r#"[{"id":"00000000-0000-0000-0000-000000000001","name":"","is_root":true,
            "threads":[{"id":"00000000-0000-0000-0000-000000000002","name":"api","description":null}]}]"#;
        assert!(serde_json::from_str::<Vec<Thread>>(json).is_err());
    }

    #[test]
    fn save_replaces_the_file_and_does_not_rewrite_it() {
        // A reader such as `tws bar where` must never see a half-written file,
        // so save writes a new file and renames it over the old one. A hard
        // link to the old file then keeps the old text.
        let dir = env::temp_dir().join(format!("tws_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, "[]").unwrap();
        fs::hard_link(&path, dir.join("old.json")).unwrap();

        save_to(&path, &[Thread::new("New")]).unwrap();

        assert_eq!(fs::read_to_string(dir.join("old.json")).unwrap(), "[]");
        let loaded: Vec<Thread> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded[0].name, "New");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_writes_through_a_symlink_and_keeps_the_mode() {
        // A dotfiles setup can link state.json into a repo. A rename onto the
        // link would replace the link with a plain file.
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = env::temp_dir().join(format!("tws_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let real = dir.join("real.json");
        fs::write(&real, "[]").unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.join("state.json");
        symlink(&real, &link).unwrap();

        save_to(&link, &[Thread::new("New")]).unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(fs::read_to_string(&real).unwrap().contains("New"));
        let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_dir_all(&dir).unwrap();
    }
}
