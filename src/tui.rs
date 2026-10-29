use std::io::{self, Stdout, stdout};

use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, prelude::CrosstermBackend};

use crate::event;

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

pub fn init() -> io::Result<Tui> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(stdout()))
}

/// Mouse capture goes off while raw mode is still on. In cooked mode the
/// tty echoes mouse reports, into the shell or into `tmux attach`.
pub fn restore() -> io::Result<()> {
    let screen = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
    let drained = event::discard_pending();
    disable_raw_mode()?;
    screen.and(drained)
}

/// The Rex popup block closes when tws exits, so no shell is left to get a
/// late mouse report. The 50 ms drain of `restore` is only a wait there.
pub fn restore_popup() -> io::Result<()> {
    let screen = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
    disable_raw_mode()?;
    screen
}
