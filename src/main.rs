mod app;
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
#[command(name = "tws", about = "tmux workspace manager")]
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
        pane_id: String,
    },
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Import) => import::run(),
        Some(Command::ForkPane { pane_id }) => fork::run(&pane_id),
        None => run_tui(),
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
