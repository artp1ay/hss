use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Frame,
};

use crate::audit::{AuditState, HostStatus};
use crate::tui::{theme, App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    draw_state(
        f,
        app.audit_state.as_ref(),
        crate::tui::help::content_area(app, f.area()),
    );
}

fn status_label(status: &HostStatus) -> (&'static str, Color) {
    match status {
        HostStatus::Ok => ("OK", theme::SUCCESS),
        HostStatus::AuthFailed(_) => ("AUTH FAIL", theme::WARNING),
        HostStatus::Unreachable(_) => ("DOWN", theme::ERROR),
        HostStatus::PortClosed(_) => ("CLOSED", theme::ERROR),
        HostStatus::NoCredential => ("NO CREDS", Color::Magenta),
        HostStatus::Checking => ("CHECKING", theme::INFO),
        HostStatus::Pending => ("PENDING", theme::MUTED),
        HostStatus::Error(_) => ("ERROR", theme::ERROR),
    }
}

fn draw_state(f: &mut Frame, state: Option<&AuditState>, area: ratatui::layout::Rect) {
    let running = state.is_some_and(|s| s.is_running);
    let hotkeys = if running {
        vec![
            ("↑/↓", "select"),
            ("C", "connect"),
            ("Esc/Q", "back"),
            ("R/Enter", "busy"),
        ]
    } else {
        vec![
            ("↑/↓", "select"),
            ("R", "test all"),
            ("Enter", "test one"),
            ("C", "connect"),
            ("Esc/Q", "back"),
        ]
    };
    let hotkey_lines = crate::tui::wrap_hotkey_lines(&hotkeys, area.width);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Min(1),
            Constraint::Length(5),
            Constraint::Length(hotkey_lines.len() as u16),
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
            Span::styled(" · audit & connectivity", Style::default().fg(theme::MUTED)),
        ])),
        chunks[0],
    );
    f.render_widget(Paragraph::new(hotkey_lines), chunks[4]);

    let Some(state) = state else {
        f.render_widget(
            Paragraph::new("Audit not started. Press R to test hosts."),
            chunks[2],
        );
        return;
    };

    let (mut ok, mut auth, mut down, mut no_creds, mut pending) = (0, 0, 0, 0, 0);
    for result in &state.results {
        match result.status {
            HostStatus::Ok => ok += 1,
            HostStatus::AuthFailed(_) => auth += 1,
            HostStatus::Unreachable(_) | HostStatus::PortClosed(_) | HostStatus::Error(_) => {
                down += 1
            }
            HostStatus::NoCredential => no_creds += 1,
            HostStatus::Checking | HostStatus::Pending => pending += 1,
        }
    }
    let progress = if running {
        "Running"
    } else if state.total == 0 {
        "No hosts"
    } else if state.completed == 0 {
        "Ready"
    } else {
        "Finished"
    };
    let summary = vec![
        Line::from(format!(
            "Hosts {} | {progress} {}/{}",
            state.results.len(),
            state.completed,
            state.total
        )),
        Line::from(vec![
            Span::styled(format!("OK {ok}  "), Style::default().fg(theme::SUCCESS)),
            Span::styled(
                format!("Auth {auth}  "),
                Style::default().fg(theme::WARNING),
            ),
            Span::styled(format!("Down {down}  "), Style::default().fg(theme::ERROR)),
            Span::styled(
                format!("No creds {no_creds}  "),
                Style::default().fg(Color::Magenta),
            ),
            Span::styled(
                format!("Pending {pending}"),
                Style::default().fg(theme::INFO),
            ),
        ]),
    ];
    f.render_widget(
        Paragraph::new(summary).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Audit summary "),
        ),
        chunks[1],
    );

    let compact = area.width < 100;
    let medium = area.width >= 75 && compact;
    let headers = if medium {
        vec!["STATUS", "HOST", "ADDRESS", "LATENCY"]
    } else if compact {
        vec!["STATUS", "HOST", "LATENCY"]
    } else {
        vec![
            "STATUS",
            "HOST",
            "GROUP",
            "ADDRESS",
            "LATENCY",
            "CREDENTIAL",
        ]
    };
    let widths = if medium {
        vec![
            Constraint::Length(9),
            Constraint::Min(12),
            Constraint::Length(24),
            Constraint::Length(8),
        ]
    } else if compact {
        vec![
            Constraint::Length(9),
            Constraint::Min(12),
            Constraint::Length(8),
        ]
    } else {
        vec![
            Constraint::Length(9),
            Constraint::Min(16),
            Constraint::Length(14),
            Constraint::Length(24),
            Constraint::Length(8),
            Constraint::Length(16),
        ]
    };
    let rows = state.results.iter().map(|r| {
        let (label, color) = status_label(&r.status);
        let mut cells = vec![
            Cell::from(label).style(Style::default().fg(color)),
            Cell::from(r.host_name.clone()).style(Style::default().fg(theme::TEXT)),
        ];
        if !compact {
            cells.push(Cell::from(r.group.clone()));
        }
        if !compact || medium {
            cells.push(Cell::from(format!("{}:{}", r.ip, r.port)));
        }
        cells.push(Cell::from(
            r.latency_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".into()),
        ));
        if !compact {
            cells.push(Cell::from(
                r.credential_name.clone().unwrap_or_else(|| "—".into()),
            ));
        }
        Row::new(cells)
    });
    let selected = state.selected.min(state.results.len().saturating_sub(1));
    if state.results.is_empty() {
        f.render_widget(Paragraph::new("No hosts configured to audit."), chunks[2]);
    } else {
        let mut table_state = TableState::default().with_selected(Some(selected));
        f.render_stateful_widget(
            Table::new(rows, widths)
                .header(
                    Row::new(headers).style(
                        Style::default()
                            .fg(theme::MUTED)
                            .add_modifier(Modifier::BOLD),
                    ),
                )
                .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                .highlight_symbol("> "),
            chunks[2],
            &mut table_state,
        );
    }
    if let Some(r) = state.results.get(selected) {
        let (label, color) = status_label(&r.status);
        let detail = if r.detail.is_empty() {
            match &r.status {
                HostStatus::AuthFailed(s)
                | HostStatus::Unreachable(s)
                | HostStatus::PortClosed(s)
                | HostStatus::Error(s) => s.as_str(),
                _ => label,
            }
        } else {
            &r.detail
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{label} · "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(detail.replace(['\n', '\r'], " ")),
            ]),
            Line::from(format!(
                "{}:{} | group: {} | credential: {}",
                r.ip,
                r.port,
                r.group,
                r.credential_name.as_deref().unwrap_or("none")
            )),
        ];
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: true }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Selected host details "),
            ),
            chunks[3],
        );
    }
}

pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    let results_len = app
        .audit_state
        .as_ref()
        .map(|s| s.results.len())
        .unwrap_or(0);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => app.screen = Screen::Main,
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(ref mut state) = app.audit_state {
                if results_len > 0 {
                    state.selected = (state.selected + 1).min(results_len - 1);
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(ref mut state) = app.audit_state {
                state.selected = state.selected.saturating_sub(1);
            }
        }
        KeyCode::Char('r') | KeyCode::Char('R') | KeyCode::Enter | KeyCode::Char(' ')
            if app.audit_state.as_ref().is_some_and(|s| s.is_running) =>
        {
            app.status_message = Some(crate::types::StatusMessage::info(
                "Audit is running; wait before starting another test.",
            ));
        }
        KeyCode::Char('r') | KeyCode::Char('R') => app.start_audit(),
        KeyCode::Enter | KeyCode::Char(' ') => {
            if let Some(ref mut state) = app.audit_state {
                state.start_single(
                    state.selected,
                    app.hosts.clone(),
                    app.credentials.clone(),
                    app.server_records.clone(),
                    app.config.clone(),
                );
            }
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            let host_id = app.audit_state.as_ref().and_then(|state| {
                state
                    .results
                    .get(state.selected.min(state.results.len().saturating_sub(1)))
                    .map(|r| r.host_id.clone())
            });
            if let Some(host_id) = host_id {
                let last_cred_id = app.last_credential_id(&host_id).map(str::to_string);
                let cred = crate::ssh::resolve_credential(
                    &app.credentials,
                    &app.config,
                    last_cred_id.as_deref(),
                )?
                .cloned();
                if let Some(c) = cred {
                    crate::tui::do_connect(terminal, app, &host_id, &c)?;
                } else if let Some(idx) = app.hosts.iter().position(|h| h.id == host_id) {
                    app.popup_selected = 0;
                    app.screen = Screen::CredentialPicker {
                        host_idx: idx,
                        after_failure: false,
                    };
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn compact_audit_render_has_status_details_progress_and_footer() {
        let mut state = AuditState::new(&[]);
        state.results.push(crate::audit::HostAuditResult {
            host_id: "test".into(),
            host_name: "prod-db".into(),
            ip: "10.0.0.1".into(),
            port: 22,
            group: "prod".into(),
            status: HostStatus::AuthFailed("Permission denied".into()),
            latency_ms: Some(12),
            credential_name: Some("deploy".into()),
            detail: "Permission denied".into(),
        });
        state.total = 1;
        state.completed = 1;
        for (width, height) in [(60, 20), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| draw_state(f, Some(&state), f.area()))
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            for text in [
                "AUTH FAIL",
                "prod-db",
                "12ms",
                "Permission denied",
                "10.0.0.1:22",
                "credential: deploy",
                "Finished 1/1",
                "Esc/Q",
                "test one",
            ] {
                assert!(rendered.contains(text), "{width}x{height}: missing {text}");
            }
        }
    }

    #[test]
    fn empty_audit_still_renders_navigation() {
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal.draw(|f| draw_state(f, None, f.area())).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Esc/Q"));
        assert!(rendered.contains("Audit not started"));
    }
}
