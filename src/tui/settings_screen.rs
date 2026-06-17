use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("hss", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" · settings", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    // Inventory path field
    let inv_focused = app.settings_focused_field == 0;
    let border_style = if inv_focused { Style::default().fg(Color::Blue) } else { Style::default().fg(Color::DarkGray) };
    let inv_display = format!(
        "{}{}",
        app.settings_inventory_input,
        if inv_focused { "█" } else { "" }
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("ANSIBLE INVENTORY PATH", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM))),
            Line::from(Span::styled(inv_display, Style::default().fg(Color::White))),
            Line::from(Span::styled("Read-only source of host data", Style::default().fg(Color::DarkGray))),
        ])
        .block(Block::default().borders(Borders::ALL).border_style(border_style)),
        chunks[1],
    );

    // Default credential (read-only display)
    let default_name = app.config.default_credential_id.as_deref()
        .and_then(|id| app.credentials.iter().find(|c| c.id == id))
        .map(|c| format!("★ {} ({})", c.name, c.username))
        .unwrap_or_else(|| "not set — go to Credentials to set".into());
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("DEFAULT CREDENTIAL", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM))),
            Line::from(Span::styled(default_name, Style::default().fg(Color::Yellow))),
        ])
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray))),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Blue)),
            Span::styled("=save  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::Blue)),
            Span::styled("=back", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[4],
    );
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        KeyCode::Enter => {
            crate::config::save_config(&app.config)?;
            app.screen = Screen::Main;
        }
        _ => {}
    }
    Ok(())
}
