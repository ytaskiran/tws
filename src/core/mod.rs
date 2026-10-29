pub mod markdown;
pub mod model;
pub mod notes;
pub mod persistence;
pub mod state;
pub mod status;
pub mod workdir;

/// A PATH lookup, not a `<name> --version` run: each run costs 20 to 100 ms of
/// the start, and the Rex open-tws key starts tws on each press.
pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}
