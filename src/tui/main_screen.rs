use crate::tui::{App, Screen, Term};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    // Hotkey bar computed first so height is dynamic and fits terminal width
    let hotkey_pairs: &[(&str, &str)] = if app.search_focused {
        &[
            ("←/→", "cursor"),
            ("Enter", "apply"),
            ("Esc", "clear / close search"),
            ("Tab", "table"),
        ]
    } else {
        &[
            ("Enter", "connect"),
            ("/", "search"),
            ("N", "new"),
            ("E", "edit"),
            ("D", "delete"),
            ("A", "audit"),
            ("I", "import/export"),
            ("R", "switch cred"),
            ("P", "ssh-copy-id"),
            ("M", "mcp server"),
            ("C", "credentials"),
            ("S", "settings"),
            ("Q", "quit"),
        ]
    };
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkey_pairs, area.width);
    let hotkey_height = hotkey_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(hotkey_height),
        ])
        .split(area);

    let hosts = app.filtered_hosts();

    // Title bar
    let title = Line::from(vec![
        Span::styled(
            "hss",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" · {} hosts", hosts.len()),
            Style::default().fg(Color::DarkGray),
        ),
        if let Some(ref msg) = app.status_message {
            let (icon, color) = match msg.kind {
                crate::types::StatusKind::Success => ("✓", Color::Green),
                crate::types::StatusKind::Info => ("i", Color::Cyan),
                crate::types::StatusKind::Warning => ("!", Color::Yellow),
                crate::types::StatusKind::Error => ("✗", Color::Red),
            };
            Span::styled(
                format!("  {icon} {}", msg.text),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )
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
        Line::from(vec![
            Span::styled("Search...", Style::default().fg(Color::DarkGray)),
            Span::styled(
                " (press [/] to type, [Esc] clear/close)",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    } else if app.search_focused {
        Line::from(crate::tui::input::spans(
            &app.search_query,
            app.search_cursor,
            Style::default().fg(Color::White),
        ))
    } else {
        Line::from(Span::styled(
            app.search_query.clone(),
            Style::default().fg(Color::White),
        ))
    };
    f.render_widget(
        Paragraph::new(search_content).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style),
        ),
        chunks[1],
    );

    // Server table
    let header = Row::new(vec!["NAME", "GROUP", "HOST", "PORT", "TAGS", "DESCRIPTION"]).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    // Tags column sized to content (capped) so tags aren't cut while space remains
    let tags_width = hosts
        .iter()
        .map(|h| h.tags.join(", ").chars().count())
        .max()
        .unwrap_or(0)
        .clamp(4, 28);

    let rows: Vec<Row> = hosts
        .iter()
        .map(|h| {
            Row::new(vec![
                Cell::from(h.name.clone()),
                Cell::from(h.group.clone()).style(Style::default().fg(group_color(&h.group))),
                Cell::from(h.ip.clone()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(h.port.to_string()).style(Style::default().fg(Color::DarkGray)),
                Cell::from(fit_tags(&h.tags, tags_width)).style(Style::default().fg(Color::Cyan)),
                Cell::from(h.description.clone().unwrap_or_default())
                    .style(Style::default().fg(Color::DarkGray)),
            ])
        })
        .collect();

    let selected = app.selected_row.min(hosts.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if hosts.is_empty() {
        None
    } else {
        Some(selected)
    });

    let table = Table::new(
        rows,
        [
            Constraint::Length(22),
            Constraint::Length(16),
            Constraint::Length(18),
            Constraint::Length(6),
            Constraint::Length(tags_width as u16),
            Constraint::Min(10),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    if hosts.is_empty() {
        let hint = if app.search_query.is_empty() {
            "No hosts — press [N] to add, [I] to import"
        } else {
            "No matches"
        };
        f.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)))
                .alignment(ratatui::layout::Alignment::Center),
            chunks[2],
        );
    } else {
        f.render_stateful_widget(table, chunks[2], &mut state);
    }

    // Hotkey bar
    f.render_widget(Paragraph::new(hotkey_lines), chunks[3]);
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

    // While the search box has focus, text editing wins over hotkeys
    // (Tab/Esc/Enter/Up/Down are not consumed by the editor).
    if app.search_focused && !matches!(key.code, KeyCode::Tab | KeyCode::Esc | KeyCode::Enter) {
        let (mut q, mut cur) = (std::mem::take(&mut app.search_query), app.search_cursor);
        let consumed = crate::tui::input::handle(&mut q, &mut cur, key);
        let changed = q != app.search_query;
        app.search_query = q;
        app.search_cursor = cur;
        if consumed {
            if changed {
                app.selected_row = 0;
            }
            return Ok(());
        }
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
                app.config.mcp_log_file.clone().unwrap_or_default(),
                app.config.audit_timeout.to_string(),
            ];
            app.settings_focused_field = 0;
            app.settings_cursor = crate::tui::input::end_of(&app.settings_inputs[0]);
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
                if app.skip_delete_confirm {
                    app.hosts.remove(idx);
                    app.save_hosts()?;
                    app.selected_row = app.selected_row.min(app.hosts.len().saturating_sub(1));
                    app.status_message = Some(crate::types::StatusMessage::info(format!(
                        "Host '{name}' deleted."
                    )));
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
        KeyCode::Char('p') | KeyCode::Char('P') if !app.search_focused && hosts_len > 0 => {
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
                    host_idx: idx,
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
            app.import_path_input.clear();
            app.import_cursor = 0;
            app.import_export_mode = 0;
            app.screen = Screen::ImportHosts;
        }
        _ => {}
    }
    Ok(())
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
    let cred =
        crate::ssh::resolve_credential(&app.credentials, &app.config, last_cred_id.as_deref())?
            .cloned();

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
        let mut term =
            ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))
                .unwrap();
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
