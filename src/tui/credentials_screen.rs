use crate::tui::{theme, App, Screen, Term};
use crate::types::{CredentialForm, CredentialKind};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = crate::tui::help::content_area(app, f.area());
    let hotkeys: &[(&str, &str)] = if app.credentials.is_empty() {
        &[("A", "add"), ("Esc", "back")]
    } else {
        &[
            ("A", "add"),
            ("E", "edit"),
            ("D", "delete"),
            ("*", "default"),
            ("Esc", "back"),
        ]
    };
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkeys, area.width);
    let hotkey_height = hotkey_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(hotkey_height),
        ])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "hss",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · credentials", Style::default().fg(theme::MUTED)),
        ])),
        chunks[0],
    );

    let header = Row::new(vec!["NAME", "TYPE", "USERNAME", "DEFAULT"])
        .style(Style::default().fg(theme::MUTED));

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
                Cell::from(kind_str).style(Style::default().fg(theme::MUTED)),
                Cell::from(c.username.clone()).style(Style::default().fg(theme::MUTED)),
                Cell::from(default_marker).style(Style::default().fg(theme::INFO)),
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
    .row_highlight_style(Style::default().bg(theme::SELECTION))
    .highlight_symbol("▶ ");

    if app.credentials.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No credentials — press [A] to add one",
                Style::default().fg(theme::MUTED),
            ))
            .alignment(ratatui::layout::Alignment::Center),
            chunks[1],
        );
    } else {
        f.render_stateful_widget(table, chunks[1], &mut state);
    }

    f.render_widget(Paragraph::new(hotkey_lines), chunks[2]);

    // Draw form overlay if active
    if let Some(ref form) = app.cred_form {
        let can_keep_password = existing_kind(app, form) == Some(&CredentialKind::Password);
        draw_form(f, form, can_keep_password, area);
    }
}

fn draw_form(f: &mut Frame, form: &CredentialForm, can_keep_password: bool, area: Rect) {
    let popup = theme::dialog_area(area, 20);
    f.render_widget(Clear, popup);

    let title = if form.editing_id.is_some() {
        "edit credential"
    } else {
        "new credential"
    };
    let block = theme::dialog_block(title);
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let hotkeys = if form.confirm_kind_change {
        vec![("Enter", "confirm save"), ("Esc", "continue editing")]
    } else {
        let mut keys = vec![("Tab", "next"), ("Shift+Tab", "prev")];
        if form.focused == 0 {
            keys.push(("Space", "change type"));
        }
        keys.extend([("Enter", "save"), ("Esc", "cancel")]);
        keys
    };
    let content_width = inner.width.saturating_sub(2);
    let hotkey_lines = crate::tui::wrap_hotkey_lines(&hotkeys, content_width);
    let hint = if form.confirm_kind_change && form.is_key {
        "Save as Key? This removes the stored password after saving. Esc keeps editing."
    } else if form.confirm_kind_change {
        "Save as Password? This replaces the key path with the new stored password."
    } else if !form.is_key && can_keep_password {
        "Leave Password blank to keep the existing stored password."
    } else if !form.is_key {
        "Password is required; it is stored in the system keychain."
    } else {
        "Private key path is required. The key stays on disk."
    };
    let hint_lines = wrap_message(hint, content_width);
    let error_lines = form
        .error_message
        .as_deref()
        .map(|error| wrap_message(error, content_width))
        .unwrap_or_default();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(hint_lines.len() as u16),
            Constraint::Length(error_lines.len().min(3) as u16),
            Constraint::Length(hotkey_lines.len() as u16),
        ])
        .margin(1)
        .split(inner);
    // Keep the focused row visible if the terminal is shorter than the form.
    let body = chunks[0];
    let offset = ((form.focused.min(3) * 2 + 2) as u16)
        .saturating_sub(body.height)
        .min(8u16.saturating_sub(body.height));
    let field_area = |field: usize| -> Option<Rect> {
        let row = field as u16 * 2;
        if row < offset || row >= offset + body.height {
            return None;
        }
        Some(Rect::new(
            body.x,
            body.y + row - offset,
            body.width,
            2.min(body.height - (row - offset)),
        ))
    };

    // Type toggle
    let (pw_style, key_style) = if form.is_key {
        (
            Style::default().fg(theme::MUTED),
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(theme::MUTED),
        )
    };
    if let Some(area) = field_area(0) {
        let label_width = theme::FORM_LABEL_WIDTH.min(area.width.saturating_sub(1));
        let focused = form.focused == 0 && !form.confirm_kind_change;
        f.render_widget(
            Paragraph::new(if focused { "▶ Type:" } else { "  Type:" })
                .style(theme::field_label(focused)),
            Rect::new(area.x, area.y, label_width, 1),
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("[Password]", pw_style),
                Span::raw("  "),
                Span::styled("[Key]", key_style),
            ])),
            Rect::new(
                area.x + label_width,
                area.y,
                area.width.saturating_sub(label_width),
                1,
            ),
        );
    }
    for (field, label, value) in [
        (1, "Name:", form.name.as_str()),
        (2, "Username:", form.username.as_str()),
    ] {
        if let Some(area) = field_area(field) {
            render_field(
                f,
                label,
                value,
                form.focused == field && !form.confirm_kind_change,
                form.cursor,
                area,
            );
        }
    }
    if let Some(area) = field_area(3) {
        let masked = "•".repeat(form.password.chars().count());
        let (label, value) = if form.is_key {
            ("Key path:", form.key_path.as_str())
        } else {
            ("Password:", masked.as_str())
        };
        render_field(
            f,
            label,
            value,
            form.focused == 3 && !form.confirm_kind_change,
            form.cursor,
            area,
        );
    }
    f.render_widget(
        Paragraph::new(hint_lines).style(Style::default().fg(theme::MUTED)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(error_lines).style(Style::default().fg(theme::ERROR)),
        chunks[2],
    );
    f.render_widget(Paragraph::new(hotkey_lines), chunks[3]);
}

fn render_field(f: &mut Frame, label: &str, value: &str, focused: bool, cursor: usize, area: Rect) {
    let label_width = theme::FORM_LABEL_WIDTH.min(area.width.saturating_sub(1));
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(label_width), Constraint::Min(0)])
        .split(area);
    let display = if focused {
        Line::from(crate::tui::input::spans_with_width(
            value,
            cursor,
            theme::field_value(),
            chunks[1].width as usize,
        ))
    } else {
        Line::styled(value.to_string(), theme::field_value())
    };
    f.render_widget(
        Paragraph::new(label).style(theme::field_label(focused)),
        chunks[0],
    );
    f.render_widget(Paragraph::new(display), chunks[1]);
}

pub(super) fn wrap_message(message: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in message.split_whitespace() {
        if !line.is_empty()
            && crate::tui::input::str_width(&line) + crate::tui::input::str_width(word) > width
        {
            lines.push(Line::from(std::mem::take(&mut line)));
        }
        for ch in word.chars() {
            if crate::tui::input::str_width(&line) + crate::tui::input::str_width(&ch.to_string())
                > width
            {
                lines.push(Line::from(std::mem::take(&mut line)));
            }
            line.push(ch);
        }
        line.push(' ');
    }
    if !line.is_empty() {
        lines.push(Line::from(line.trim_end().to_string()));
    }
    lines
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
                ..CredentialForm::default()
            });
        }
        KeyCode::Char('d') | KeyCode::Char('D') if creds_len > 0 => {
            let idx = app.cred_selected.min(creds_len - 1);
            let name = app.credentials[idx].name.clone();
            if app.skip_credential_delete_confirm {
                if let Err(error) = crate::tui::delete_popup::delete_credential(app, idx) {
                    app.status_message = Some(crate::types::StatusMessage::error(format!(
                        "Delete failed: {error}"
                    )));
                }
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
            let id = app.credentials[app.cred_selected.min(creds_len - 1)]
                .id
                .clone();
            let mut config = app.config.clone();
            config.default_credential_id = Some(id);
            crate::config::save_config(&config)?;
            app.config = config;
        }
        _ => {}
    }
    Ok(())
}

fn handle_form_key(app: &mut App, key: KeyEvent, mut form: CredentialForm) -> Result<()> {
    if form.confirm_kind_change {
        match key.code {
            KeyCode::Esc => form.confirm_kind_change = false,
            KeyCode::Enter if try_save_form(app, &mut form) => {
                app.cred_form = None;
                return Ok(());
            }
            _ => {}
        }
        app.cred_form = Some(form);
        return Ok(());
    }
    // Text fields (1..=3) get full line editing; field 0 is the type toggle.
    if form.focused > 0
        && !matches!(
            key.code,
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter
        )
    {
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
        KeyCode::Tab | KeyCode::BackTab => {
            form.focused = if key.code == KeyCode::Tab {
                (form.focused + 1) % 4
            } else {
                (form.focused + 3) % 4
            };
            form.cursor = if form.focused == 0 {
                0
            } else {
                crate::tui::input::end_of(form_field(&mut form))
            };
            app.cred_form = Some(form);
        }
        KeyCode::Char(' ') if form.focused == 0 => {
            form.is_key = !form.is_key;
            app.cred_form = Some(form);
        }
        KeyCode::Enter => {
            if let Err((field, message)) = validate_form(&form, existing_kind(app, &form)) {
                focus_error(&mut form, field, message);
                app.cred_form = Some(form);
            } else if kind_changed(app, &form) {
                form.confirm_kind_change = true;
                app.cred_form = Some(form);
            } else if try_save_form(app, &mut form) {
                app.cred_form = None;
            } else {
                app.cred_form = Some(form);
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
        _ if form.is_key => &mut form.key_path,
        _ => &mut form.password,
    }
}

fn existing_kind<'a>(app: &'a App, form: &CredentialForm) -> Option<&'a CredentialKind> {
    form.editing_id
        .as_ref()
        .and_then(|id| {
            app.credentials
                .iter()
                .find(|credential| &credential.id == id)
        })
        .map(|credential| &credential.kind)
}

fn kind_changed(app: &App, form: &CredentialForm) -> bool {
    existing_kind(app, form).is_some_and(|kind| (kind == &CredentialKind::Key) != form.is_key)
}

fn validate_form(
    form: &CredentialForm,
    existing: Option<&CredentialKind>,
) -> std::result::Result<(), (usize, String)> {
    let invalid = |field, message: &str| Err((field, message.to_string()));
    if form.name.trim().is_empty() {
        return invalid(1, "Name is required.");
    }
    if form.username.trim().is_empty() {
        return invalid(2, "Username is required.");
    }
    if form.editing_id.is_some() && existing.is_none() {
        return invalid(
            1,
            "Credential no longer exists. Cancel and reopen the form.",
        );
    }
    if form.is_key && form.key_path.trim().is_empty() {
        return invalid(3, "Private key path is required for Key credentials.");
    }
    // Blank keeps a password only when the original credential already had one.
    if !form.is_key && form.password.is_empty() && existing != Some(&CredentialKind::Password) {
        return invalid(
            3,
            "Password is required for new or converted Password credentials.",
        );
    }
    Ok(())
}

fn focus_error(form: &mut CredentialForm, field: usize, message: String) {
    form.focused = field;
    form.cursor = crate::tui::input::end_of(form_field(form));
    form.error_message = Some(message);
    form.confirm_kind_change = false;
}

fn try_save_form(app: &mut App, form: &mut CredentialForm) -> bool {
    if let Err((field, message)) = validate_form(form, existing_kind(app, form)) {
        focus_error(form, field, message);
        return false;
    }
    form.confirm_kind_change = false;
    match save_form(app, form) {
        Ok(()) => {
            app.status_message = Some(crate::types::StatusMessage::success("Credential saved."));
            true
        }
        Err(error) => {
            form.error_message = Some(format!("Could not save credential: {error}"));
            false
        }
    }
}

fn save_form(app: &mut App, form: &CredentialForm) -> Result<()> {
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
    let password = if !form.is_key && !form.password.is_empty() {
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
    use crate::config::AppConfig;
    use crate::types::Credential;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};

    fn valid_form() -> CredentialForm {
        CredentialForm {
            name: "production".into(),
            username: "alice".into(),
            password: "secret".into(),
            focused: 1,
            ..CredentialForm::default()
        }
    }

    fn app() -> App {
        App::new(vec![], vec![], AppConfig::default(), vec![])
    }

    fn key(app: &mut App, code: KeyCode) {
        let form = app.cred_form.clone().unwrap();
        handle_form_key(app, KeyEvent::new(code, KeyModifiers::NONE), form).unwrap();
    }

    #[test]
    fn validation_identifies_each_required_field() {
        let mut form = CredentialForm::default();
        assert_eq!(validate_form(&form, None).unwrap_err().0, 1);
        form.name = "name".into();
        assert_eq!(validate_form(&form, None).unwrap_err().0, 2);
        form.username = "user".into();
        assert_eq!(validate_form(&form, None).unwrap_err().0, 3);
        form.is_key = true;
        assert_eq!(validate_form(&form, None).unwrap_err().0, 3);
        form.key_path = "~/.ssh/id_ed25519".into();
        assert!(validate_form(&form, None).is_ok());
    }

    #[test]
    fn blank_password_keeps_only_an_existing_password_and_spaces_are_preserved() {
        let mut form = valid_form();
        form.password.clear();
        form.editing_id = Some("existing".into());
        assert!(validate_form(&form, Some(&CredentialKind::Password)).is_ok());
        assert_eq!(
            validate_form(&form, Some(&CredentialKind::Key))
                .unwrap_err()
                .0,
            3
        );
        assert!(validate_form(&form, None).is_err());
        form.editing_id = None;
        form.password = "   ".into();
        assert!(validate_form(&form, None).is_ok());
    }

    #[test]
    fn inline_error_survives_editing_and_navigation_and_focuses_invalid_field() {
        let mut app = app();
        app.cred_form = Some(CredentialForm::default());
        key(&mut app, KeyCode::Enter);
        let form = app.cred_form.as_ref().unwrap();
        assert_eq!(form.focused, 1);
        assert_eq!(form.error_message.as_deref(), Some("Name is required."));
        assert!(app.status_message.is_none());
        key(&mut app, KeyCode::Char('x'));
        key(&mut app, KeyCode::Tab);
        assert_eq!(
            app.cred_form.as_ref().unwrap().error_message.as_deref(),
            Some("Name is required.")
        );
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.cred_form.as_ref().unwrap().focused, 2);
        assert_eq!(
            app.cred_form.as_ref().unwrap().error_message.as_deref(),
            Some("Username is required.")
        );
    }

    #[test]
    fn backtab_wraps_and_space_toggles_only_when_type_is_focused() {
        let mut app = app();
        app.cred_form = Some(valid_form());
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.cred_form.as_ref().unwrap().focused, 0);
        assert_eq!(app.cred_form.as_ref().unwrap().cursor, 0);
        key(&mut app, KeyCode::Char(' '));
        assert!(app.cred_form.as_ref().unwrap().is_key);
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.cred_form.as_ref().unwrap().focused, 3);
        key(&mut app, KeyCode::Char(' '));
        assert_eq!(app.cred_form.as_ref().unwrap().key_path, " ");
        assert!(app.cred_form.as_ref().unwrap().is_key);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.cred_form.as_ref().unwrap().focused, 0);
    }

    #[test]
    fn kind_change_requires_confirmation_before_touching_storage() {
        let mut app = app();
        app.credentials.push(Credential {
            id: "existing".into(),
            name: "name".into(),
            username: "user".into(),
            kind: CredentialKind::Password,
            key_path: None,
        });
        app.cred_form = Some(CredentialForm {
            editing_id: Some("existing".into()),
            is_key: true,
            key_path: "~/.ssh/id_ed25519".into(),
            ..valid_form()
        });
        key(&mut app, KeyCode::Enter);
        assert!(app.cred_form.as_ref().unwrap().confirm_kind_change);
        assert_eq!(app.credentials[0].kind, CredentialKind::Password);
        key(&mut app, KeyCode::Char('x'));
        assert!(app.cred_form.as_ref().unwrap().confirm_kind_change);
        key(&mut app, KeyCode::Esc);
        assert!(!app.cred_form.as_ref().unwrap().confirm_kind_change);
        assert!(app.cred_form.as_ref().unwrap().is_key);
    }

    fn rendered(form: &CredentialForm, keep_password: bool, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw_form(frame, form, keep_password, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn compact_forms_show_fields_password_keep_hint_errors_and_hotkeys() {
        let form = CredentialForm {
            editing_id: Some("existing".into()),
            focused: 3,
            error_message: Some("Could not save credential: keychain is locked.".into()),
            ..valid_form()
        };
        for (width, height) in [(80, 24), (60, 20)] {
            let text = rendered(&form, true, width, height);
            for expected in [
                "Name:",
                "Username:",
                "Password:",
                "Leave Password blank",
                "existing stored password",
                "keychain is locked",
                "[Shift+Tab] prev",
                "[Enter] save",
                "[Esc] cancel",
            ] {
                assert!(
                    text.contains(expected),
                    "missing {expected:?} at {width}x{height}"
                );
            }
            assert!(!text.contains("secret"));
        }
    }

    #[test]
    fn type_focus_is_visible_and_conversion_copy_does_not_promise_password_keep() {
        let form = CredentialForm {
            focused: 0,
            ..valid_form()
        };
        let text = rendered(&form, false, 60, 20);
        assert!(text.contains("▶ Type:"));
        assert!(text.contains("[Space] change type"));
        assert!(text.contains("Password is required"));
        assert!(!text.contains("Leave Password blank"));
        let form = CredentialForm {
            confirm_kind_change: true,
            is_key: true,
            ..form
        };
        let text = rendered(&form, true, 60, 20);
        assert!(text.contains("removes the stored password"));
        assert!(text.contains("[Enter] confirm save"));
    }

    #[test]
    fn short_form_scrolls_to_focused_secret_without_panicking() {
        let form = CredentialForm {
            focused: 3,
            is_key: true,
            ..valid_form()
        };
        assert!(rendered(&form, false, 60, 12).contains("Key path:"));
        for (width, height) in [(0, 0), (1, 1), (20, 8)] {
            rendered(&form, false, width, height);
        }
    }
}
