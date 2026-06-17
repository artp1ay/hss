use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Term};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn draw_import(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(_terminal: &mut Term, app: &mut App, _key: KeyEvent) -> Result<()> {
    let _ = app;
    Ok(())
}
pub fn handle_import_key(_terminal: &mut Term, app: &mut App, _key: KeyEvent) -> Result<()> {
    let _ = app;
    Ok(())
}
