use crate::tui::{theme, App, Term};
use crate::types::{DeleteKind, DeletePopup, Host, StatusMessage};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let Some(popup) = &app.delete_popup else {
        return;
    };

    let blocked = match popup.kind {
        DeleteKind::Host => host_delete_block_reason(app, popup.idx),
        DeleteKind::Credential => credential_delete_block_reason(app, popup.idx),
    };
    let available = crate::tui::help::content_area(app, f.area());
    let width = theme::dialog_area(available, 1).width.saturating_sub(4);
    let message_rows = crate::tui::credentials_screen::wrap_message(
        &format!("Delete credential '{}' ?", popup.name),
        width,
    )
    .len();
    let dependency_rows = blocked
        .as_deref()
        .map(|reason| 1 + crate::tui::credentials_screen::wrap_message(reason, width).len())
        .unwrap_or(0);
    let hotkeys = if blocked.is_some() {
        vec![("N/Enter/Esc", "cancel — deletion blocked")]
    } else {
        vec![
            ("Y", "delete"),
            ("N/Enter/Esc", "cancel"),
            ("Space", "toggle"),
        ]
    };
    let hotkey_lines = crate::tui::wrap_hotkey_lines(&hotkeys, width);
    let height =
        4 + message_rows + dependency_rows + usize::from(blocked.is_none()) + hotkey_lines.len();
    let area = theme::dialog_area(available, height.min(u16::MAX as usize) as u16);
    f.render_widget(Clear, area);

    let block =
        theme::dialog_block("confirm delete").border_style(Style::default().fg(theme::ERROR));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1), // message and dependent hosts
            Constraint::Length(if blocked.is_some() { 0 } else { 1 }), // checkbox
            Constraint::Length(hotkey_lines.len() as u16), // hotkeys
        ])
        .margin(1)
        .split(inner);

    let kind_str = match popup.kind {
        DeleteKind::Host => "host",
        DeleteKind::Credential => "credential",
    };

    let mut message = vec![Line::from(vec![
        Span::styled("Delete ", Style::default().fg(theme::TEXT)),
        Span::styled(kind_str, Style::default().fg(theme::MUTED)),
        Span::styled(" '", Style::default().fg(theme::TEXT)),
        Span::styled(
            &popup.name,
            Style::default()
                .fg(theme::WARNING)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("'?", Style::default().fg(theme::TEXT)),
    ])];
    if let Some(reason) = &blocked {
        message.push(Line::default());
        message.push(Line::styled(reason, Style::default().fg(theme::WARNING)));
    }
    f.render_widget(
        Paragraph::new(message).wrap(Wrap { trim: false }),
        chunks[0],
    );

    let check = if popup.dont_ask { "[x]" } else { "[ ]" };
    if blocked.is_none() {
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(check, Style::default().fg(theme::FOCUS)),
                Span::styled(
                    format!(" skip {kind_str} confirmations this session"),
                    Style::default().fg(theme::MUTED),
                ),
            ])),
            chunks[1],
        );
    }

    f.render_widget(Paragraph::new(hotkey_lines), chunks[2]);
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    handle_delete_key(app, key)
}

fn handle_delete_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            if let Some(popup) = app.delete_popup.clone() {
                match do_delete(app, &popup) {
                    Ok(true) => {
                        if popup.dont_ask {
                            skip_confirmations(app, &popup.kind);
                        }
                        app.delete_popup = None;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        app.status_message =
                            Some(StatusMessage::error(format!("Delete failed: {error}")));
                    }
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter | KeyCode::Esc => {
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

fn skip_confirmations(app: &mut App, kind: &DeleteKind) {
    match kind {
        DeleteKind::Host => app.skip_host_delete_confirm = true,
        DeleteKind::Credential => app.skip_credential_delete_confirm = true,
    }
}

/// Includes downstream hosts whose jump chain uses this bastion, not just direct references.
fn dependent_hosts<'a>(hosts: &'a [Host], id: &str) -> Vec<&'a Host> {
    hosts
        .iter()
        .filter(|host| {
            if host.id == id {
                return false;
            }
            let mut jump = host.jump_host_id.as_deref();
            // The bound also terminates malformed, cyclic jump chains.
            for _ in 0..hosts.len() {
                let Some(jump_id) = jump else { break };
                if jump_id == id {
                    return true;
                }
                jump = hosts
                    .iter()
                    .find(|candidate| candidate.id == jump_id)
                    .and_then(|candidate| candidate.jump_host_id.as_deref());
            }
            false
        })
        .collect()
}

pub fn host_delete_block_reason(app: &App, idx: usize) -> Option<String> {
    let host = app.hosts.get(idx)?;
    let dependents = dependent_hosts(&app.hosts, &host.id);
    if dependents.is_empty() {
        return None;
    }
    Some(format!(
        "Deletion blocked: {} dependent host(s) would lose their SSH jump route. Reassign jump hosts before deleting. Affected: {}.",
        dependents.len(),
        dependents.iter().map(|host| host.name.as_str()).collect::<Vec<_>>().join(", ")
    ))
}

/// Shared with the main screen's skip-confirmation path: skipping never bypasses dependencies.
pub fn delete_host(app: &mut App, idx: usize) -> Result<bool> {
    if idx >= app.hosts.len() {
        return Ok(false);
    }
    if let Some(reason) = host_delete_block_reason(app, idx) {
        app.status_message = Some(StatusMessage::error(reason));
        return Ok(false);
    }
    let removed = app.hosts.remove(idx);
    if let Err(error) = app.save_hosts() {
        app.hosts.insert(idx, removed);
        return Err(error);
    }
    app.selected_row = app.selected_row.min(app.hosts.len().saturating_sub(1));
    app.status_message = Some(StatusMessage::info(format!(
        "Host '{}' deleted.",
        removed.name
    )));
    Ok(true)
}

pub fn credential_delete_block_reason(app: &App, idx: usize) -> Option<String> {
    let credential = app.credentials.get(idx)?;
    let dependents: Vec<_> = app
        .hosts
        .iter()
        .filter(|host| host.credential_id.as_deref() == Some(credential.id.as_str()))
        .map(|host| host.name.as_str())
        .collect();
    if dependents.is_empty() {
        return None;
    }
    Some(format!(
        "Deletion blocked: {} host(s) explicitly use this credential. Edit each affected host and reassign Credentials or reset to Automatic before deleting. Affected: {}.",
        dependents.len(), dependents.join(", ")
    ))
}

/// Both confirmation paths share the dependency guard and partial-success handling.
pub fn delete_credential(app: &mut App, idx: usize) -> Result<bool> {
    delete_credential_with_io(
        app,
        idx,
        crate::credentials::delete_credential,
        crate::config::save_config,
        crate::config::load_credentials,
    )
}

fn delete_credential_with_io(
    app: &mut App,
    idx: usize,
    delete: impl FnOnce(&str) -> Result<()>,
    save_config: impl FnOnce(&crate::config::AppConfig) -> Result<()>,
    reload: impl FnOnce() -> Result<Vec<crate::types::Credential>>,
) -> Result<bool> {
    let Some(credential) = app.credentials.get(idx).cloned() else {
        return Ok(false);
    };
    if let Some(reason) = credential_delete_block_reason(app, idx) {
        app.status_message = Some(StatusMessage::error(reason));
        return Ok(false);
    }
    delete(&credential.id)?;
    // Deletion has committed: never leave the deleted row visible or report a total failure.
    app.credentials.retain(|c| c.id != credential.id);
    let mut warnings = Vec::new();
    if app.config.default_credential_id.as_deref() == Some(credential.id.as_str()) {
        app.config.default_credential_id = None;
        if let Err(error) = save_config(&app.config) {
            warnings.push(format!(
                "could not save default cleanup: {error}. Choose a new default in Credentials or save Settings to repair the saved default"
            ));
        }
    }
    match reload() {
        Ok(credentials) => app.credentials = credentials,
        Err(error) => warnings.push(format!(
            "could not reload credentials: {error}. Reopen Credentials to refresh"
        )),
    }
    app.cred_selected = app
        .cred_selected
        .min(app.credentials.len().saturating_sub(1));
    let message = format!("Credential '{}' deleted.", credential.name);
    app.status_message = Some(if warnings.is_empty() {
        StatusMessage::info(message)
    } else {
        StatusMessage::warning(format!("{message} However, {}.", warnings.join("; ")))
    });
    Ok(true)
}

fn do_delete(app: &mut App, popup: &DeletePopup) -> Result<bool> {
    match popup.kind {
        DeleteKind::Host => delete_host(app, popup.idx),
        DeleteKind::Credential => delete_credential(app, popup.idx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};

    fn host(name: &str, jump: Option<&str>) -> Host {
        Host {
            id: name.into(),
            name: name.into(),
            ip: "example.com".into(),
            group: String::new(),
            port: 22,
            user: None,
            tags: vec![],
            description: None,
            jump_host_id: jump.map(str::to_string),
            credential_id: None,
        }
    }

    fn app() -> App {
        let mut app = App::new(
            vec![
                host("bastion", None),
                host("database", Some("bastion")),
                host("worker", Some("database")),
            ],
            vec![],
            crate::config::AppConfig::default(),
            vec![],
        );
        app.delete_popup = Some(DeletePopup {
            kind: DeleteKind::Host,
            name: "bastion".into(),
            idx: 0,
            dont_ask: true,
        });
        app
    }

    #[test]
    fn enter_cancels_without_deleting_or_disabling_confirmations() {
        let mut app = app();
        handle_delete_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).unwrap();
        assert!(app.delete_popup.is_none());
        assert_eq!(app.hosts.len(), 3);
        assert!(!app.skip_host_delete_confirm);
        assert!(!app.skip_credential_delete_confirm);
    }

    #[test]
    fn bastion_deletion_is_blocked_and_never_unlinks_dependents() {
        let mut app = app();
        let reason = host_delete_block_reason(&app, 0).unwrap();
        assert!(reason.contains("database"));
        assert!(reason.contains("worker"));
        handle_delete_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(app.hosts.len(), 3);
        assert_eq!(app.hosts[1].jump_host_id.as_deref(), Some("bastion"));
        assert!(app.delete_popup.is_some());
        assert!(!app.skip_host_delete_confirm);
        assert!(!app.skip_credential_delete_confirm);
        assert!(app
            .status_message
            .as_ref()
            .unwrap()
            .text
            .contains("SSH jump route"));
        // The same guard is used when confirmation has already been skipped.
        app.skip_host_delete_confirm = true;
        assert!(!delete_host(&mut app, 0).unwrap());
        app.hosts[1].jump_host_id = None;
        assert!(host_delete_block_reason(&app, 0).is_none());
    }

    #[test]
    fn skipped_confirmations_are_scoped_to_the_deleted_kind() {
        let mut app = app();
        skip_confirmations(&mut app, &DeleteKind::Host);
        assert!(app.skip_host_delete_confirm);
        assert!(!app.skip_credential_delete_confirm);
        app.skip_host_delete_confirm = false;
        skip_confirmations(&mut app, &DeleteKind::Credential);
        assert!(!app.skip_host_delete_confirm);
        assert!(app.skip_credential_delete_confirm);
    }

    #[test]
    fn dependent_detection_terminates_on_unrelated_cycles() {
        let hosts = vec![
            host("bastion", None),
            host("a", Some("b")),
            host("b", Some("a")),
        ];
        assert!(dependent_hosts(&hosts, "bastion").is_empty());
    }

    #[test]
    fn dependency_impact_and_cancel_default_render_at_small_sizes() {
        for (width, height) in [(60, 20), (80, 24)] {
            let mut app = app();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("database"));
            assert!(text.contains("worker"));
            assert!(text.contains("Reassign jump hosts"));
            assert!(text.contains("[N/Enter/Esc]"));
            assert!(!text.contains("[Y]"));
            app.hosts.truncate(1);
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("skip host confirmations"));
            assert!(text.contains("[Y]"));
            assert!(text.contains("[N/Enter/Esc]"));
        }
    }
}
