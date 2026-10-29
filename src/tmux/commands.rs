use std::path::Path;
use std::process::Command;

/// False when tmux cannot start at all, for example when it is not installed.
pub fn installed() -> bool {
    Command::new("tmux").arg("-V").output().is_ok()
}

pub fn list_sessions_with_timestamps() -> Vec<(String, i64)> {
    let output = Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_last_attached}",
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    let (name, ts_str) = line.split_once('\t')?;
                    let ts = ts_str.parse::<i64>().unwrap_or(0);
                    Some((name.to_string(), ts))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub fn new_session(name: &str, cwd: Option<&Path>) -> std::io::Result<bool> {
    let mut cmd = Command::new("tmux");
    cmd.args(["new-session", "-d", "-s", name]);
    if let Some(dir) = cwd {
        cmd.arg("-c").arg(dir);
    }
    let status = cmd.status()?;
    Ok(status.success())
}

/// An exact session target. A bare `-t api` falls back to a prefix match, so
/// with `api` gone it would hit `api-v2`.
fn exact(name: &str) -> String {
    format!("={name}")
}

pub fn kill_session(name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["kill-session", "-t", &exact(name)])
        .status()?;
    Ok(status.success())
}

pub fn rename_session(old_name: &str, new_name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["rename-session", "-t", &exact(old_name), new_name])
        .status()?;
    Ok(status.success())
}

/// Switches the current client without blocking; only works inside tmux.
pub fn switch_client(name: &str) -> std::io::Result<bool> {
    let output = Command::new("tmux")
        .args(["switch-client", "-t", &exact(name)])
        .output()?;
    Ok(output.status.success())
}

/// Attaches outside tmux and blocks until the user detaches.
pub fn attach_session(name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["attach-session", "-t", &exact(name)])
        .status()?;
    Ok(status.success())
}

/// Selects a window across sessions without requiring an attached client.
pub fn select_window(session_name: &str, window_index: u32) -> std::io::Result<bool> {
    let target = format!("{}:{}", exact(session_name), window_index);
    let output = Command::new("tmux")
        .args(["select-window", "-t", &target])
        .output()?;
    Ok(output.status.success())
}

/// Selects a pane by global ID without requiring an attached client.
pub fn select_pane(pane_id: &str) -> std::io::Result<bool> {
    let output = Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
        .output()?;
    Ok(output.status.success())
}

/// Returns the active pane tmux uses when attaching to a session.
///
/// An empty result is treated as missing because `display-message` can exit
/// successfully for an unknown target.
pub fn active_pane(session_name: &str) -> Option<String> {
    let output = Command::new("tmux")
        // A pane target: `=name` alone finds no pane, `=name:` finds the active one.
        .args([
            "display-message",
            "-p",
            "-t",
            &format!("{}:", exact(session_name)),
            "#{pane_id}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let pane = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if pane.is_empty() { None } else { Some(pane) }
}

/// Shows a transient message in the attached client's status line. Used when
/// tws is about to exit via switch-client, where its own status bar would
/// never be painted.
pub fn display_message(msg: &str) -> std::io::Result<bool> {
    let output = Command::new("tmux")
        .args(["display-message", msg])
        .output()?;
    Ok(output.status.success())
}

/// Captures visible pane content, preserving ANSI escape sequences.
pub fn capture_pane(pane_id: &str) -> Option<String> {
    let output = Command::new("tmux")
        .args(["capture-pane", "-t", pane_id, "-e", "-p"])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        None
    }
}

pub fn is_inside_tmux() -> bool {
    std::env::var("TMUX").is_ok_and(|v| !v.is_empty())
}
