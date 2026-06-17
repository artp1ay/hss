use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let hosts = app.filtered_hosts();

    // Title bar
    let title = Line::from(vec![
        Span::styled("hss", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {} hosts", hosts.len()), Style::default().fg(Color::DarkGray)),
        if let Some(ref msg) = app.status_message {
            Span::styled(format!("  ⚠ {msg}"), Style::default().fg(Color::Yellow))
        } else {
            Span::raw("")
        },
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // Search box
    let border_style = if app.search_focused {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let search_content = if app.search_query.is_empty() && !app.search_focused {
        Line::from(Span::styled("Search...", Style::default().fg(Color::DarkGray)))
    } else {
        Line::from(vec![
            Span::styled(&app.search_query, Style::default().fg(Color::White)),
            if app.search_focused {
                Span::styled("█", Style::default().fg(Color::Blue))
            } else {
                Span::raw("")
            },
        ])
    };
    f.render_widget(
        Paragraph::new(search_content).block(Block::default().borders(Borders::ALL).border_style(border_style)),
        chunks[1],
    );

    // Server table
    let header = Row::new(vec!["NAME", "GROUP", "HOST", "PORT", "TAGS"])
        .style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD));

    let rows: Vec<Row> = hosts.iter().map(|h| {
        Row::new(vec![
            Cell::from(h.name.clone()),
            Cell::from(h.group.clone()).style(Style::default().fg(group_color(&h.group))),
            Cell::from(h.ip.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(h.port.to_string()).style(Style::default().fg(Color::DarkGray)),
            Cell::from({
                if h.tags.is_empty() {
                    "".to_string()
                } else {
                    h.tags.iter().take(2).cloned().collect::<Vec<_>>().join(", ")
                }
            }).style(Style::default().fg(Color::DarkGray)),
        ])
    }).collect();

    let selected = app.selected_row.min(hosts.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if hosts.is_empty() { None } else { Some(selected) });

    let table = Table::new(rows, [
        Constraint::Length(22),
        Constraint::Length(16),
        Constraint::Length(18),
        Constraint::Length(6),
        Constraint::Min(8),
    ])
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[2], &mut state);

    // Hotkey bar
    let hotkeys = if app.search_focused {
        hotkey_line(&[("Esc", "clear/back"), ("Tab", "table")])
    } else if app.delete_confirm.is_some() {
        Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::DarkGray)),
            Span::styled("[D]", Style::default().fg(Color::Red)),
            Span::styled(" again to confirm deletion, or any other key to cancel", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        hotkey_line(&[
            ("Enter", "connect"), ("N", "new"), ("E", "edit"), ("D", "delete"),
            ("I", "import"), ("R", "creds"), ("S", "settings"), ("Q", "quit"),
        ])
    };
    f.render_widget(Paragraph::new(hotkeys), chunks[3]);
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

fn group_color(group: &str) -> Color {
    let hash: usize = group.bytes().map(|b| b as usize).sum::<usize>() % 5;
    [Color::Green, Color::Cyan, Color::Yellow, Color::Magenta, Color::LightBlue][hash]
}

pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    let hosts_len = app.filtered_hosts().len();

    // D with pending confirm: actually delete
    if !app.search_focused {
        if matches!(key.code, KeyCode::Char('d') | KeyCode::Char('D')) && app.delete_confirm.is_some() {
            if let Some(idx) = app.delete_confirm.take() {
                app.hosts.remove(idx);
                app.save_hosts()?;
                app.selected_row = app.selected_row.min(app.hosts.len().saturating_sub(1));
                app.status_message = Some("Host deleted.".into());
            }
            return Ok(());
        }
        // Clear delete confirm on any other key
        if !matches!(key.code, KeyCode::Char('d') | KeyCode::Char('D')) {
            if app.delete_confirm.is_some() {
                app.delete_confirm = None;
                app.status_message = None;
            }
        }
    }

    match key.code {
        KeyCode::Tab => {
            app.search_focused = !app.search_focused;
        }
        KeyCode::Esc if app.search_focused => {
            if !app.search_query.is_empty() {
                app.search_query.clear();
                app.selected_row = 0;
            } else {
                app.search_focused = false;
            }
        }
        KeyCode::Char(c) if app.search_focused => {
            app.search_query.push(c);
            app.selected_row = 0;
        }
        KeyCode::Backspace if app.search_focused => {
            app.search_query.pop();
            app.selected_row = 0;
        }
        KeyCode::Down | KeyCode::Char('j') if !app.search_focused => {
            if hosts_len > 0 {
                app.selected_row = (app.selected_row + 1).min(hosts_len - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') if !app.search_focused => {
            app.selected_row = app.selected_row.saturating_sub(1);
        }
        KeyCode::Char('q') | KeyCode::Char('Q') if !app.search_focused => {
            app.should_quit = true;
        }
        KeyCode::Char('c') | KeyCode::Char('C') if !app.search_focused => {
            app.screen = Screen::Credentials;
            app.cred_selected = 0;
        }
        KeyCode::Char('s') | KeyCode::Char('S') if !app.search_focused => {
            app.screen = Screen::Settings;
        }
        KeyCode::Char('r') | KeyCode::Char('R') if !app.search_focused && hosts_len > 0 => {
            let idx_in_all = get_host_idx_in_all(app);
            if let Some(idx) = idx_in_all {
                app.popup_selected = 0;
                app.screen = Screen::CredentialPicker { host_idx: idx, after_failure: false };
            }
        }
        KeyCode::Enter if !app.search_focused && hosts_len > 0 => {
            connect_selected(terminal, app)?;
        }
        KeyCode::Char('n') | KeyCode::Char('N') if !app.search_focused => {
            app.host_form = Some(crate::types::HostForm {
                editing_id: None,
                port: "22".into(),
                ..Default::default()
            });
            app.screen = Screen::HostForm;
        }
        KeyCode::Char('e') | KeyCode::Char('E') if !app.search_focused && hosts_len > 0 => {
            if let Some(idx) = get_host_idx_in_all(app) {
                let h = &app.hosts[idx];
                app.host_form = Some(crate::types::HostForm {
                    editing_id: Some(h.id.clone()),
                    name: h.name.clone(),
                    ip: h.ip.clone(),
                    group: h.group.clone(),
                    port: h.port.to_string(),
                    user: h.user.clone().unwrap_or_default(),
                    tags: h.tags.join(", "),
                    description: h.description.clone().unwrap_or_default(),
                    focused: 0,
                });
                app.screen = Screen::HostForm;
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') if !app.search_focused && hosts_len > 0 => {
            if let Some(idx) = get_host_idx_in_all(app) {
                app.delete_confirm = Some(idx);
                app.status_message = Some("Press D again to confirm deletion.".into());
            }
        }
        KeyCode::Char('i') | KeyCode::Char('I') if !app.search_focused => {
            app.import_path_input.clear();
            app.screen = Screen::ImportHosts;
        }
        _ => {}
    }
    Ok(())
}

fn get_host_idx_in_all(app: &App) -> Option<usize> {
    let filtered = app.filtered_hosts();
    if filtered.is_empty() { return None; }
    let host_name = &filtered[app.selected_row.min(filtered.len() - 1)].name;
    app.hosts.iter().position(|h| &h.name == host_name)
}

fn connect_selected(terminal: &mut Term, app: &mut App) -> Result<()> {
    let filtered = app.filtered_hosts();
    if filtered.is_empty() { return Ok(()); }
    let host = filtered[app.selected_row.min(filtered.len() - 1)].clone();

    let last_cred_id = app.last_credential_id(&host.name).map(|s| s.to_string());
    let cred = crate::ssh::resolve_credential(&app.credentials, &app.config, last_cred_id.as_deref())?
        .cloned();

    if let Some(cred) = cred {
        crate::tui::do_connect(terminal, app, &host.name, &cred)?;
    } else {
        // No credential resolved — show picker
        if let Some(idx) = app.hosts.iter().position(|h| h.name == host.name) {
            app.popup_selected = 0;
            app.screen = Screen::CredentialPicker { host_idx: idx, after_failure: false };
        }
    }
    Ok(())
}
