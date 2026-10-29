mod app;
mod backend;
mod bar;
mod components;
mod config;
mod core;
mod event;
mod fork;
mod import;
mod rex;
mod theme;
mod tmux;
mod tui;

use app::App;
use clap::{Parser, Subcommand};
use core::persistence;
use core::state::AppState;

#[derive(Parser)]
#[command(name = "tws", about = "terminal workspace manager", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Quit after a session opens. The Rex open-tws key uses it for its popup.
    #[arg(long)]
    popup: bool,
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
        Some(Command::Import) if backend::is_rex() => {
            eprintln!(
                "tws import works with tmux only. To copy tmux sessions into Rex, \
                 run extras/rex/tmux2rex.py."
            );
            std::process::exit(1);
        }
        Some(Command::Import) => import::run(),
        Some(Command::ForkPane { .. }) if backend::is_rex() => {
            eprintln!("tws fork-pane does not work with the Rex backend.");
            std::process::exit(1);
        }
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
        None => run_tui(cli.popup),
    }
}

/// A tmux hook runs this on every focus change, so it never fails or prints.
fn ack_pane(pane_id: Option<String>) {
    if let Some(pane_id) = pane_id.or_else(|| std::env::var("TMUX_PANE").ok()) {
        core::status::ack_pane(&pane_id);
    }
}

fn run_tui(popup: bool) -> std::io::Result<()> {
    // First, so a config that does not parse stops here with its own error.
    // Otherwise backend::current() reads it as the default backend.
    let cfg = config::load_config();
    // Without this check, tws opens and then stops with a raw OS error at the
    // first session that it tries to start.
    if !backend::installed() {
        eprintln!("{}", backend::missing_message());
        std::process::exit(1);
    }
    let threads = persistence::load()?;
    let ui_state = persistence::load_ui();
    let state = AppState {
        threads,
        active_sessions: Vec::new(),
        agent_sessions: Vec::new(),
    };

    let keymap = config::build_keymap(&cfg);
    let theme_name = cfg.theme.unwrap_or_else(|| "default".to_string());

    // Before tui::init, so an unknown theme warning shows on a normal screen.
    let mut app = App::new(state, theme_name, cfg.palette, cfg.glass, keymap);
    app.popup = popup;
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
