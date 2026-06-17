use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen, Term};
use crate::types::{CredentialForm, CredentialKind};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("hss", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" · credentials", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let header = Row::new(vec!["NAME", "TYPE", "USERNAME", "DEFAULT"])
        .style(Style::default().fg(Color::DarkGray));

    let rows: Vec<Row> = app.credentials.iter().map(|c| {
        let kind_str = if c.kind == CredentialKind::Key { "key" } else { "password" };
        let default_marker = if app.config.default_credential_id.as_deref() == Some(&c.id) {
            "★ default"
        } else {
            ""
        };
        Row::new(vec![
            Cell::from(c.name.clone()),
            Cell::from(kind_str).style(Style::default().fg(Color::DarkGray)),
            Cell::from(c.username.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(default_marker).style(Style::default().fg(Color::Yellow)),
        ])
    }).collect();

    let selected = app.cred_selected.min(app.credentials.len().saturating_sub(1));
    let mut state = TableState::default()
        .with_selected(if app.credentials.is_empty() { None } else { Some(selected) });

    let table = Table::new(rows, [
        Constraint::Min(18),
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Length(10),
    ])
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[1], &mut state);

    f.render_widget(
        Paragraph::new(hotkey_line(&[("A", "add"), ("E", "edit"), ("D", "delete"), ("*", "default"), ("Esc", "back")])),
        chunks[2],
    );

    // Draw form overlay if active
    if let Some(ref form) = app.cred_form {
        draw_form(f, form, f.area());
    }
}

fn draw_form(f: &mut Frame, form: &CredentialForm, area: Rect) {
    let popup = centered_rect(62, 70, area);
    f.render_widget(Clear, popup);

    let title = if form.editing_id.is_some() { " Edit credential " } else { " New credential " };
    let block = Block::default().title(title).borders(Borders::ALL).border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // type toggle
            Constraint::Length(2), // name
            Constraint::Length(2), // username
            Constraint::Length(2), // password or key_path
            Constraint::Length(1), // hotkeys
        ])
        .margin(1)
        .split(inner);

    // Type toggle
    let (pw_style, key_style) = if form.is_key {
        (Style::default().fg(Color::DarkGray), Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
    } else {
        (Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD), Style::default().fg(Color::DarkGray))
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Password]", pw_style),
            Span::raw("  "),
            Span::styled("[Key]", key_style),
        ])),
        chunks[0],
    );

    // Fields
    render_field(f, "Name:    ", &form.name, form.focused == 1, chunks[1]);
    render_field(f, "Username:", &form.username, form.focused == 2, chunks[2]);
    if form.is_key {
        render_field(f, "Key path:", &form.key_path, form.focused == 3, chunks[3]);
    } else {
        let masked: String = "•".repeat(form.password.len());
        let pw_label = if form.editing_id.is_some() { "Password: (blank=keep)" } else { "Password:" };
        render_field(f, pw_label, &masked, form.focused == 3, chunks[3]);
    }

    f.render_widget(
        Paragraph::new(hotkey_line(&[("Tab", "next"), ("Space", "toggle type"), ("Enter", "save"), ("Esc", "cancel")])),
        chunks[4],
    );
}

fn render_field(f: &mut Frame, label: &str, value: &str, focused: bool, area: Rect) {
    let border_style = if focused { Style::default().fg(Color::Blue) } else { Style::default().fg(Color::DarkGray) };
    let display = if focused { format!("{value}█") } else { value.to_string() };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);
    f.render_widget(Paragraph::new(label).style(Style::default().fg(Color::DarkGray)), chunks[0]);
    f.render_widget(
        Paragraph::new(display).block(Block::default().borders(Borders::BOTTOM).border_style(border_style)),
        chunks[1],
    );
}

fn hotkey_line<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = vec![];
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 { spans.push(Span::raw("  ")); }
        spans.push(Span::styled(format!("[{key}]"), Style::default().fg(Color::Blue)));
        spans.push(Span::styled(format!(" {label}"), Style::default().fg(Color::DarkGray)));
    }
    Line::from(spans)
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

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if app.cred_form.is_some() {
        let form = app.cred_form.clone().unwrap();
        handle_form_key(app, key, form)?;
    } else {
        handle_list_key(app, key)?;
    }
    Ok(())
}

fn handle_list_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let creds_len = app.credentials.len();
    match key.code {
        KeyCode::Esc => { app.screen = Screen::Main; }
        KeyCode::Down | KeyCode::Char('j') => {
            if creds_len > 0 { app.cred_selected = (app.cred_selected + 1).min(creds_len - 1); }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.cred_selected = app.cred_selected.saturating_sub(1);
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.cred_form = Some(CredentialForm::default());
        }
        KeyCode::Char('e') | KeyCode::Char('E') if creds_len > 0 => {
            let c = &app.credentials[app.cred_selected.min(creds_len - 1)];
            app.cred_form = Some(CredentialForm {
                editing_id: Some(c.id.clone()),
                is_key: c.kind == CredentialKind::Key,
                name: c.name.clone(),
                username: c.username.clone(),
                key_path: c.key_path.clone().unwrap_or_default(),
                password: String::new(),
                focused: 1,
            });
        }
        KeyCode::Char('d') | KeyCode::Char('D') if creds_len > 0 => {
            let idx = app.cred_selected.min(creds_len - 1);
            let name = app.credentials[idx].name.clone();
            if app.skip_delete_confirm {
                let id = app.credentials[idx].id.clone();
                crate::credentials::delete_credential(&id)?;
                if app.config.default_credential_id.as_deref() == Some(&id) {
                    app.config.default_credential_id = None;
                    crate::config::save_config(&app.config)?;
                }
                app.reload_credentials()?;
                app.cred_selected = app.cred_selected.saturating_sub(1);
            } else {
                app.delete_popup = Some(crate::types::DeletePopup {
                    kind: crate::types::DeleteKind::Credential,
                    name,
                    idx,
                    dont_ask: false,
                });
            }
        }
        KeyCode::Char('*') if creds_len > 0 => {
            let id = app.credentials[app.cred_selected.min(creds_len - 1)].id.clone();
            app.config.default_credential_id = Some(id);
            crate::config::save_config(&app.config)?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_form_key(app: &mut App, key: KeyEvent, mut form: CredentialForm) -> Result<()> {
    match key.code {
        KeyCode::Esc => { app.cred_form = None; }
        KeyCode::Tab => {
            form.focused = (form.focused + 1) % 4;
            app.cred_form = Some(form);
        }
        KeyCode::Char(' ') if form.focused == 0 => {
            form.is_key = !form.is_key;
            app.cred_form = Some(form);
        }
        KeyCode::Enter => {
            save_form(app, &form)?;
            app.cred_form = None;
        }
        KeyCode::Backspace => {
            match form.focused {
                1 => { form.name.pop(); }
                2 => { form.username.pop(); }
                3 if form.is_key => { form.key_path.pop(); }
                3 => { form.password.pop(); }
                _ => {}
            }
            app.cred_form = Some(form);
        }
        KeyCode::Char(c) => {
            match form.focused {
                1 => form.name.push(c),
                2 => form.username.push(c),
                3 if form.is_key => form.key_path.push(c),
                3 => form.password.push(c),
                _ => {}
            }
            app.cred_form = Some(form);
        }
        _ => {}
    }
    Ok(())
}

fn save_form(app: &mut App, form: &CredentialForm) -> Result<()> {
    let kind = if form.is_key { CredentialKind::Key } else { CredentialKind::Password };
    let key_path = if form.is_key && !form.key_path.is_empty() { Some(form.key_path.clone()) } else { None };
    let password = if !form.is_key && !form.password.is_empty() { Some(form.password.as_str()) } else { None };

    if let Some(ref id) = form.editing_id {
        crate::credentials::update_credential(id, &form.name, &form.username, kind, key_path, password)?;
    } else {
        crate::credentials::add_credential(&form.name, &form.username, kind, key_path, password)?;
    }
    app.reload_credentials()?;
    Ok(())
}
