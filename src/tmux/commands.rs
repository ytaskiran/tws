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

/// Returns only sessions whose names start with "tws_".
pub fn list_tws_sessions() -> Vec<String> {
    list_sessions()
        .into_iter()
        .filter(|name| name.starts_with("tws_"))
        .collect()
}

/// Checks whether a tmux session with the given name exists.
pub fn has_session(name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
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

/// Returns true if we're currently running inside a tmux session.
pub fn is_inside_tmux() -> bool {
    std::env::var("TMUX").is_ok_and(|v| !v.is_empty())
}
