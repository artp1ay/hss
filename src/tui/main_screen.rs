use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Term};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if key.code == crossterm::event::KeyCode::Char('q') { app.should_quit = true; }
    let _ = (terminal, key);
    Ok(())
}
