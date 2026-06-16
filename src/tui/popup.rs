use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let Screen::CredentialPicker { host_idx, after_failure } = &app.screen else { return };
    let host = &app.hosts[*host_idx];

    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    let title = if *after_failure {
        format!(" Auth failed — choose credentials for {} ", host.name)
    } else {
        format!(" Credentials for {} ", host.name)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    // Credential list
    let rows: Vec<Row> = app.credentials.iter().map(|c| {
        let kind = match c.kind {
            crate::types::CredentialKind::Key => "key",
            crate::types::CredentialKind::Password => "password",
        };
        let default_marker = if app.config.default_credential_id.as_deref() == Some(&c.id) { "★" } else { "" };
        Row::new(vec![
            Cell::from(c.name.clone()),
            Cell::from(kind).style(Style::default().fg(Color::DarkGray)),
            Cell::from(c.username.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(default_marker).style(Style::default().fg(Color::Yellow)),
        ])
    }).collect();

    let selected = app.popup_selected.min(app.credentials.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if app.credentials.is_empty() { None } else { Some(selected) });

    let table = Table::new(rows, [
        Constraint::Min(16),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(2),
    ])
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[0], &mut state);

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Blue)),
        Span::styled("=connect  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Blue)),
        Span::styled("=cancel", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[1]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}

pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent, host_idx: usize, _after_failure: bool) -> Result<()> {
    let creds_len = app.credentials.len();

    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if creds_len > 0 {
                app.popup_selected = (app.popup_selected + 1).min(creds_len - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.popup_selected = app.popup_selected.saturating_sub(1);
        }
        KeyCode::Enter if creds_len > 0 => {
            let cred = app.credentials[app.popup_selected.min(creds_len - 1)].clone();
            let host_name = app.hosts[host_idx].name.clone();
            app.screen = Screen::Main;
            crate::tui::do_connect(terminal, app, &host_name, &cred)?;
        }
        _ => {}
    }
    Ok(())
}
