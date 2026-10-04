//! `tws bar`: text for the tmux status bar, printed by a `#()` job.
//!
//! tmux puts every byte of stdout on the bar, so these commands never fail and
//! print no errors. tmux drops the stderr of a `#()` job.

use std::path::Path;

use ratatui::style::Color;

use crate::config::palette::Palette;
use crate::config::{self, Config};
use crate::core::model::{AgentStatus, Collection};
use crate::core::state::AppState;
use crate::core::{persistence, status};

/// Prints the glyphs of the agent panes in one window. tmux gives the panes
/// with `#{P:#{pane_id} }`. `plain` leaves out the colors, for the current
/// tab, whose background tws does not know.
pub fn window(pane_ids: &[String], plain: bool) {
    let palette = (!plain).then(palette);
    print!(
        "{}",
        window_glyphs(&status::agents_dir(), pane_ids, palette.as_ref())
    );
}

/// Prints `thread › session` for a tws session, else the session name.
pub fn session_label(session_name: &str) {
    let collections = persistence::load().unwrap_or_default();
    print!("{}", label_for(collections, session_name));
}

/// tmux reads `#` in job output as the start of a format, so each `#` in a
/// name is doubled.
fn label_for(collections: Vec<Collection>, session_name: &str) -> String {
    let mut state = AppState {
        collections,
        active_sessions: Vec::new(),
        agent_sessions: Vec::new(),
    };
    state.refresh_sessions(&[(session_name.to_string(), 0)]);
    let label = state
        .active_sessions
        .first()
        .and_then(|s| {
            let (_, thread) = state.resolve_thread_path(s.thread_id)?;
            Some(format!("{thread} › {}", s.display_name))
        })
        .unwrap_or_else(|| session_name.to_string());
    label.replace('#', "##")
}

/// A space, then one glyph for each pane with a status file, in argument
/// order. Empty when no pane has one. No color reset at the end: the glyphs
/// are the last visible text of the tab.
fn window_glyphs(dir: &Path, pane_ids: &[String], palette: Option<&Palette>) -> String {
    let mut out = String::new();
    for id in pane_ids.iter().filter(|id| is_pane_id(id)) {
        let Ok(word) = std::fs::read_to_string(dir.join(id)) else {
            continue;
        };
        let st = status::parse_status(&word);
        if let Some(p) = palette {
            out.push_str(&format!("#[fg={}]", hex(status_color(p, st))));
        }
        out.push_str(status::status_glyph(st));
    }
    if out.is_empty() {
        out
    } else {
        format!(" {out}")
    }
}

/// A tmux pane ID is `%` and digits. Any other text could name a file
/// outside the status directory.
fn is_pane_id(s: &str) -> bool {
    s.strip_prefix('%')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The same colors as the TUI rows (`theme.rs`).
fn status_color(p: &Palette, st: AgentStatus) -> Color {
    match st {
        AgentStatus::Working => p.green,
        AgentStatus::Waiting | AgentStatus::Review => p.accent,
        AgentStatus::Idle | AgentStatus::Unknown => p.muted,
    }
}

fn hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => "default".to_string(),
    }
}

/// Like `config::load_config`, but a bad `config.toml` gives the default
/// palette, not an exit: the bar has no place to show an error.
fn palette() -> Palette {
    let cfg: Config = std::fs::read_to_string(persistence::config_dir().join("config.toml"))
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    config::resolve_palette(&cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Thread;
    use uuid::Uuid;

    fn status_dir(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tws-test-bar-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        for (name, word) in files {
            std::fs::write(dir.join(name), word).unwrap();
        }
        dir
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn window_with_no_agent_prints_nothing() {
        let dir = status_dir("none", &[]);
        assert_eq!(window_glyphs(&dir, &ids(&["%1", "%2"]), None), "");
    }

    #[test]
    fn window_plain_keeps_argument_order() {
        let dir = status_dir(
            "plain",
            &[("%1", "working"), ("%3", "review\n"), ("%4", "idle")],
        );
        assert_eq!(
            window_glyphs(&dir, &ids(&["%4", "%2", "%1", "%3"]), None),
            " ○●◐"
        );
    }

    #[test]
    fn window_colors_come_from_the_palette() {
        let dir = status_dir(
            "color",
            &[("%1", "working"), ("%2", "waiting"), ("%3", "idle")],
        );
        let p = Palette::default();
        let want = format!(
            " #[fg={}]●#[fg={}]◐#[fg={}]○",
            hex(p.green),
            hex(p.accent),
            hex(p.muted)
        );
        assert_eq!(
            window_glyphs(&dir, &ids(&["%1", "%2", "%3"]), Some(&p)),
            want
        );
    }

    #[test]
    fn window_skips_arguments_that_are_not_pane_ids() {
        let dir = status_dir("ids", &[("%1", "working"), ("x", "working")]);
        std::fs::write(dir.join("..%1"), "working").unwrap();
        let args = ids(&["x", "../x", "%", "%1a", "..%1", "%1"]);
        assert_eq!(window_glyphs(&dir, &args, None), " ●");
    }

    #[test]
    fn hex_prints_rgb() {
        assert_eq!(hex(Color::Rgb(0x82, 0xb4, 0x02)), "#82b402");
    }

    fn col(name: &str, is_root: bool, threads: &[&str]) -> Collection {
        Collection {
            id: Uuid::new_v4(),
            name: name.to_string(),
            is_root,
            threads: threads
                .iter()
                .map(|t| Thread {
                    id: Uuid::new_v4(),
                    name: t.to_string(),
                    description: None,
                    working_dir: None,
                })
                .collect(),
        }
    }

    #[test]
    fn where_root_thread() {
        let cols = vec![col("root", true, &["tws"])];
        assert_eq!(label_for(cols, "twsr_tws_status-bar"), "tws › status-bar");
    }

    #[test]
    fn where_never_shows_the_collection() {
        let cols = vec![col("Work", false, &["api"])];
        assert_eq!(label_for(cols, "tws_work_api_main"), "api › main");
    }

    #[test]
    fn where_unknown_session_keeps_its_name() {
        assert_eq!(label_for(Vec::new(), "scratch"), "scratch");
    }

    #[test]
    fn where_escapes_the_tmux_format_character() {
        let cols = vec![col("root", true, &["C# work"])];
        assert_eq!(label_for(cols, "twsr_c-work_main"), "C## work › main");
        assert_eq!(label_for(Vec::new(), "a#[fg=red]"), "a##[fg=red]");
    }
}
