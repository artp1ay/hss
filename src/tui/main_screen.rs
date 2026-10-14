use crate::tui::{theme, App, Screen, Term};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = crate::tui::help::content_area(app, f.area());

    let footer_lines = footer(app.search_focused, area.width);
    let chunks = main_layout(area, footer_lines.len() as u16);

    let hosts = app.filtered_hosts();

    // Title bar
    let title = Line::from(vec![
        Span::styled(
            "hss",
            Style::default()
                .fg(theme::TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {}/{} hosts", hosts.len(), app.hosts.len()),
            Style::default().fg(theme::MUTED),
        ),
        Span::styled(
            if app.copy_id_rx.is_some() {
                " · Copy ID running [P]"
            } else {
                ""
            },
            Style::default().fg(theme::INFO),
        ),
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // Search box
    let border_style = Style::default().fg(if app.search_focused {
        theme::FOCUS
    } else {
        theme::MUTED
    });
    let search_content = if app.search_query.is_empty() && !app.search_focused {
        Line::from(Span::styled(
            "Search names, groups, addresses, tags…",
            Style::default().fg(theme::MUTED),
        ))
    } else if app.search_focused {
        Line::from(crate::tui::input::spans_with_width(
            &app.search_query,
            app.search_cursor,
            Style::default().fg(theme::TEXT),
            usize::from(chunks[1].width.saturating_sub(2)),
        ))
    } else {
        Line::from(Span::styled(
            app.search_query.clone(),
            Style::default().fg(theme::TEXT),
        ))
    };
    f.render_widget(
        Paragraph::new(search_content).block(
            Block::default()
                .title(if app.search_focused {
                    " Search · editing "
                } else {
                    " Search [/] "
                })
                .borders(Borders::ALL)
                .border_style(border_style),
        ),
        chunks[1],
    );

    // Keep the endpoint visible at compact widths; secondary fields live in details.
    let show_tags = area.width >= 80;
    let show_description = area.width >= 110;
    let tags_width = if show_description { 20 } else { 14 };
    let mut headings = vec!["NAME", "GROUP", "ENDPOINT"];
    let mut widths = vec![
        Constraint::Min(10),
        Constraint::Length(if show_tags { 12 } else { 10 }),
        Constraint::Length(if show_tags { 28 } else { 24 }),
    ];
    if show_tags {
        headings.push("TAGS");
        widths.push(Constraint::Length(tags_width as u16));
    }
    if show_description {
        headings.push("DESCRIPTION");
        widths.push(Constraint::Percentage(25));
    }
    let header = Row::new(headings).style(
        Style::default()
            .fg(theme::MUTED)
            .add_modifier(Modifier::BOLD),
    );
    let rows: Vec<Row> = hosts
        .iter()
        .map(|h| {
            let mut cells = vec![
                Cell::from(h.name.clone()).style(Style::default().fg(theme::TEXT)),
                Cell::from(h.group.clone()).style(Style::default().fg(group_color(&h.group))),
                Cell::from(endpoint(h)).style(Style::default().fg(theme::TEXT)),
            ];
            if show_tags {
                cells.push(
                    Cell::from(fit_tags(&h.tags, tags_width))
                        .style(Style::default().fg(theme::INFO)),
                );
            }
            if show_description {
                cells.push(
                    Cell::from(h.description.clone().unwrap_or_default())
                        .style(Style::default().fg(theme::MUTED)),
                );
            }
            Row::new(cells)
        })
        .collect();

    let selected = app.selected_row.min(hosts.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if hosts.is_empty() {
        None
    } else {
        Some(selected)
    });

    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(Style::default().bg(theme::SELECTION))
        .highlight_symbol("▶ ");

    if hosts.is_empty() {
        let hint = if app.search_query.is_empty() {
            "No hosts — press [N] to add, [I] to import"
        } else {
            "No matches — [Esc] clears search when focused"
        };
        f.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(theme::MUTED)))
                .alignment(ratatui::layout::Alignment::Center),
            chunks[2],
        );
    } else {
        f.render_stateful_widget(table, chunks[2], &mut state);
    }

    if let Some(host) = hosts.get(selected) {
        f.render_widget(
            Paragraph::new(host_details(app, host, chunks[3].width)).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme::MUTED)),
            ),
            chunks[3],
        );
    }
    f.render_widget(Paragraph::new(footer_lines), chunks[4]);
}

fn main_layout(area: Rect, footer_height: u16) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(4),
            Constraint::Length(footer_height),
        ])
        .split(area)
}

fn page_rows(area: Rect) -> usize {
    let footer_height = footer(false, area.width).len() as u16;
    usize::from(
        main_layout(area, footer_height)[2]
            .height
            .saturating_sub(1)
            .max(1),
    )
}

fn endpoint(host: &crate::types::Host) -> String {
    if host.ip.contains(':') && !host.ip.starts_with('[') {
        format!("[{}]:{}", host.ip, host.port)
    } else {
        format!("{}:{}", host.ip, host.port)
    }
}

fn host_details(app: &App, host: &crate::types::Host, width: u16) -> Vec<Line<'static>> {
    let credential = match crate::ssh::resolve_host_credential(
        host,
        &app.credentials,
        &app.config,
        app.last_credential_id(&host.id),
    ) {
        Ok(Some(cred)) => format!("{} · {}", cred.username, cred.name),
        Ok(None) => "choose credentials on connect".into(),
        Err(_) => "selected credential missing — edit host".into(),
    };
    let (route, route_color) = match crate::ssh::resolve_jump_spec(&app.hosts, host) {
        Ok(Some(spec)) => {
            let name = app
                .hosts
                .iter()
                .find(|h| Some(&h.id) == host.jump_host_id.as_ref())
                .map(|h| h.name.as_str())
                .unwrap_or("");
            (
                format!("via {name} ({spec}) → {}", endpoint(host)),
                theme::TEXT,
            )
        }
        Ok(None) => (format!("direct → {}", endpoint(host)), theme::TEXT),
        Err(err) => (format!("invalid jump: {err}"), theme::WARNING),
    };
    let tags = fit_tags(&host.tags, usize::from(width.saturating_sub(6)));
    let mut metadata = vec![
        Span::styled("Tags: ", Style::default().fg(theme::MUTED)),
        Span::styled(
            if tags.is_empty() { "—".into() } else { tags },
            Style::default().fg(theme::INFO),
        ),
    ];
    if let Some(description) = host.description.as_deref().filter(|s| !s.is_empty()) {
        metadata.push(Span::styled(
            format!(" · {description}"),
            Style::default().fg(theme::MUTED),
        ));
    }
    vec![
        Line::from(vec![
            Span::styled("Connect as: ", Style::default().fg(theme::MUTED)),
            Span::styled(credential, Style::default().fg(theme::TEXT)),
        ]),
        Line::from(vec![
            Span::styled("Route: ", Style::default().fg(theme::MUTED)),
            Span::styled(route, Style::default().fg(route_color)),
        ]),
        Line::from(metadata),
    ]
}

fn footer(search_focused: bool, width: u16) -> Vec<Line<'static>> {
    let pairs: &[(&str, &str)] = if search_focused {
        &[
            ("←/→", "cursor"),
            ("Enter", "table"),
            ("Esc", "clear/close"),
            ("Tab", "table"),
            ("F1", "help"),
        ]
    } else {
        &[
            ("↑↓", "select"),
            ("Enter", "connect"),
            ("R", "connect as"),
            ("/", "search"),
            ("N", "new"),
            ("E", "edit"),
            ("D", "delete"),
            ("P", "ssh-copy-id"),
            ("I", "import/export"),
            ("C", "credentials"),
            ("S", "settings"),
            ("A", "audit"),
            ("M", "mcp server"),
            ("Q", "quit"),
            ("?", "help"),
        ]
    };
    crate::tui::wrap_hotkey_lines(pairs, width)
}

/// Join tags into terminal `width` columns; when they don't fit, show whole tags
/// that do fit plus a " +N" marker so hidden tags are visible.
fn fit_tags(tags: &[String], width: usize) -> String {
    let full = tags.join(", ");
    if crate::tui::input::str_width(&full) <= width {
        return full;
    }
    let mut out = String::new();
    let mut shown = 0;
    for t in tags {
        let cand = if out.is_empty() {
            t.clone()
        } else {
            format!("{out}, {t}")
        };
        let suffix = format!(" +{}", tags.len() - shown - 1);
        let suffix_width = crate::tui::input::str_width(&suffix);
        if crate::tui::input::str_width(&cand) + suffix_width <= width {
            out = cand;
            shown += 1;
        } else {
            break;
        }
    }
    if shown == 0 {
        return format!("+{}", tags.len());
    }
    format!("{out} +{}", tags.len() - shown)
}

fn group_color(group: &str) -> Color {
    // FNV-1a: a byte sum collides on anagrams and similar-length names
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
        Color::Indexed(208), // orange
        Color::Indexed(141), // violet
        Color::Indexed(219), // pink
        Color::Indexed(115), // teal
    ];
    PALETTE[(h % PALETTE.len() as u32) as usize]
}

pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    let hosts_len = app.filtered_hosts().len();

    // Search editing wins over hotkeys, but moving the cursor preserves selection.
    if handle_search_input(app, key) {
        return Ok(());
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') if !app.search_focused => {
            app.should_quit = true;
        }
        KeyCode::Tab => {
            app.search_focused = !app.search_focused;
            app.search_cursor = crate::tui::input::end_of(&app.search_query);
        }
        KeyCode::Esc if app.search_focused => {
            if !app.search_query.is_empty() {
                app.search_query.clear();
                app.search_cursor = 0;
                app.selected_row = 0;
            } else {
                app.search_focused = false;
            }
        }
        // Enter in search → drop to table (don't connect yet)
        KeyCode::Enter if app.search_focused => {
            app.search_focused = false;
        }
        KeyCode::Char('/') if !app.search_focused => {
            app.search_focused = true;
            app.search_cursor = crate::tui::input::end_of(&app.search_query);
        }
        KeyCode::Down | KeyCode::Char('j') if !app.search_focused => {
            if hosts_len > 0 {
                app.selected_row = (app.selected_row + 1).min(hosts_len - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') if !app.search_focused => {
            app.selected_row = app.selected_row.saturating_sub(1);
        }
        KeyCode::PageDown | KeyCode::PageUp if !app.search_focused => {
            let size = terminal.size()?;
            let rows = page_rows(crate::tui::help::content_area(
                app,
                Rect::new(0, 0, size.width, size.height),
            ));
            move_page(app, rows, key.code == KeyCode::PageDown);
        }
        KeyCode::Home | KeyCode::End if !app.search_focused => {
            app.selected_row = if key.code == KeyCode::Home {
                0
            } else {
                hosts_len.saturating_sub(1)
            };
        }
        KeyCode::Char('c') | KeyCode::Char('C') if !app.search_focused => {
            app.screen = Screen::Credentials;
            app.cred_selected = 0;
        }
        KeyCode::Char('s') | KeyCode::Char('S') if !app.search_focused => {
            app.settings_inputs = vec![
                app.config.default_user.clone().unwrap_or_default(),
                app.config.default_port.to_string(),
                app.config.default_group.clone(),
                app.config.connect_timeout.to_string(),
                app.config.strict_host_checking.clone(),
                app.config.ssh_extra_args.clone(),
                if app.config.auto_save_credential {
                    "yes".into()
                } else {
                    "no".into()
                },
                app.config.exec_timeout.to_string(),
                app.config.mcp_port.to_string(),
                app.config.mcp_token.clone().unwrap_or_default(),
                if app.config.mcp_read_only {
                    "yes".into()
                } else {
                    "no".into()
                },
                app.config.mcp_allowed_hosts.clone(),
                if app.config.mcp_dlp_filter {
                    "yes".into()
                } else {
                    "no".into()
                },
                app.config.mcp_log_file.clone().unwrap_or_default(),
                app.config.audit_timeout.to_string(),
            ];
            app.settings_focused_field = 0;
            app.settings_cursor = crate::tui::input::end_of(&app.settings_inputs[0]);
            app.settings_reveal_token = false;
            app.settings_error = None;
            app.settings_confirm_discard = false;
            app.screen = Screen::Settings;
        }
        KeyCode::Char('a') | KeyCode::Char('A') if !app.search_focused => {
            app.start_audit();
            app.screen = Screen::Audit;
        }
        KeyCode::Char('r') | KeyCode::Char('R') if !app.search_focused && hosts_len > 0 => {
            let idx_in_all = get_host_idx_in_all(app);
            if let Some(idx) = idx_in_all {
                app.popup_selected = 0;
                app.screen = Screen::CredentialPicker {
                    host_idx: idx,
                    after_failure: false,
                };
            }
        }
        KeyCode::Enter if !app.search_focused && hosts_len > 0 => {
            connect_selected(terminal, app)?;
        }
        KeyCode::Char('n') | KeyCode::Char('N') if !app.search_focused => {
            let default_port = if app.config.default_port != 0 {
                app.config.default_port.to_string()
            } else {
                "22".into()
            };
            app.host_form = Some(crate::types::HostForm {
                editing_id: None,
                port: default_port,
                group: app.config.default_group.clone(),
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
                    jump_host_id: h.jump_host_id.clone(),
                    credential_id: h.credential_id.clone(),
                    focused: 0,
                    cursor: crate::tui::input::end_of(&h.name),
                    error_message: None,
                });
                app.screen = Screen::HostForm;
            }
        }
        KeyCode::Char('d') | KeyCode::Char('D') if !app.search_focused && hosts_len > 0 => {
            if let Some(idx) = get_host_idx_in_all(app) {
                let name = app.hosts[idx].name.clone();
                if app.skip_host_delete_confirm {
                    crate::tui::delete_popup::delete_host(app, idx)?;
                } else {
                    app.delete_popup = Some(crate::types::DeletePopup {
                        kind: crate::types::DeleteKind::Host,
                        name,
                        idx,
                        dont_ask: false,
                    });
                }
            }
        }
        KeyCode::Char('p') | KeyCode::Char('P')
            if !app.search_focused && (hosts_len > 0 || app.copy_id_rx.is_some()) =>
        {
            if app.copy_id_rx.is_some() {
                app.screen = Screen::CopyId;
                return Ok(());
            }
            if let Some(idx) = get_host_idx_in_all(app) {
                let host = &app.hosts[idx];
                let mut keys: Vec<(String, bool)> = crate::ssh::find_public_keys(&app.credentials)
                    .into_iter()
                    .map(|p| (p, false))
                    .collect();
                if keys.len() == 1 {
                    keys[0].1 = true;
                }
                let user = host
                    .user
                    .clone()
                    .or_else(|| app.config.default_user.clone())
                    .or_else(|| std::env::var("USER").ok())
                    .unwrap_or_default();
                app.copy_id_form = Some(crate::types::CopyIdForm {
                    host_id: host.id.clone(),
                    keys,
                    user,
                    ..Default::default()
                });
                app.screen = Screen::CopyId;
            }
        }
        KeyCode::Char('m') | KeyCode::Char('M') if !app.search_focused => {
            match crate::mcp::McpServer::start() {
                Ok(mcp) => {
                    app.mcp = Some(mcp);
                    app.screen = Screen::McpServer;
                    app.status_message = None;
                }
                Err(e) => {
                    app.status_message = Some(crate::types::StatusMessage::error(format!("{e}")));
                }
            }
        }
        KeyCode::Char('i') | KeyCode::Char('I') if !app.search_focused => {
            app.import_error = None;
            app.export_confirm_overwrite = false;
            app.import_path_input.clear();
            app.import_cursor = 0;
            app.import_export_mode = 0;
            app.screen = Screen::ImportHosts;
        }
        _ => {}
    }
    Ok(())
}

fn handle_search_input(app: &mut App, key: KeyEvent) -> bool {
    if !app.search_focused || matches!(key.code, KeyCode::Tab | KeyCode::Esc | KeyCode::Enter) {
        return false;
    }
    let previous = app.search_query.clone();
    let consumed = crate::tui::input::handle(&mut app.search_query, &mut app.search_cursor, key);
    if app.search_query != previous {
        app.selected_row = 0;
    }
    consumed
}

fn move_page(app: &mut App, rows: usize, down: bool) {
    let last = app.filtered_hosts().len().saturating_sub(1);
    app.selected_row = if down {
        app.selected_row.saturating_add(rows).min(last)
    } else {
        app.selected_row.saturating_sub(rows).min(last)
    };
}

fn get_host_idx_in_all(app: &App) -> Option<usize> {
    let filtered = app.filtered_hosts();
    if filtered.is_empty() {
        return None;
    }
    let host_id = &filtered[app.selected_row.min(filtered.len() - 1)].id;
    app.hosts.iter().position(|h| &h.id == host_id)
}

fn connect_selected(terminal: &mut Term, app: &mut App) -> Result<()> {
    let filtered = app.filtered_hosts();
    if filtered.is_empty() {
        return Ok(());
    }
    let host = filtered[app.selected_row.min(filtered.len() - 1)].clone();

    let last_cred_id = app.last_credential_id(&host.id).map(|s| s.to_string());
    let cred = match crate::ssh::resolve_host_credential(
        &host,
        &app.credentials,
        &app.config,
        last_cred_id.as_deref(),
    ) {
        Ok(credential) => credential.cloned(),
        Err(error) => {
            app.status_message = Some(crate::types::StatusMessage::error(error.to_string()));
            return Ok(());
        }
    };

    if let Some(cred) = cred {
        crate::tui::do_connect(terminal, app, &host.id, &cred)?;
    } else {
        if let Some(idx) = app.hosts.iter().position(|h| h.id == host.id) {
            app.popup_selected = 0;
            app.screen = Screen::CredentialPicker {
                host_idx: idx,
                after_failure: false,
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn test_app() -> App {
        App::new(vec![], vec![], crate::config::AppConfig::default(), vec![])
    }

    #[test]
    fn search_typing_does_not_trigger_hotkeys() {
        let mut app = test_app();
        let mut term = match ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(
            std::io::stdout(),
        )) {
            Ok(t) => t,
            Err(_) => return, // In headless CI environments without standard stdout TTY
        };
        app.search_focused = true;
        for c in "quit".chars() {
            handle_key(
                &mut term,
                &mut app,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            )
            .unwrap();
        }
        assert_eq!(app.search_query, "quit");
        assert!(!app.should_quit, "'q' must type in search, not quit");

        // Left/Right move the cursor; insert lands at the cursor
        handle_key(
            &mut term,
            &mut app,
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        )
        .unwrap();
        handle_key(
            &mut term,
            &mut app,
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(app.search_query, "qui-t");

        // Unfocused, 'q' still quits
        app.search_focused = false;
        handle_key(
            &mut term,
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        )
        .unwrap();
        assert!(app.should_quit);
    }

    fn host(id: &str) -> crate::types::Host {
        crate::types::Host {
            id: id.into(),
            name: id.into(),
            ip: "10.0.0.2".into(),
            group: "prod".into(),
            port: 2222,
            // SSH uses the resolved credential username, not this host hint.
            user: Some("host-user".into()),
            tags: vec!["web".into(), "prod".into()],
            description: Some("Application server".into()),
            jump_host_id: None,
            credential_id: None,
        }
    }

    #[test]
    fn host_form_initialization_keeps_explicit_choice_and_new_hosts_are_automatic() {
        let mut app = test_app();
        let mut selected = host("web-01");
        selected.credential_id = Some("production".into());
        app.hosts = vec![selected];
        let Ok(mut terminal) =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
        else {
            return;
        };
        handle_key(
            &mut terminal,
            &mut app,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(
            app.host_form.as_ref().unwrap().credential_id.as_deref(),
            Some("production")
        );
        app.screen = Screen::Main;
        handle_key(
            &mut terminal,
            &mut app,
            KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        )
        .unwrap();
        assert!(app.host_form.as_ref().unwrap().credential_id.is_none());
    }

    #[test]
    fn missing_explicit_connection_stays_on_main_and_reports_error_without_picker() {
        let mut app = test_app();
        let mut selected = host("web-01");
        selected.credential_id = Some("deleted".into());
        app.hosts = vec![selected];
        app.credentials = vec![credential("fallback", "fallback-user")];
        app.config.default_credential_id = Some("fallback".into());
        app.server_records = vec![crate::types::ServerRecord {
            host_id: "web-01".into(),
            last_credential_id: Some("fallback".into()),
        }];
        let original = serde_json::to_value((&app.hosts, &app.server_records)).unwrap();
        let Ok(mut terminal) =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
        else {
            return;
        };
        connect_selected(&mut terminal, &mut app).unwrap();
        assert_eq!(app.screen, Screen::Main);
        let status = app.status_message.as_ref().unwrap();
        assert_eq!(status.kind, crate::types::StatusKind::Error);
        assert!(status.text.contains("deleted"));
        assert!(!status.is_expired());
        assert_eq!(
            serde_json::to_value((&app.hosts, &app.server_records)).unwrap(),
            original
        );
    }

    fn credential(id: &str, username: &str) -> crate::types::Credential {
        crate::types::Credential {
            id: id.into(),
            name: id.into(),
            username: username.into(),
            kind: crate::types::CredentialKind::Key,
            key_path: Some("~/.ssh/id_ed25519".into()),
        }
    }

    fn rendered(app: &App, width: u16, height: u16) -> Vec<String> {
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
            .collect()
    }

    #[test]
    fn compact_render_preserves_table_details_counts_and_footer() {
        let mut app = test_app();
        app.hosts = vec![host("web-01"), host("db-01")];
        app.credentials = vec![credential("production", "deploy")];
        app.search_query = "web-01".into();
        for (width, height) in [(60, 20), (80, 24)] {
            let lines = rendered(&app, width, height);
            let screen = lines.join("\n");
            for expected in [
                "1/2 hosts",
                "NAME",
                "GROUP",
                "ENDPOINT",
                "▶ web-01",
                "10.0.0.2:2222",
                "Connect as: deploy · production",
                "Route: direct",
                "Tags: web, prod",
                "[R] connect as",
                "[?] help",
            ] {
                assert!(
                    screen.contains(expected),
                    "missing {expected:?} at {width}x{height}:\n{screen}"
                );
            }
            assert!(lines[usize::from(height - 1)].contains("[?] help"));
            let chunks = main_layout(
                Rect::new(0, 0, width, height),
                footer(false, width).len() as u16,
            );
            assert!(chunks[2].height >= 7);
        }
    }

    #[test]
    fn full_menu_remains_visible_with_hosts_empty_results_and_notifications() {
        let mut app = test_app();
        for (width, height) in [(60, 20), (80, 24), (100, 30), (160, 40)] {
            let footer_lines = footer(false, width);
            assert!(footer_lines
                .iter()
                .all(|line| line.width() <= usize::from(width)));
            for with_notification in [false, true] {
                app.status_message = with_notification
                    .then(|| crate::types::StatusMessage::error("Persistent warning"));
                for state in 0..3 {
                    app.hosts = if state == 0 {
                        vec![]
                    } else {
                        vec![host("web-01")]
                    };
                    app.search_query = if state == 2 {
                        "unmatched".into()
                    } else {
                        String::new()
                    };
                    let lines = rendered(&app, width, height);
                    let area = crate::tui::help::content_area(&app, Rect::new(0, 0, width, height));
                    let menu_area = main_layout(area, footer_lines.len() as u16)[4];
                    assert_eq!(usize::from(menu_area.height), footer_lines.len());
                    let menu =
                        lines[usize::from(menu_area.y)..usize::from(menu_area.bottom())].join("\n");
                    for expected in [
                        "[↑↓] select",
                        "[Enter] connect",
                        "[R] connect as",
                        "[/] search",
                        "[N] new",
                        "[E] edit",
                        "[D] delete",
                        "[P] ssh-copy-id",
                        "[I] import/export",
                        "[C] credentials",
                        "[S] settings",
                        "[A] audit",
                        "[M] mcp server",
                        "[Q] quit",
                        "[?] help",
                    ] {
                        assert!(
                            menu.contains(expected),
                            "missing {expected:?} at {width}x{height}, state={state}, notification={with_notification}:\n{menu}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn search_menu_shows_editing_keys_instead_of_inactive_actions() {
        let mut app = test_app();
        app.search_focused = true;
        for (width, height) in [(60, 20), (80, 24)] {
            let screen = rendered(&app, width, height).join("\n");
            for expected in [
                "[←/→] cursor",
                "[Enter] table",
                "[Esc] clear/close",
                "[Tab] table",
                "[F1] help",
            ] {
                assert!(screen.contains(expected), "missing {expected:?}:\n{screen}");
            }
            assert!(!screen.contains("[S] settings"));
            assert!(!screen.contains("[Q] quit"));
        }
    }

    #[test]
    fn details_resolve_last_default_and_ambiguous_credentials_and_jump() {
        let mut app = test_app();
        let mut selected = host("web-01");
        selected.jump_host_id = Some("bastion".into());
        let mut jump = host("bastion");
        jump.ip = "10.0.0.1".into();
        jump.port = 22;
        jump.user = Some("jump".into());
        app.hosts = vec![selected, jump];
        app.credentials = vec![
            credential("default-key", "default-user"),
            credential("last-key", "deploy"),
        ];
        app.config.default_credential_id = Some("default-key".into());
        app.server_records.push(crate::types::ServerRecord {
            host_id: "web-01".into(),
            last_credential_id: Some("last-key".into()),
        });
        let screen = rendered(&app, 80, 24).join("\n");
        assert!(screen.contains("Connect as: deploy · last-key"));
        assert!(screen.contains("via bastion (jump@10.0.0.1) → 10.0.0.2:2222"));
        assert!(!screen.contains("host-user"));
        app.hosts[0].credential_id = Some("default-key".into());
        assert!(rendered(&app, 80, 24)
            .join("\n")
            .contains("default-user · default-key"));
        app.hosts[0].credential_id = Some("deleted".into());
        assert!(rendered(&app, 80, 24)
            .join("\n")
            .contains("selected credential missing"));
        app.hosts[0].credential_id = None;
        app.server_records.clear();
        assert!(rendered(&app, 60, 20)
            .join("\n")
            .contains("default-user · default-key"));
        app.config.default_credential_id = None;
        assert!(rendered(&app, 60, 20)
            .join("\n")
            .contains("choose credentials on connect"));
        app.hosts[0].jump_host_id = Some("missing".into());
        assert!(rendered(&app, 60, 20)
            .join("\n")
            .contains("Route: invalid jump:"));
    }

    #[test]
    fn empty_and_no_match_render_keep_actions_available() {
        let mut app = test_app();
        for (width, height) in [(60, 20), (80, 24)] {
            let screen = rendered(&app, width, height).join("\n");
            assert!(screen.contains("0/0 hosts"));
            assert!(screen.contains("No hosts"));
            assert!(screen.contains("[N] new"));
            assert!(screen.contains("[?] help"));
        }
        app.hosts.push(host("web-01"));
        app.search_query = "unmatched".into();
        let screen = rendered(&app, 60, 20).join("\n");
        assert!(screen.contains("0/1 hosts"));
        assert!(screen.contains("No matches"));
        assert!(!screen.contains("Connect as:"));
    }

    #[test]
    fn search_cursor_and_noop_edits_preserve_selection() {
        let mut app = test_app();
        app.search_focused = true;
        app.search_query = "你好web".into();
        app.search_cursor = crate::tui::input::end_of(&app.search_query);
        app.selected_row = 3;
        for code in [
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::Backspace,
            KeyCode::End,
            KeyCode::Delete,
        ] {
            assert!(handle_search_input(
                &mut app,
                KeyEvent::new(code, KeyModifiers::NONE)
            ));
            assert_eq!(
                app.selected_row, 3,
                "cursor/no-op key {code:?} reset selection"
            );
            assert_eq!(app.search_query, "你好web");
        }
        assert!(handle_search_input(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)
        ));
        assert_eq!(app.search_query, "你好webq");
        assert_eq!(app.selected_row, 0);
        assert!(!app.should_quit);
        app.selected_row = 2;
        assert!(handle_search_input(
            &mut app,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE)
        ));
        assert_eq!(app.selected_row, 0);
        assert_eq!(app.search_query, "你好web");
    }

    #[test]
    fn pages_follow_visible_table_rows_and_clamp_to_filtered_hosts() {
        let mut app = test_app();
        app.hosts = (0..40).map(|i| host(&format!("web-{i:02}"))).collect();
        for (width, height) in [(60, 20), (80, 24)] {
            app.selected_row = 0;
            let expected = rendered(&app, width, height)
                .iter()
                .filter(|line| line.contains("10.0.0.2:2222"))
                .count()
                - 1; // The connection details also contain the endpoint.
            let rows = page_rows(Rect::new(0, 0, width, height));
            assert_eq!(rows, expected);
            app.selected_row = 0;
            move_page(&mut app, rows, true);
            assert_eq!(app.selected_row, expected);
            move_page(&mut app, rows, false);
            assert_eq!(app.selected_row, 0);
            move_page(&mut app, rows, false);
            assert_eq!(app.selected_row, 0);
            app.selected_row = 39;
            move_page(&mut app, rows, true);
            assert_eq!(app.selected_row, 39);
        }
        app.search_query = "web-0".into();
        app.selected_row = 0;
        move_page(&mut app, 14, true);
        assert_eq!(app.selected_row, 9);
        app.search_query = "missing".into();
        move_page(&mut app, 14, false);
        assert_eq!(app.selected_row, 0);
    }

    #[test]
    fn long_search_keeps_cursor_visible_at_compact_widths() {
        let mut app = test_app();
        app.search_focused = true;
        app.search_query = format!("{}Z", "你好".repeat(40));
        app.search_cursor = crate::tui::input::end_of(&app.search_query);
        for (width, height) in [(60, 20), (80, 24)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let search = (1..width - 1)
                .map(|x| buffer[(x, 2)].symbol())
                .collect::<String>();
            assert!(
                search.contains('Z'),
                "search end was clipped at {width}x{height}: {search}"
            );
            assert!(
                (1..width - 1).any(|x| buffer[(x, 2)].modifier.contains(Modifier::REVERSED)),
                "search cursor was clipped at {width}x{height}"
            );
        }
    }

    #[test]
    fn render_keeps_last_selected_host_visible_and_handles_tiny_terminals() {
        let mut app = test_app();
        app.hosts = (0..40).map(|i| host(&format!("web-{i:02}"))).collect();
        app.selected_row = 39;
        for (width, height) in [(60, 20), (80, 24)] {
            assert!(rendered(&app, width, height)
                .join("\n")
                .contains("▶ web-39"));
        }
        for (width, height) in [(1, 1), (20, 5), (40, 10)] {
            rendered(&app, width, height);
        }
    }

    #[test]
    fn fit_tags_fits_truncates_and_marks_hidden() {
        let tags: Vec<String> = ["web", "prod", "db"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(fit_tags(&tags, 28), "web, prod, db");
        assert_eq!(fit_tags(&tags, 10), "web +2");
        assert_eq!(fit_tags(&tags, 2), "+3");
        assert_eq!(fit_tags(&[], 10), "");
    }
}
