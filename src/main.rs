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
        /// Print the glyphs with no color (for the current tab)
        #[arg(long)]
        plain: bool,
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
            what: BarCommand::Window { plain, pane_ids },
        }) => {
            bar::window(&pane_ids, plain);
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
    let collections = persistence::load()?;
    let ui_state = persistence::load_ui();
    let state = AppState {
        collections,
        active_sessions: Vec::new(),
        agent_sessions: Vec::new(),
    };

    let cfg = config::load_config();
    let palette = config::resolve_palette(&cfg);
    let theme = theme::Theme::build(&palette);
    let note_stylesheet = theme::NoteStyleSheet::new(&palette);
    let keymap = config::build_keymap(&cfg);

    let mut terminal = tui::init()?;
    let mut app = App::new(state, theme, note_stylesheet, keymap);
    let result = app.run(&mut terminal, ui_state);
    tui::restore()?;
    result
}
