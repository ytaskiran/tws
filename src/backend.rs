//! The one place that knows which backend runs. The app calls only these
//! functions, so a Rex change cannot break tmux, and each backend keeps its
//! own side effects: tmux the hook files, Rex its snapshot.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use crate::rex;
use crate::tmux::commands as tmux;

pub use crate::config::Backend;

/// `backend` in config.toml picks the backend, and `TWS_BACKEND` overrides it.
/// `run_tui` loads the config first, so a config that does not parse stops
/// there with its own error before this default applies.
pub fn current() -> Backend {
    static CURRENT: OnceLock<Backend> = OnceLock::new();
    *CURRENT.get_or_init(|| {
        let env = std::env::var("TWS_BACKEND").map(|v| v.to_ascii_lowercase());
        match env.as_deref() {
            Ok("tmux") => Backend::Tmux,
            Ok("rex") => Backend::Rex,
            _ => crate::config::try_load_config()
                .map(|c| c.backend)
                .unwrap_or_default(),
        }
    })
}

pub fn is_rex() -> bool {
    current() == Backend::Rex
}

/// False when the backend command cannot start, for example when it is not
/// installed.
pub fn installed() -> bool {
    match current() {
        Backend::Tmux => tmux::installed(),
        Backend::Rex => rex::installed(),
    }
}

/// The error for a missing backend command, with how to fix it.
pub fn missing_message() -> String {
    match current() {
        Backend::Rex => "tws uses Rex, and the rex command is not on PATH. \
                         To use tmux, set backend = \"tmux\" in ~/.config/tws/config.toml."
            .to_string(),
        Backend::Tmux => {
            let hint = if cfg!(target_os = "macos") {
                "Install it with: brew install tmux"
            } else {
                "Install it with your package manager, for example: sudo apt install tmux"
            };
            format!("tws needs tmux, and tmux is not installed. {hint}")
        }
    }
}

/// tmux hooks touch a trigger file, so a slow poll is enough. Rex has no
/// trigger, so tws polls its snapshot (one `rex do` call).
pub fn refresh_interval() -> Duration {
    match current() {
        Backend::Tmux => Duration::from_secs(30),
        Backend::Rex => Duration::from_secs(2),
    }
}

/// The live tws sessions with their last-attached time. `None` when Rex did
/// not answer: the caller keeps the last state, so one missed poll does not
/// drop the sessions, the pins and the agent names.
pub fn refresh() -> Option<Vec<(String, i64)>> {
    match current() {
        Backend::Tmux => Some(tmux::list_tws_sessions_with_timestamps()),
        Backend::Rex => rex::refresh().then(rex::list_tws_sessions_with_timestamps),
    }
}

pub fn new_session(name: &str, cwd: Option<&Path>) -> std::io::Result<bool> {
    match current() {
        Backend::Tmux => tmux::new_session(name, cwd),
        Backend::Rex => rex::new_session(name, cwd),
    }
}

pub fn kill_session(name: &str) -> std::io::Result<bool> {
    match current() {
        Backend::Tmux => tmux::kill_session(name),
        Backend::Rex => rex::kill_session(name),
    }
}

pub fn rename_session(old_name: &str, new_name: &str) -> std::io::Result<bool> {
    match current() {
        Backend::Tmux => tmux::rename_session(old_name, new_name),
        Backend::Rex => rex::rename_session(old_name, new_name),
    }
}

/// The pane text with its colors, for the preview.
pub fn capture(pane_id: &str) -> Option<String> {
    match current() {
        Backend::Tmux => tmux::capture_pane(pane_id),
        Backend::Rex => rex::capture_pane(pane_id),
    }
}

/// Inside tmux, tws exits by switch-client before it draws again, so a flash
/// would never show. The tmux status line shows the message instead. True
/// when the message is shown; otherwise the caller flashes it.
pub fn notify_before_exit(msg: &str) -> bool {
    match current() {
        Backend::Tmux if tmux::is_inside_tmux() => tmux::display_message(msg).is_ok(),
        _ => false,
    }
}
