use crate::tui::{App, Screen, Term};
use crate::types::{CredentialForm, CredentialKind, StatusKind, StatusMessage};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let hotkeys = &[
        ("A", "add"),
        ("E", "edit"),
        ("D", "delete"),
        ("*/S", "default"),
        ("Esc", "back"),
    ];
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkeys, area.width);
    let hotkey_height = hotkey_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(if app.status_message.is_some() { 2 } else { 0 }),
            Constraint::Length(hotkey_height),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "hss",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · credentials", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let header = Row::new(vec!["NAME", "TYPE", "USERNAME", "DEFAULT"])
        .style(Style::default().fg(Color::DarkGray));

    let rows: Vec<Row> = app
        .credentials
        .iter()
        .map(|c| {
            let kind_str = if c.kind == CredentialKind::Key {
                "key"
            } else {
                "password"
            };
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
        })
        .collect();

    let selected = app
        .cred_selected
        .min(app.credentials.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if app.credentials.is_empty() {
        None
    } else {
        Some(selected)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Min(18),
            Constraint::Length(10),
            Constraint::Length(16),
            Constraint::Length(10),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    if app.credentials.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No credentials — press [A] to add one",
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(ratatui::layout::Alignment::Center),
            chunks[1],
        );
    } else {
        f.render_stateful_widget(table, chunks[1], &mut state);
    }

    f.render_widget(Paragraph::new(hotkey_lines), chunks[3]);

    if let Some(ref form) = app.cred_form {
        draw_form(
            f,
            form,
            Rect {
                height: chunks[2].y.saturating_sub(area.y),
                ..area
            },
        );
    }
    if let Some(ref status) = app.status_message {
        let color = match status.kind {
            StatusKind::Success => Color::Green,
            StatusKind::Info => Color::Blue,
            StatusKind::Warning => Color::Yellow,
            StatusKind::Error => Color::Red,
        };
        f.render_widget(
            Paragraph::new(status.text.as_str())
                .style(Style::default().fg(color))
                .wrap(Wrap { trim: false }),
            chunks[2],
        );
    }
}

fn draw_form(f: &mut Frame, form: &CredentialForm, area: Rect) {
    let popup = centered_rect(80, 90, area);
    f.render_widget(Clear, popup);

    let title = if form.editing_id.is_some() {
        " Edit credential "
    } else {
        " New credential "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(if form.is_key { 2 } else { 0 }),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .margin(1)
        .split(inner);

    // Type toggle
    let (pw_style, key_style) = if form.is_key {
        (
            Style::default().fg(Color::DarkGray),
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::DarkGray),
        )
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
    render_field(
        f,
        "Name:    ",
        &form.name,
        form.focused == 1,
        form.cursor,
        chunks[1],
    );
    render_field(
        f,
        "Username:",
        &form.username,
        form.focused == 2,
        form.cursor,
        chunks[2],
    );
    if form.is_key {
        render_field(
            f,
            "Key path:",
            &form.key_path,
            form.focused == 3,
            form.cursor,
            chunks[3],
        );
    }
    let secret_field = if form.is_key { 4 } else { 3 };
    let masked = "•".repeat(form.password.chars().count());
    render_field(
        f,
        if form.is_key {
            "Passphrase:"
        } else {
            "Password:"
        },
        &masked,
        form.focused == secret_field,
        form.cursor,
        chunks[secret_field],
    );
    let guidance = if form.editing_id.is_some() {
        "Blank keeps secret only for same type.\nNew type: password required; passphrase optional."
    } else if form.is_key {
        "Passphrase optional; blank for an unencrypted key."
    } else {
        "Password required."
    };
    f.render_widget(
        Paragraph::new(guidance)
            .style(Style::default().fg(Color::DarkGray))
            .wrap(Wrap { trim: false }),
        chunks[5],
    );
    f.render_widget(
        Paragraph::new(crate::tui::wrap_hotkey_lines(
            &[
                ("Tab", "next"),
                ("Space", "toggle type"),
                ("Enter", "save"),
                ("Esc", "cancel"),
            ],
            chunks[6].width,
        )),
        chunks[6],
    );
}

fn render_field(f: &mut Frame, label: &str, value: &str, focused: bool, cursor: usize, area: Rect) {
    let border_style = if focused {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let display = if focused {
        Line::from(crate::tui::input::spans(value, cursor, Style::default()))
    } else {
        Line::from(value.to_string())
    };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(12), Constraint::Min(0)])
        .split(area);
    f.render_widget(
        Paragraph::new(label).style(Style::default().fg(Color::DarkGray)),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(display).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(border_style),
        ),
        chunks[1],
    );
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
        KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if creds_len > 0 {
                app.cred_selected = (app.cred_selected + 1).min(creds_len - 1);
            }
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
                cursor: crate::tui::input::end_of(&c.name),
            });
        }
        KeyCode::Char('d') | KeyCode::Char('D') if creds_len > 0 => {
            let idx = app.cred_selected.min(creds_len - 1);
            let name = app.credentials[idx].name.clone();
            if app.skip_delete_confirm {
                let id = app.credentials[idx].id.clone();
                crate::credentials::delete_credential(&id)?;
                app.clear_credential_references(&id)?;
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
        KeyCode::Char('*') | KeyCode::Char('s') | KeyCode::Char('S') if creds_len > 0 => {
            let id = app.credentials[app.cred_selected.min(creds_len - 1)]
                .id
                .clone();
            let mut config = app.config.clone();
            config.default_credential_id = Some(id);
            match crate::config::save_config(&config) {
                Ok(()) => {
                    app.config = config;
                    app.status_message = Some(StatusMessage::success("Default credential saved."));
                }
                Err(e) => {
                    app.status_message = Some(StatusMessage::error(format!("Error: {e:#}")));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_form_key(app: &mut App, key: KeyEvent, mut form: CredentialForm) -> Result<()> {
    if form.focused > 0 && !matches!(key.code, KeyCode::Esc | KeyCode::Tab | KeyCode::Enter) {
        let mut cursor = form.cursor;
        let mut buf = std::mem::take(form_field(&mut form));
        let consumed = crate::tui::input::handle(&mut buf, &mut cursor, key);
        *form_field(&mut form) = buf;
        form.cursor = cursor;
        if consumed {
            app.cred_form = Some(form);
            return Ok(());
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.cred_form = None;
        }
        KeyCode::Tab => {
            form.focused = (form.focused + 1) % if form.is_key { 5 } else { 4 };
            form.cursor = crate::tui::input::end_of(form_field(&mut form));
            app.cred_form = Some(form);
        }
        KeyCode::Char(' ') if form.focused == 0 => {
            form.is_key = !form.is_key;
            form.password.clear();
            app.cred_form = Some(form);
        }
        KeyCode::Enter => {
            if let Err(e) = save_form(app, &form) {
                app.status_message =
                    Some(crate::types::StatusMessage::error(format!("Error: {e:#}")));
            } else {
                app.cred_form = None;
                app.status_message = Some(StatusMessage::success("Credential saved."));
            }
        }
        _ => {}
    }
    Ok(())
}

fn form_field(form: &mut CredentialForm) -> &mut String {
    match form.focused {
        1 => &mut form.name,
        2 => &mut form.username,
        3 if form.is_key => &mut form.key_path,
        _ => &mut form.password,
    }
}

fn save_form(app: &mut App, form: &CredentialForm) -> Result<()> {
    if form.name.trim().is_empty() {
        anyhow::bail!("Name is required.");
    }
    if form.username.trim().is_empty() {
        anyhow::bail!("Username is required.");
    }
    if form.is_key && form.key_path.trim().is_empty() {
        anyhow::bail!("Private key path is required for Key credentials.");
    }
    if !form.is_key && form.editing_id.is_none() && form.password.trim().is_empty() {
        anyhow::bail!("Password is required for Password credentials.");
    }

    let kind = if form.is_key {
        CredentialKind::Key
    } else {
        CredentialKind::Password
    };
    let key_path = if form.is_key && !form.key_path.trim().is_empty() {
        Some(form.key_path.trim().to_string())
    } else {
        None
    };
    let password = if !form.password.is_empty() {
        Some(form.password.as_str())
    } else {
        None
    };

    if let Some(ref id) = form.editing_id {
        crate::credentials::update_credential(
            id,
            form.name.trim(),
            form.username.trim(),
            kind,
            key_path,
            password,
        )?;
    } else {
        crate::credentials::add_credential(
            form.name.trim(),
            form.username.trim(),
            kind,
            key_path,
            password,
        )?;
    }
    app.reload_credentials()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};

    fn app() -> App {
        App::new(vec![], vec![], crate::config::AppConfig::default(), vec![])
    }

    fn press(app: &mut App, code: KeyCode) {
        let form = app.cred_form.clone().unwrap();
        handle_form_key(app, KeyEvent::new(code, KeyModifiers::NONE), form).unwrap();
    }

    fn rendered(app: &App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn key_navigation_and_editing_use_password_for_passphrase() {
        let mut app = app();
        app.cred_form = Some(CredentialForm {
            is_key: true,
            key_path: "/tmp/key".into(),
            password: "密".into(),
            focused: 3,
            ..Default::default()
        });
        press(&mut app, KeyCode::Tab);
        let form = app.cred_form.as_ref().unwrap();
        assert_eq!(form.focused, 4);
        assert_eq!(form.cursor, 1);
        press(&mut app, KeyCode::Char('!'));
        let form = app.cred_form.as_ref().unwrap();
        assert_eq!(form.password, "密!");
        assert_eq!(form.key_path, "/tmp/key");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.cred_form.as_ref().unwrap().focused, 0);
        press(&mut app, KeyCode::Char(' '));
        assert!(app.cred_form.as_ref().unwrap().password.is_empty());
        for expected in [1, 2, 3, 0] {
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.cred_form.as_ref().unwrap().focused, expected);
        }
    }

    #[test]
    fn secret_field_depends_on_kind_and_focus() {
        let mut form = CredentialForm::default();
        for (is_key, focused, expected) in [
            (false, 1, "name"),
            (false, 2, "user"),
            (false, 3, "secret"),
            (true, 3, "path"),
            (true, 4, "secret"),
        ] {
            form.name = "name".into();
            form.username = "user".into();
            form.password = "secret".into();
            form.key_path = "path".into();
            form.is_key = is_key;
            form.focused = focused;
            assert_eq!(form_field(&mut form), expected);
        }
    }

    #[test]
    fn renders_masked_secrets_and_blank_guidance() {
        let mut app = app();
        for is_key in [false, true] {
            app.cred_form = Some(CredentialForm {
                is_key,
                editing_id: Some("test".into()),
                password: "密secret".into(),
                ..Default::default()
            });
            let text = rendered(&app);
            assert!(text.contains(if is_key { "Passphrase:" } else { "Password:" }));
            assert!(text.contains("•••••••"));
            assert!(!text.contains("密secret"));
            assert!(text.contains("Blank keeps secret only for same type."));
            assert!(text.contains("password required; passphrase optional."));
        }
        app.cred_form.as_mut().unwrap().editing_id = None;
        assert!(rendered(&app).contains("Passphrase optional; blank for an unencrypted key."));
    }

    #[test]
    fn validation_error_is_visible_with_form_open() {
        let mut app = app();
        app.cred_form = Some(CredentialForm::default());
        press(&mut app, KeyCode::Enter);
        assert!(app.cred_form.is_some());
        assert_eq!(app.status_message.as_ref().unwrap().kind, StatusKind::Error);
        assert!(rendered(&app).contains("Error: Name is required."));
    }

    #[test]
    fn renders_default_save_success() {
        let mut app = app();
        app.status_message = Some(StatusMessage::success("Default credential saved."));
        assert!(rendered(&app).contains("Default credential saved."));
    }
}
