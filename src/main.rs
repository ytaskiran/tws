mod action;
mod app;
mod components;
mod core;
mod event;
mod theme;
mod tmux;
mod tui;

use app::App;
use core::persistence;
use core::state::AppState;

fn main() -> std::io::Result<()> {
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
