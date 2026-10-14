use crate::tui::{theme, App, Screen, Term};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let Screen::CredentialPicker {
        host_idx,
        after_failure,
    } = &app.screen
    else {
        return;
    };
    let host = &app.hosts[*host_idx];

    let area = crate::tui::adaptive_centered_rect(
        60,
        50,
        60,
        14,
        crate::tui::help::content_area(app, f.area()),
    );
    f.render_widget(Clear, area);

    let title = format!(" Connect as · {} ", host.name);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::FOCUS));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(u16::from(*after_failure)),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    if *after_failure {
        f.render_widget(
            Paragraph::new("SSH failed; credentials may not be the cause.")
                .style(Style::default().fg(theme::WARNING)),
            chunks[0],
        );
    }
    let is_empty = app.credentials.is_empty();

    if is_empty {
        let empty_msg = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No credentials configured yet.",
                Style::default().fg(theme::WARNING),
            )),
            Line::from(Span::styled(
                "Press [A] to add credentials, or [Esc] to return.",
                Style::default().fg(theme::MUTED),
            )),
        ])
        .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(empty_msg, chunks[1]);
    } else {
        // Credential list
        let rows: Vec<Row> = app
            .credentials
            .iter()
            .map(|c| {
                let kind = match c.kind {
                    crate::types::CredentialKind::Key => "key",
                    crate::types::CredentialKind::Password => "password",
                };
                let default_marker = if app.config.default_credential_id.as_deref() == Some(&c.id) {
                    "★"
                } else {
                    ""
                };
                Row::new(vec![
                    Cell::from(c.name.clone()).style(Style::default().fg(theme::TEXT)),
                    Cell::from(c.username.clone()).style(Style::default().fg(theme::TEXT)),
                    Cell::from(kind).style(Style::default().fg(theme::MUTED)),
                    Cell::from(default_marker).style(Style::default().fg(theme::INFO)),
                ])
            })
            .collect();

        let selected = app
            .popup_selected
            .min(app.credentials.len().saturating_sub(1));
        let mut state = TableState::default().with_selected(Some(selected));

        let table = Table::new(
            rows,
            [
                Constraint::Min(12),
                Constraint::Min(12),
                Constraint::Length(8),
                Constraint::Length(1),
            ],
        )
        .header(
            Row::new(["NAME", "USERNAME", "TYPE", "★"]).style(
                Style::default()
                    .fg(theme::MUTED)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .row_highlight_style(Style::default().bg(theme::SELECTION))
        .highlight_symbol("▶ ");

        f.render_stateful_widget(table, chunks[1], &mut state);
    }

    let pairs: &[(&str, &str)] = if is_empty {
        &[("A", "add credential"), ("Esc", "cancel")]
    } else {
        &[("Enter", "connect"), ("A", "add"), ("Esc", "cancel")]
    };
    let spans = pairs
        .iter()
        .flat_map(|(key, label)| {
            [
                Span::styled(format!("[{key}]"), Style::default().fg(theme::FOCUS)),
                Span::styled(format!(" {label}  "), Style::default().fg(theme::MUTED)),
            ]
        })
        .collect::<Vec<_>>();
    f.render_widget(Paragraph::new(Line::from(spans)), chunks[2]);
}

pub fn handle_key(
    terminal: &mut Term,
    app: &mut App,
    key: KeyEvent,
    host_idx: usize,
    _after_failure: bool,
) -> Result<()> {
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
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.cred_form = Some(crate::types::CredentialForm::default());
            app.screen = Screen::Credentials;
        }
        KeyCode::Enter if creds_len > 0 => {
            let cred = app.credentials[app.popup_selected.min(creds_len - 1)].clone();
            let host_id = app.hosts[host_idx].id.clone();
            app.screen = Screen::Main;
            crate::tui::do_connect(terminal, app, &host_id, &cred)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Credential, CredentialKind, Host};

    fn test_app(after_failure: bool) -> App {
        let mut app = App::new(
            vec![Host {
                id: "web".into(),
                name: "web-01".into(),
                ip: "10.0.0.2".into(),
                group: "prod".into(),
                port: 22,
                user: None,
                tags: vec![],
                description: None,
                jump_host_id: None,
            }],
            vec![Credential {
                id: "production".into(),
                name: "production".into(),
                username: "deploy".into(),
                kind: CredentialKind::Key,
                key_path: Some("~/.ssh/id_ed25519".into()),
            }],
            crate::config::AppConfig::default(),
            vec![],
        );
        app.config.default_credential_id = Some("production".into());
        app.screen = Screen::CredentialPicker {
            host_idx: 0,
            after_failure,
        };
        app
    }

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn compact_picker_shows_username_name_default_and_actions() {
        let app = test_app(false);
        for (width, height) in [(60, 20), (80, 24)] {
            let screen = rendered(&app, width, height);
            for expected in [
                "Connect as · web-01",
                "NAME",
                "USERNAME",
                "TYPE",
                "production",
                "deploy",
                "★",
                "[Enter] connect",
                "[A] add",
                "[Esc] cancel",
            ] {
                assert!(
                    screen.contains(expected),
                    "missing {expected:?} at {width}x{height}:\n{screen}"
                );
            }
            assert!(!screen.contains("SSH failed"));
        }
    }

    #[test]
    fn failed_picker_does_not_assume_authentication_failure() {
        let app = test_app(true);
        for (width, height) in [(60, 20), (80, 24)] {
            let screen = rendered(&app, width, height);
            assert!(screen.contains("SSH failed; credentials may not be the cause."));
            assert!(screen.contains("production"));
            assert!(screen.contains("[Enter] connect"));
            assert!(!screen.to_lowercase().contains("auth failed"));
        }
    }

    #[test]
    fn empty_picker_and_selected_last_row_render_on_small_terminals() {
        let mut app = test_app(true);
        app.credentials.clear();
        let screen = rendered(&app, 60, 20);
        assert!(screen.contains("No credentials configured yet."));
        assert!(screen.contains("[A] add credential"));
        assert!(screen.contains("[Esc] cancel"));
        assert!(!screen.contains("[Enter] connect"));
        app = test_app(false);
        let template = app.credentials[0].clone();
        app.credentials = (0..40)
            .map(|i| Credential {
                id: format!("key-{i:02}"),
                name: format!("key-{i:02}"),
                ..template.clone()
            })
            .collect();
        app.popup_selected = 99;
        assert!(rendered(&app, 60, 20).contains("▶ key-39"));
        for (width, height) in [(1, 1), (20, 5), (40, 10)] {
            rendered(&app, width, height);
        }
    }
}
