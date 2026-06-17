use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Term};
use crate::types::DeleteKind;

pub fn draw(f: &mut Frame, app: &App) {
    let Some(popup) = &app.delete_popup else { return };

    let area = centered_rect(50, 40, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // message
            Constraint::Length(2), // checkbox
            Constraint::Length(1), // hotkeys
        ])
        .margin(1)
        .split(inner);

    let kind_str = match popup.kind {
        DeleteKind::Host => "host",
        DeleteKind::Credential => "credential",
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Delete ", Style::default().fg(Color::White)),
            Span::styled(kind_str, Style::default().fg(Color::DarkGray)),
            Span::styled(" '", Style::default().fg(Color::White)),
            Span::styled(&popup.name, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled("'?", Style::default().fg(Color::White)),
        ])),
        chunks[0],
    );

    let check = if popup.dont_ask { "[x]" } else { "[ ]" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(check, Style::default().fg(Color::Blue)),
            Span::styled(" don't ask again this session", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[1],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Y]", Style::default().fg(Color::Red)),
            Span::styled(" delete  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[N/Esc]", Style::default().fg(Color::Blue)),
            Span::styled(" cancel  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Space]", Style::default().fg(Color::Blue)),
            Span::styled(" toggle", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[2],
    );
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(popup) = app.delete_popup.take() {
                if popup.dont_ask {
                    app.skip_delete_confirm = true;
                }
                do_delete(app, &popup)?;
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            app.delete_popup = None;
        }
        KeyCode::Char(' ') => {
            if let Some(ref mut p) = app.delete_popup {
                p.dont_ask = !p.dont_ask;
            }
        }
        _ => {}
    }
    Ok(())
}

fn do_delete(app: &mut App, popup: &crate::types::DeletePopup) -> Result<()> {
    match popup.kind {
        DeleteKind::Host => {
            if popup.idx < app.hosts.len() {
                app.hosts.remove(popup.idx);
                app.save_hosts()?;
                app.selected_row = app.selected_row.min(app.hosts.len().saturating_sub(1));
                app.status_message = Some(format!("Host '{}' deleted.", popup.name));
            }
        }
        DeleteKind::Credential => {
            if popup.idx < app.credentials.len() {
                let id = app.credentials[popup.idx].id.clone();
                crate::credentials::delete_credential(&id)?;
                if app.config.default_credential_id.as_deref() == Some(&id) {
                    app.config.default_credential_id = None;
                    crate::config::save_config(&app.config)?;
                }
                app.reload_credentials()?;
                app.cred_selected = app.cred_selected.saturating_sub(1);
                app.status_message = Some(format!("Credential '{}' deleted.", popup.name));
            }
        }
    }
    Ok(())
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
