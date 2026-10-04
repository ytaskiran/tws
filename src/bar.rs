//! `tws bar`: text for the tmux status bar, printed by a `#()` job.
//!
//! tmux puts every byte of stdout on the bar, so these commands never fail and
//! print no errors. tmux drops the stderr of a `#()` job.

use std::path::Path;

use ratatui::style::Color;

use crate::config;
use crate::config::palette::Palette;
use crate::core::model::{AgentStatus, Collection};
use crate::core::state::AppState;
use crate::core::{persistence, status};

/// Prints the glyphs of the agent panes in one window. tmux gives the panes
/// with `#{P:#{pane_id} }`, and the server start time with `#{start_time}`.
pub fn window(pane_ids: &[String], server_start: i64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let states = window_states(&persistence::config_dir(), pane_ids, server_start, now);
    print!("{}", window_label(&states, &palette()));
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

/// The state of each pane with a status file, in argument order.
///
/// The TUI cleans the status files, so with the TUI closed two kinds of
/// files are stale. A file older than the tmux server belongs to a pane of
/// an earlier server that had the same ID, and is skipped. A `working` file
/// with no sign of life counts as idle, the change the TUI would write.
fn window_states(
    config_dir: &Path,
    pane_ids: &[String],
    server_start: i64,
    now: i64,
) -> Vec<AgentStatus> {
    let agents = config_dir.join("agents");
    let mut out = Vec::new();
    for id in pane_ids.iter().filter(|id| is_pane_id(id)) {
        let path = agents.join(id);
        let (Ok(word), Ok(meta)) = (std::fs::read_to_string(&path), std::fs::metadata(&path))
        else {
            continue;
        };
        let mtime = status::mtime_secs(&meta);
        if mtime < server_start {
            continue;
        }
        let mut st = status::parse_status(&word);
        if st == AgentStatus::Working
            && status::working_is_stale(
                id,
                mtime,
                &config_dir.join("inflight"),
                &config_dir.join("heartbeat"),
                now,
            )
        {
            st = AgentStatus::Idle;
        }
        out.push(st);
    }
    out
}

/// A space, then the glyphs in two versions. tmux expands the format in the
/// job output for each tab, so it picks the darker tones for the current tab.
/// The command string is then the same for every tab, and a window change
/// starts no new job. Empty when no pane has an agent. No color reset at the
/// end: the glyphs are the last visible text of the tab.
fn window_label(states: &[AgentStatus], palette: &Palette) -> String {
    if states.is_empty() {
        return String::new();
    }
    format!(
        " #{{?window_active,{},{}}}",
        glyph_run(states, &dark(palette)),
        glyph_run(states, palette)
    )
}

fn glyph_run(states: &[AgentStatus], palette: &Palette) -> String {
    states
        .iter()
        .map(|&st| format!("#[fg={}]{}", hex(status_color(palette, st)), glyph(st)))
        .collect()
}

/// A tmux pane ID is `%` and digits. Any other text could name a file
/// outside the status directory.
fn is_pane_id(s: &str) -> bool {
    s.strip_prefix('%')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `●` for a live turn, `○` for idle. Most coding fonts have both, but not
/// `◐`: a terminal takes `◐` from a fallback font, and then the glyphs do not
/// line up. The color tells waiting from working.
fn glyph(st: AgentStatus) -> &'static str {
    match st {
        AgentStatus::Working | AgentStatus::Waiting | AgentStatus::Review => "●",
        AgentStatus::Idle | AgentStatus::Unknown => "○",
    }
}

/// The palette at 90% brightness, for glyphs on a bright tab. A lower value
/// makes the green and orange look grey on the tab.
fn dark(p: &Palette) -> Palette {
    let d = |c: Color| match c {
        Color::Rgb(r, g, b) => {
            let f = |v: u8| (v as u16 * 90 / 100) as u8;
            Color::Rgb(f(r), f(g), f(b))
        }
        other => other,
    };
    Palette {
        green: d(p.green),
        accent: d(p.accent),
        muted: d(p.muted),
        ..p.clone()
    }
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

/// A bad `config.toml` gives the default palette, not an exit: the bar has
/// no place to show an error.
fn palette() -> Palette {
    config::resolve_palette(&config::try_load_config().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::Thread;
    use uuid::Uuid;

    /// A config dir with `agents/`; each file gets the current mtime.
    fn status_dir(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tws-test-bar-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(dir.join("agents")).unwrap();
        for (name, word) in files {
            std::fs::write(dir.join("agents").join(name), word).unwrap();
        }
        dir
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn age(path: &std::path::Path, secs: u64) {
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(secs);
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn states(dir: &std::path::Path, args: &[&str]) -> Vec<AgentStatus> {
        window_states(dir, &ids(args), 0, now())
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    use AgentStatus::{Idle, Review, Waiting, Working};

    #[test]
    fn window_with_no_agent_prints_nothing() {
        let dir = status_dir("none", &[]);
        assert!(states(&dir, &["%1", "%2"]).is_empty());
        assert_eq!(window_label(&[], &Palette::default()), "");
    }

    #[test]
    fn window_keeps_argument_order() {
        let dir = status_dir(
            "order",
            &[("%1", "working"), ("%3", "review\n"), ("%4", "idle")],
        );
        assert_eq!(
            states(&dir, &["%4", "%2", "%1", "%3"]),
            [Idle, Working, Review]
        );
    }

    #[test]
    fn glyph_run_uses_only_glyphs_from_one_font() {
        // ◐ comes from a fallback font in many terminals, and then it does not
        // line up with ● and ○. The color tells waiting from working.
        let p = Palette::default();
        let run = glyph_run(&[Working, Waiting, Review, Idle], &p);
        let want = format!(
            "#[fg={g}]●#[fg={a}]●#[fg={a}]●#[fg={m}]○",
            g = hex(p.green),
            a = hex(p.accent),
            m = hex(p.muted)
        );
        assert_eq!(run, want);
    }

    #[test]
    fn active_tab_uses_dark_tones_of_the_palette() {
        let d = dark(&Palette::default());
        assert_eq!(d.green, Color::Rgb(0x75, 0xa2, 0x75));
        assert_eq!(d.accent, Color::Rgb(0xb7, 0x6c, 0x2d));
    }

    #[test]
    fn window_label_picks_the_tones_in_tmux() {
        // One command string for the current tab and the other tabs, so a
        // window change does not start a new job and the glyphs do not blink.
        let p = Palette::default();
        let label = window_label(&[Working, Idle], &p);
        let want = format!(
            " #{{?window_active,{},{}}}",
            glyph_run(&[Working, Idle], &dark(&p)),
            glyph_run(&[Working, Idle], &p)
        );
        assert_eq!(label, want);
        // A `,` or `}` in a run would end the tmux conditional early.
        let all = glyph_run(&[Working, Waiting, Review, Idle], &p);
        assert!(!all.contains([',', '}']));
    }

    #[test]
    fn window_skips_arguments_that_are_not_pane_ids() {
        let dir = status_dir("ids", &[("%1", "working"), ("x", "working")]);
        std::fs::write(dir.join("agents").join("..%1"), "working").unwrap();
        let args = ["x", "../x", "%", "%1a", "..%1", "%1"];
        assert_eq!(states(&dir, &args), [Working]);
    }

    #[test]
    fn window_skips_files_from_before_the_server_start() {
        // tmux numbers panes from %0 again after a restart.
        let dir = status_dir("restart", &[("%0", "review"), ("%1", "working")]);
        age(&dir.join("agents/%0"), 600);
        let out = window_states(&dir, &ids(&["%0", "%1"]), now() - 60, now());
        assert_eq!(out, [Working]);
    }

    #[test]
    fn window_shows_a_stale_working_pane_as_idle() {
        let dir = status_dir("stale", &[("%1", "working"), ("%2", "working")]);
        let old = status::STALE_WORKING_SECS as u64 + 60;
        age(&dir.join("agents/%1"), old);
        age(&dir.join("agents/%2"), old);
        std::fs::create_dir_all(dir.join("heartbeat")).unwrap();
        std::fs::write(dir.join("heartbeat/%2"), "").unwrap();
        assert_eq!(states(&dir, &["%1", "%2"]), [Idle, Working]);
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
