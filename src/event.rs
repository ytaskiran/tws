use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, MouseEvent, MouseEventKind};

pub enum Input {
    Key(KeyEvent),
    Mouse(MouseEvent),
}

pub fn poll(timeout: Duration) -> std::io::Result<Option<Input>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }
    Ok(match event::read()? {
        Event::Key(key) => Some(Input::Key(key)),
        // Without mouse capture, terminals send the wheel as arrow keys.
        // Keep that, so the wheel scrolls every list, modal and the notes.
        Event::Mouse(mouse) => Some(match mouse.kind {
            MouseEventKind::ScrollUp => Input::Key(KeyCode::Up.into()),
            MouseEventKind::ScrollDown => Input::Key(KeyCode::Down.into()),
            _ => Input::Mouse(mouse),
        }),
        _ => None,
    })
}

/// Drop queued input until it stays quiet for a moment. It was aimed at a
/// screen that is about to go away, and the terminal can still be sending
/// mouse reports from before capture went off.
// ponytail: fixed 50ms quiet window; a slow SSH link can still leak a late report.
pub fn discard_pending() -> std::io::Result<()> {
    while event::poll(Duration::from_millis(50))? {
        event::read()?;
    }
    Ok(())
}
