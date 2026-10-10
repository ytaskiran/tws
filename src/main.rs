mod app;
mod bar;
mod components;
mod config;
mod core;
mod event;
mod fork;
mod import;
mod theme;
mod tmux;
mod tui;

use app::App;
use clap::{Parser, Subcommand};
use core::persistence;
use core::state::AppState;

#[derive(Parser)]
#[command(name = "tws", about = "tmux workspace manager", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Import existing tmux sessions into tws
    Import,
    /// Fork the Claude Code session running in a tmux pane (experimental)
    ForkPane {
        /// tmux pane id, e.g. %12
        pane_id: Option<String>,
    },
    /// Mark a pane as read when you move into it (called by a tmux hook)
    AckPane {
        /// tmux pane id, e.g. %12. Defaults to $TMUX_PANE
        pane_id: Option<String>,
    },
    /// Print text for the tmux status bar (called by a tmux #() job)
    Bar {
        #[command(subcommand)]
        what: BarCommand,
    },
}

#[derive(Subcommand)]
enum BarCommand {
    /// The agent glyphs of the panes in one window
    Window {
        /// tmux server start time (#{start_time}); older status files are skipped
        #[arg(long, default_value_t = 0)]
        since: i64,
        /// tmux pane ids, e.g. %12 %13
        pane_ids: Vec<String>,
    },
    /// "thread › session" for a tmux session
    Where {
        /// tmux session name
        session_name: String,
    },
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Import) => import::run(),
        Some(Command::ForkPane { pane_id }) => fork::run(pane_id.as_deref()),
        Some(Command::AckPane { pane_id }) => {
            ack_pane(pane_id);
            Ok(())
        }
        Some(Command::Bar {
            what: BarCommand::Window { since, pane_ids },
        }) => {
            bar::window(&pane_ids, since);
            Ok(())
        }
        Some(Command::Bar {
            what: BarCommand::Where { session_name },
        }) => {
            bar::session_label(&session_name);
            Ok(())
        }
        None => run_tui(),
    }
}

/// A tmux hook runs this on every focus change, so it never fails or prints.
fn ack_pane(pane_id: Option<String>) {
    if let Some(pane_id) = pane_id.or_else(|| std::env::var("TMUX_PANE").ok()) {
        core::status::ack_pane(&pane_id);
    }
}

fn run_tui() -> std::io::Result<()> {
    let threads = persistence::load()?;
    let ui_state = persistence::load_ui();
    let state = AppState {
        threads,
        active_sessions: Vec::new(),
        agent_sessions: Vec::new(),
    };

    let cfg = config::load_config();
    let keymap = config::build_keymap(&cfg);
    let theme_name = cfg.theme.unwrap_or_else(|| "default".to_string());

    // Before tui::init, so an unknown theme warning shows on a normal screen.
    let mut app = App::new(state, theme_name, cfg.palette, keymap);
    // Without this, a panic leaves the shell in raw mode, and each mouse move
    // prints an escape code into it.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = tui::restore();
        hook(info);
    }));
    let mut terminal = tui::init()?;
    let result = app.run(&mut terminal, ui_state);
    tui::restore()?;
    result
}
