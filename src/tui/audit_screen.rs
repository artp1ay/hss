use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

use crate::audit::HostStatus;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let hotkeys = &[
        ("R", "re-test all"),
        ("Enter", "test selected"),
        ("C", "connect"),
        ("Esc/Q", "back"),
    ];
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkeys, area.width);
    let hotkey_height = hotkey_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Title bar
            Constraint::Length(3), // Summary bar
            Constraint::Min(0),    // Table
            Constraint::Length(hotkey_height), // Hotkey bar
        ])
        .split(area);

    let state = match &app.audit_state {
        Some(s) => s,
        None => {
            f.render_widget(
                Paragraph::new("Audit not started. Press [R] to run audit.")
                    .alignment(ratatui::layout::Alignment::Center),
                chunks[2],
            );
            return;
        }
    };

    // 1. Title bar
    let title = Line::from(vec![
        Span::styled(
            "hss",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " · audit & connectivity monitor",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // 2. Summary stats
    let mut ok_cnt = 0;
    let mut auth_cnt = 0;
    let mut unreach_cnt = 0;
    let mut no_cred_cnt = 0;
    let mut checking_cnt = 0;

    for r in &state.results {
        match &r.status {
            HostStatus::Ok => ok_cnt += 1,
            HostStatus::AuthFailed(_) => auth_cnt += 1,
            HostStatus::Unreachable(_) | HostStatus::PortClosed(_) | HostStatus::Error(_) => {
                unreach_cnt += 1
            }
            HostStatus::NoCredential => no_cred_cnt += 1,
            HostStatus::Checking | HostStatus::Pending => checking_cnt += 1,
        }
    }

    let summary_line = Line::from(vec![
        Span::styled(format!("Total: {}  ·  ", state.total), Style::default().fg(Color::White)),
        Span::styled(format!("● OK: {ok_cnt}  "), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::styled(format!("⚠ Auth Fail: {auth_cnt}  "), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        Span::styled(format!("✖ Down: {unreach_cnt}  "), Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
        Span::styled(format!("? No Creds: {no_cred_cnt}  "), Style::default().fg(Color::Magenta)),
        if state.is_running {
            Span::styled(
                format!("⏳ In Progress [{}/{}]", state.completed, state.total),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled("✓ Finished", Style::default().fg(Color::DarkGray))
        },
    ]);

    let border_style = if state.is_running {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    f.render_widget(
        Paragraph::new(summary_line).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(" Audit Summary "),
        ),
        chunks[1],
    );

    // 3. Table
    let header = Row::new(vec![
        "STATUS", "HOST", "GROUP", "ADDRESS", "LATENCY", "CREDENTIAL", "DETAILS",
    ])
    .style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = state
        .results
        .iter()
        .map(|r| {
            let (status_badge, status_style) = match &r.status {
                HostStatus::Ok => ("● OK", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                HostStatus::AuthFailed(_) => ("⚠ AUTH", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                HostStatus::Unreachable(_) => ("✖ DOWN", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                HostStatus::PortClosed(_) => ("⊘ CLOSED", Style::default().fg(Color::Red)),
                HostStatus::NoCredential => ("? NO CRED", Style::default().fg(Color::Magenta)),
                HostStatus::Checking => ("⏳ CHECK", Style::default().fg(Color::Cyan)),
                HostStatus::Pending => ("· PENDING", Style::default().fg(Color::DarkGray)),
                HostStatus::Error(_) => ("! ERROR", Style::default().fg(Color::Red)),
            };

            let latency_cell = match r.latency_ms {
                Some(ms) if ms < 50 => {
                    Cell::from(format!("{ms}ms")).style(Style::default().fg(Color::Green))
                }
                Some(ms) if ms < 200 => {
                    Cell::from(format!("{ms}ms")).style(Style::default().fg(Color::Yellow))
                }
                Some(ms) => {
                    Cell::from(format!("{ms}ms")).style(Style::default().fg(Color::Indexed(208)))
                }
                None => Cell::from("—").style(Style::default().fg(Color::DarkGray)),
            };

            let cred_display = r.credential_name.clone().unwrap_or_else(|| "—".into());
            let addr_display = format!("{}:{}", r.ip, r.port);

            Row::new(vec![
                Cell::from(status_badge).style(status_style),
                Cell::from(r.host_name.clone()).style(Style::default().fg(Color::White)),
                Cell::from(r.group.clone()).style(Style::default().fg(group_color(&r.group))),
                Cell::from(addr_display).style(Style::default().fg(Color::DarkGray)),
                latency_cell,
                Cell::from(cred_display).style(Style::default().fg(Color::Cyan)),
                Cell::from(r.detail.clone()).style(Style::default().fg(Color::Gray)),
            ])
        })
        .collect();

    let selected = state.selected.min(state.results.len().saturating_sub(1));
    let mut table_state = TableState::default().with_selected(if state.results.is_empty() {
        None
    } else {
        Some(selected)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(11), // STATUS
            Constraint::Length(18), // HOST
            Constraint::Length(14), // GROUP
            Constraint::Length(22), // ADDRESS
            Constraint::Length(9),  // LATENCY
            Constraint::Length(16), // CREDENTIAL
            Constraint::Min(20),    // DETAILS
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    if state.results.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No hosts configured to audit.",
                Style::default().fg(Color::DarkGray),
            ))
            .alignment(ratatui::layout::Alignment::Center),
            chunks[2],
        );
    } else {
        f.render_stateful_widget(table, chunks[2], &mut table_state);
    }

    // 4. Hotkeys
    f.render_widget(Paragraph::new(hotkey_lines), chunks[3]);
}

fn group_color(group: &str) -> Color {
    let mut h: u32 = 2166136261;
    for b in group.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    const PALETTE: [Color; 12] = [
        Color::Green,
        Color::Cyan,
        Color::Yellow,
        Color::Magenta,
        Color::LightBlue,
        Color::LightGreen,
        Color::LightCyan,
        Color::LightMagenta,
        Color::Indexed(208),
        Color::Indexed(141),
        Color::Indexed(219),
        Color::Indexed(115),
    ];
    PALETTE[(h % PALETTE.len() as u32) as usize]
}

pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    let results_len = app.audit_state.as_ref().map(|s| s.results.len()).unwrap_or(0);

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.screen = Screen::Main;
        }
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
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.start_audit();
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if let Some(ref mut state) = app.audit_state {
                let sel = state.selected;
                state.start_single(
                    sel,
                    app.hosts.clone(),
                    app.credentials.clone(),
                    app.server_records.clone(),
                    app.config.clone(),
                );
            }
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(ref state) = app.audit_state {
                if !state.results.is_empty() {
                    let sel = state.selected.min(state.results.len() - 1);
                    let host_id = state.results[sel].host_id.clone();
                    if let Some(h) = app.hosts.iter().find(|h| h.id == host_id) {
                        let last_cred_id = app.last_credential_id(&h.id).map(|s| s.to_string());
                        let cred = crate::ssh::resolve_credential(&app.credentials, &app.config, last_cred_id.as_deref())?.cloned();
                        if let Some(c) = cred {
                            crate::tui::do_connect(terminal, app, &h.id, &c)?;
                        } else {
                            if let Some(idx) = app.hosts.iter().position(|x| x.id == h.id) {
                                app.popup_selected = 0;
                                app.screen = Screen::CredentialPicker {
                                    host_idx: idx,
                                    after_failure: false,
                                };
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}
