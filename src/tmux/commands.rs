use std::process::Command;

/// Returns the names of all running tmux sessions.
/// Returns an empty Vec if the tmux server isn't running.
pub fn list_sessions() -> Vec<String> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect()
        }
        // tmux returns error when no server is running — that's fine
        _ => Vec::new(),
    }
}

/// Returns tws-prefixed sessions with their `last_attached` Unix timestamps.
/// Each entry is `(session_name, last_attached_timestamp)`.
pub fn list_tws_sessions_with_timestamps() -> Vec<(String, i64)> {
    let output = Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}\t#{session_last_attached}"])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout
                .lines()
                .filter_map(|line| {
                    let (name, ts_str) = line.split_once('\t')?;
                    if !name.starts_with("tws_") && !name.starts_with("twsr_") {
                        return None;
                    }
                    let ts = ts_str.parse::<i64>().unwrap_or(0);
                    Some((name.to_string(), ts))
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Creates a new detached tmux session with the given name.
pub fn new_session(name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["new-session", "-d", "-s", name])
        .status()?;
    Ok(status.success())
}

/// Kills the tmux session with the given name.
pub fn kill_session(name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["kill-session", "-t", name])
        .status()?;
    Ok(status.success())
}

/// Renames a tmux session.
pub fn rename_session(old_name: &str, new_name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["rename-session", "-t", old_name, new_name])
        .status()?;
    Ok(status.success())
}

/// Switches the current tmux client to the given session.
/// Non-blocking — only works when already inside tmux.
pub fn switch_client(name: &str) -> std::io::Result<bool> {
    let output = Command::new("tmux")
        .args(["switch-client", "-t", name])
        .output()?;
    Ok(output.status.success())
}

/// Attaches to the given tmux session, inheriting stdio.
/// **Blocks** until the user detaches. Only use outside tmux.
pub fn attach_session(name: &str) -> std::io::Result<bool> {
    let status = Command::new("tmux")
        .args(["attach-session", "-t", name])
        .status()?;
    Ok(status.success())
}

/// Selects the given window in the target session.
/// Works across sessions — doesn't require being attached to that session.
pub fn select_window(session_name: &str, window_index: u32) -> std::io::Result<bool> {
    let target = format!("{}:{}", session_name, window_index);
    let output = Command::new("tmux")
        .args(["select-window", "-t", &target])
        .output()?;
    Ok(output.status.success())
}

/// Selects the given pane (by global pane ID like "%5").
/// Works across sessions — doesn't require being attached to that session.
pub fn select_pane(pane_id: &str) -> std::io::Result<bool> {
    let output = Command::new("tmux")
        .args(["select-pane", "-t", pane_id])
        .output()?;
    Ok(output.status.success())
}

/// Captures the visible content of a tmux pane, including ANSI escape sequences.
/// Returns `None` if the pane doesn't exist or the command fails.
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

/// Returns true if we're currently running inside a tmux session.
pub fn is_inside_tmux() -> bool {
    std::env::var("TMUX").is_ok_and(|v| !v.is_empty())
}
