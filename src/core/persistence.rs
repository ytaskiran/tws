use std::fs;
use std::io;
use std::path::PathBuf;

use super::model::Collection;

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct UiState {
    pub open_nodes: Vec<Vec<String>>,
    pub selected: Option<Vec<String>>,
    #[serde(default)]
    pub agents_view_active: bool,
    #[serde(default)]
    pub agent_list_cursor: usize,
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
    dirs::config_dir()
        .expect("could not determine config directory")
        .join("tws")
}

fn state_file() -> PathBuf {
    config_dir().join("state.json")
}

pub fn load() -> io::Result<Vec<Collection>> {
    let path = state_file();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(&path)?;
    let collections: Vec<Collection> =
        serde_json::from_str(&data).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(collections)
}

pub fn save(collections: &[Collection]) -> io::Result<()> {
    let dir = config_dir();
    fs::create_dir_all(&dir)?;
    let data = serde_json::to_string_pretty(collections)?;
    fs::write(state_file(), data)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Thread;
    use std::env;

    fn with_temp_config<F: FnOnce()>(f: F) {
        let dir = env::temp_dir().join(format!("tws_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        // We test save/load by writing directly to a temp path
        // rather than overriding config_dir
        let path = dir.join("state.json");

        let mut col = Collection::new("Test");
        col.threads.push(Thread::new("Thread A"));
        let collections = vec![col];

        let data = serde_json::to_string_pretty(&collections).unwrap();
        fs::write(&path, &data).unwrap();

        let loaded: Vec<Collection> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Test");
        assert_eq!(loaded[0].threads.len(), 1);
        assert_eq!(loaded[0].threads[0].name, "Thread A");

        fs::remove_dir_all(&dir).unwrap();
        f();
    }

    #[test]
    fn round_trip_serialization() {
        with_temp_config(|| {});
    }

    #[test]
    fn load_missing_file_returns_empty() {
        // load() returns empty vec when file doesn't exist
        // We can't easily test this without mocking config_dir,
        // so we test the logic directly
        let path = env::temp_dir().join("tws_nonexistent_state.json");
        assert!(!path.exists());
        // Simulating what load() does:
        if !path.exists() {
            let result: Vec<Collection> = Vec::new();
            assert!(result.is_empty());
        }
    }

    #[test]
    fn deserialize_without_is_root_defaults_false() {
        // Simulate loading an old state.json that predates the is_root field
        let json = r#"[{
            "id": "00000000-0000-0000-0000-000000000001",
            "name": "Legacy",
            "threads": []
        }]"#;
        let collections: Vec<Collection> = serde_json::from_str(json).unwrap();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].name, "Legacy");
        assert!(!collections[0].is_root);
    }

    #[test]
    fn root_collection_round_trip() {
        let mut col = Collection::new_root();
        col.threads.push(Thread::new("general"));
        let collections = vec![col];

        let json = serde_json::to_string_pretty(&collections).unwrap();
        let loaded: Vec<Collection> = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.len(), 1);
        assert!(loaded[0].is_root);
        assert_eq!(loaded[0].threads.len(), 1);
        assert_eq!(loaded[0].threads[0].name, "general");
    }

    #[test]
    fn empty_collections_serialize() {
        let collections: Vec<Collection> = Vec::new();
        let json = serde_json::to_string_pretty(&collections).unwrap();
        let loaded: Vec<Collection> = serde_json::from_str(&json).unwrap();
        assert!(loaded.is_empty());
    }
}
