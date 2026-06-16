use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent, _host_idx: usize, _after_failure: bool) -> Result<()> {
    if key.code == crossterm::event::KeyCode::Esc { app.screen = Screen::Main; }
    Ok(())
}
