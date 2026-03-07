mod app;
mod components;
mod core;
mod event;
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
}

fn main() -> std::io::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Import) => import::run(),
        None => run_tui(),
    }
}

fn run_tui() -> std::io::Result<()> {
    let collections = persistence::load()?;
    let ui_state = persistence::load_ui();
    let state = AppState {
        collections,
        active_sessions: Vec::new(),
    };

    let mut terminal = tui::init()?;
    let mut app = App::new(state);
    let result = app.run(&mut terminal, ui_state);
    tui::restore()?;
    result
}
