use crate::tui::{App, Screen, Term};
use crate::types::{Host, HostForm};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};
use uuid::Uuid;

fn form_area(area: Rect) -> Rect {
    crate::tui::adaptive_centered_rect(70, 90, 70, 16, area)
}

fn form_rows(area: Rect) -> [Rect; 4] {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let spacious = area.height >= 20;
    let rows = Layout::vertical([
        Constraint::Length(if spacious { 2 } else { 1 }),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .margin(1)
    .split(inner);
    [rows[0], rows[1], rows[2], rows[3]]
}

fn tab_at(area: Rect, column: u16, row: u16) -> Option<usize> {
    let tabs = form_rows(form_area(area))[0];
    if row != tabs.y || column < tabs.x || column >= tabs.right() {
        return None;
    }
    match column - tabs.x {
        0..=11 => Some(0),
        14..=34 => Some(1),
        _ => None,
    }
}

fn page_fields(form: &HostForm) -> &'static [usize] {
    if form.page == 0 {
        if form.jump_host_id.is_some() {
            &[0, 1, 2, 3, 4, 5, 6, 7, 12, 11]
        } else {
            &[0, 1, 2, 3, 4, 5, 6, 7, 11]
        }
    } else {
        &[8, 9, 10]
    }
}

fn switch_page(form: &mut HostForm, page: usize) {
    form.page = page;
    form.focused = if page == 0 { 0 } else { 8 };
    form.cursor = crate::tui::input::end_of(active_field(form));
}

pub fn handle_mouse(app: &mut App, mouse: MouseEvent, area: Rect) {
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) || app.delete_popup.is_some() {
        return;
    }
    let Some(form) = app.host_form.as_mut() else {
        return;
    };
    if form.jump_picker || form.credential_picker {
        return;
    }
    if let Some(page) = tab_at(area, mouse.column, mouse.row) {
        switch_page(form, page);
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let Some(form) = &app.host_form else { return };

    let area = form_area(f.area());
    f.render_widget(Clear, area);
    let title = if form.editing_id.is_some() {
        " Edit Host "
    } else {
        " Add Host "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    f.render_widget(block, area);
    let rows = form_rows(area);

    let tabs = Line::from(vec![
        Span::styled(
            " [F1] Host  ",
            if form.page == 0 {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray).bg(Color::Rgb(35, 39, 49))
            },
        ),
        Span::raw("  "),
        Span::styled(
            " [F2] Ports & Proxy  ",
            if form.page == 1 {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray).bg(Color::Rgb(35, 39, 49))
            },
        ),
    ]);
    f.render_widget(
        Paragraph::new(tabs),
        Rect {
            height: 1,
            ..rows[0]
        },
    );

    let fields: &[(usize, &str, &str)] = if form.page == 0 {
        &[
            (0, "Name *", &form.name),
            (1, "IP / Hostname *", &form.ip),
            (2, "Group", &form.group),
            (3, "Port", &form.port),
            (4, "User", &form.user),
            (5, "Tags", &form.tags),
            (6, "Description", &form.description),
        ]
    } else {
        &[
            (8, "Forwards", &form.port_forwards),
            (9, "SOCKS port", &form.socks_port),
            (10, "HTTP proxy port", &form.http_proxy_port),
        ]
    };
    let has_jump = form.jump_host_id.is_some();
    let field_count = if form.page == 0 {
        if has_jump {
            10
        } else {
            9
        }
    } else {
        3
    };
    let row_height = if rows[1].height >= field_count * 2 {
        2
    } else {
        1
    };
    let field_rows =
        Layout::vertical(vec![Constraint::Length(row_height); field_count as usize]).split(rows[1]);
    for (index, (field, label, value)) in fields.iter().enumerate() {
        let row = Rect {
            height: 1,
            ..field_rows[index]
        };
        let focused = form.focused == *field;
        let label_style = if focused {
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        f.render_widget(
            Paragraph::new(*label).style(label_style),
            Rect {
                width: row.width.min(18),
                ..row
            },
        );
        let input = Rect {
            x: row.x.saturating_add(18),
            width: row.width.saturating_sub(18),
            ..row
        };
        let input_style = Style::default()
            .fg(if focused { Color::White } else { Color::Gray })
            .bg(if focused {
                Color::Rgb(44, 59, 78)
            } else {
                Color::Rgb(35, 39, 49)
            });
        let mut spans = vec![Span::styled(" ", input_style)];
        if focused {
            spans.extend(crate::tui::input::spans(value, form.cursor, input_style));
        } else {
            spans.push(Span::styled((*value).to_string(), input_style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).style(input_style), input);
    }
    if form.page == 0 {
        let row = Rect {
            height: 1,
            ..field_rows[7]
        };
        let focused = form.focused == 7;
        let value = form
            .jump_host_id
            .as_deref()
            .and_then(|id| app.hosts.iter().find(|h| h.id == id))
            .map(|h| format!("{} ({})", h.name, h.ip))
            .unwrap_or_else(|| "(none)".into());
        f.render_widget(
            Paragraph::new("Jump host").style(Style::default().fg(if focused {
                Color::Blue
            } else {
                Color::DarkGray
            })),
            Rect {
                width: row.width.min(18),
                ..row
            },
        );
        f.render_widget(
            Paragraph::new(format!(" {value}  [Enter] select")).style(
                Style::default()
                    .fg(if focused { Color::White } else { Color::Gray })
                    .bg(if focused {
                        Color::Rgb(44, 59, 78)
                    } else {
                        Color::Rgb(35, 39, 49)
                    }),
            ),
            Rect {
                x: row.x.saturating_add(18),
                width: row.width.saturating_sub(18),
                ..row
            },
        );
        if has_jump {
            let row = Rect {
                height: 1,
                ..field_rows[8]
            };
            let focused = form.focused == 12;
            let jump_cred_value = if let Some(ref cid) = form.jump_credential_id {
                app.credentials
                    .iter()
                    .find(|c| c.id == *cid)
                    .map(|c| format!("{} ({})", c.name, c.username))
                    .unwrap_or_else(|| "(auto / ssh default)".into())
            } else {
                let jump_record = form
                    .jump_host_id
                    .as_deref()
                    .and_then(|jid| app.record_for(jid));
                [
                    jump_record.and_then(|r| r.default_credential_id.as_deref()),
                    jump_record.and_then(|r| r.last_credential_id.as_deref()),
                ]
                .into_iter()
                .flatten()
                .find_map(|cid| app.credentials.iter().find(|c| c.id == cid))
                .map(|c| format!("{} ({}) [jump host]", c.name, c.username))
                .unwrap_or_else(|| "(auto / ssh default)".into())
            };
            f.render_widget(
                Paragraph::new("Jump credential").style(Style::default().fg(if focused {
                    Color::Blue
                } else {
                    Color::DarkGray
                })),
                Rect {
                    width: row.width.min(18),
                    ..row
                },
            );
            f.render_widget(
                Paragraph::new(format!(" {jump_cred_value}  [Enter] select")).style(
                    Style::default()
                        .fg(if focused { Color::White } else { Color::Gray })
                        .bg(if focused {
                            Color::Rgb(44, 59, 78)
                        } else {
                            Color::Rgb(35, 39, 49)
                        }),
                ),
                Rect {
                    x: row.x.saturating_add(18),
                    width: row.width.saturating_sub(18),
                    ..row
                },
            );
        }
        let cred_row_idx = if has_jump { 9 } else { 8 };
        let row = Rect {
            height: 1,
            ..field_rows[cred_row_idx]
        };
        let focused = form.focused == 11;
        let record = form.editing_id.as_deref().and_then(|id| app.record_for(id));
        let pinned = if form.credential_changed {
            form.credential_id.as_deref()
        } else {
            form.credential_id
                .as_deref()
                .or_else(|| record.and_then(|r| r.default_credential_id.as_deref()))
        };
        let value = crate::ssh::resolve_credential(
            &app.credentials,
            &app.config,
            record.and_then(|r| r.last_credential_id.as_deref()),
            pinned,
        )
        .ok()
        .flatten()
        .map(|c| format!("{} ({})", c.name, c.username))
        .unwrap_or_else(|| "(none)".into());
        f.render_widget(
            Paragraph::new("Credential").style(Style::default().fg(if focused {
                Color::Blue
            } else {
                Color::DarkGray
            })),
            Rect {
                width: row.width.min(18),
                ..row
            },
        );
        f.render_widget(
            Paragraph::new(format!(" {value}  [Enter] select")).style(
                Style::default()
                    .fg(if focused { Color::White } else { Color::Gray })
                    .bg(if focused {
                        Color::Rgb(44, 59, 78)
                    } else {
                        Color::Rgb(35, 39, 49)
                    }),
            ),
            Rect {
                x: row.x.saturating_add(18),
                width: row.width.saturating_sub(18),
                ..row
            },
        );
    }
    if let Some(err) = &form.error_message {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(Color::Red),
            ))),
            rows[2],
        );
    } else if form.page == 1 {
        f.render_widget(
            Paragraph::new("Forwards: local:remote, comma-separated; HTTP supports HTTPS CONNECT"),
            rows[2],
        );
    }
    f.render_widget(
        Paragraph::new("[↑/↓/Tab] field  [F1/F2] tab  [Enter] save  [Esc] cancel"),
        rows[3],
    );

    if form.jump_picker {
        draw_jump_picker(f, app, form);
    } else if form.credential_picker {
        draw_credential_picker(f, app, form);
    }
}

pub fn draw_import(f: &mut Frame, app: &App) {
    let area = crate::tui::adaptive_centered_rect(60, 50, 60, 16, f.area());
    f.render_widget(Clear, area);

    let mode = app.import_export_mode;
    let title = match mode {
        0 => " Import Ansible INI ",
        1 => " Export Ansible INI ",
        _ => " Export SSH config ",
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // mode toggle
            Constraint::Length(1), // spacer
            Constraint::Length(1), // label
            Constraint::Length(1), // spacer
            Constraint::Length(1), // input
            Constraint::Min(0),
            Constraint::Length(1), // hotkeys
        ])
        .margin(1)
        .split(inner);

    // Mode toggle
    let style_for = |m: usize| {
        if m == mode {
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Import INI]", style_for(0)),
            Span::raw("  "),
            Span::styled("[Export INI]", style_for(1)),
            Span::raw("  "),
            Span::styled("[Export SSH config]", style_for(2)),
            Span::styled("   Tab to switch", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let label = if mode == 0 {
        "Path to inventory file:"
    } else {
        "Save to path:"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            label,
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(crate::tui::input::spans(
            &app.import_path_input,
            app.import_cursor,
            Style::default().fg(Color::White),
        ))),
        chunks[4],
    );

    let action = if mode == 0 { "import" } else { "export" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Blue)),
            Span::styled(format!(" {action}  "), Style::default().fg(Color::DarkGray)),
            Span::styled("[Tab]", Style::default().fg(Color::Blue)),
            Span::styled(" switch  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc]", Style::default().fg(Color::Blue)),
            Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[6],
    );
}

pub fn cancel(app: &mut App) {
    app.host_form = None;
    app.screen = if app.host_form_from_audit {
        Screen::Audit
    } else {
        Screen::Main
    };
    app.host_form_from_audit = false;
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    handle_form_key(app, key)
}

fn handle_form_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(form) = app.host_form.as_ref() else {
        return Ok(());
    };
    if form.jump_picker {
        handle_jump_picker(app, key);
        return Ok(());
    }
    if form.credential_picker {
        handle_credential_picker(app, key);
        return Ok(());
    }

    if !matches!(form.focused, 7 | 11 | 12)
        && !matches!(
            key.code,
            KeyCode::Esc
                | KeyCode::Tab
                | KeyCode::BackTab
                | KeyCode::Enter
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::F(1)
                | KeyCode::F(2)
        )
    {
        let form = app.host_form.as_mut().unwrap();
        let mut cursor = form.cursor;
        let mut buf = std::mem::take(active_field(form));
        let consumed = crate::tui::input::handle(&mut buf, &mut cursor, key);
        *active_field(form) = buf;
        form.cursor = cursor;
        if consumed {
            return Ok(());
        }
    }

    let form = app.host_form.as_mut().unwrap();
    match key.code {
        KeyCode::Esc => cancel(app),
        KeyCode::F(1) | KeyCode::F(2) => {
            switch_page(form, if key.code == KeyCode::F(1) { 0 } else { 1 });
        }
        KeyCode::Tab | KeyCode::Down | KeyCode::BackTab | KeyCode::Up => {
            let order = page_fields(form);
            let count = order.len();
            let pos = order.iter().position(|&f| f == form.focused).unwrap_or(0);
            let next = if matches!(key.code, KeyCode::Tab | KeyCode::Down) {
                (pos + 1) % count
            } else {
                (pos + count - 1) % count
            };
            form.focused = order[next];
            form.cursor = crate::tui::input::end_of(active_field(form));
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter
            if form.focused == 11 =>
        {
            form.credential_picker = true;
            form.credential_picker_for_jump = false;
            form.credential_query.clear();
            form.credential_cursor = 0;
            form.credential_selected = 0;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter
            if form.focused == 12 =>
        {
            form.credential_picker = true;
            form.credential_picker_for_jump = true;
            form.credential_query.clear();
            form.credential_cursor = 0;
            form.credential_selected = 0;
        }
        KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Enter
            if form.focused == 7 =>
        {
            form.jump_picker = true;
            form.jump_query.clear();
            form.jump_cursor = 0;
            form.jump_selected = 0;
        }
        KeyCode::Backspace | KeyCode::Delete if form.focused == 7 => {
            form.jump_host_id = None;
            form.jump_credential_id = None;
        }
        KeyCode::Backspace | KeyCode::Delete if form.focused == 12 => {
            form.jump_credential_id = None;
        }
        KeyCode::Backspace | KeyCode::Delete if form.focused == 11 => {
            form.credential_id = None;
            form.credential_changed = true;
        }
        KeyCode::Enter => save_host(app)?,
        _ => {}
    }
    Ok(())
}

fn credential_matches<'a>(app: &'a App, form: &HostForm) -> Vec<&'a crate::types::Credential> {
    let query = form.credential_query.to_lowercase();
    app.credentials
        .iter()
        .filter(|c| {
            c.name.to_lowercase().contains(&query) || c.username.to_lowercase().contains(&query)
        })
        .collect()
}

fn draw_credential_picker(f: &mut Frame, app: &App, form: &HostForm) {
    let area = crate::tui::adaptive_centered_rect(60, 60, 50, 10, f.area());
    f.render_widget(Clear, area);
    let title = if form.credential_picker_for_jump {
        " Select jump credential "
    } else {
        " Select credential "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    let mut search = vec![Span::raw("Search: ")];
    search.extend(crate::tui::input::spans(
        &form.credential_query,
        form.credential_cursor,
        Style::default().fg(Color::White),
    ));
    f.render_widget(Paragraph::new(Line::from(search)), rows[0]);
    let matches = credential_matches(app, form);
    let mut items: Vec<String> = Vec::with_capacity(matches.len() + 1);
    if form.credential_picker_for_jump {
        items.push("(auto / from jump host)".to_string());
    }
    items.extend(
        matches
            .iter()
            .map(|cred| format!("{} ({})", cred.name, cred.username)),
    );
    let selected = form.credential_selected.min(items.len().saturating_sub(1));
    let visible = rows[1].height as usize;
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for (offset, (index, label)) in items
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .enumerate()
    {
        let style = if index == selected {
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{} {label}", if index == selected { "▶" } else { " " }),
                style,
            )),
            Rect {
                y: rows[1].y + offset as u16,
                height: 1,
                ..rows[1]
            },
        );
    }
    f.render_widget(
        Paragraph::new("[↑/↓] select  [Enter] apply  [Esc] cancel"),
        rows[2],
    );
}

fn handle_credential_picker(app: &mut App, key: KeyEvent) {
    let for_jump = app.host_form.as_ref().unwrap().credential_picker_for_jump;
    let matches = credential_matches(app, app.host_form.as_ref().unwrap());
    let count = if for_jump {
        matches.len() + 1
    } else {
        matches.len()
    };
    let selected_idx = app
        .host_form
        .as_ref()
        .unwrap()
        .credential_selected
        .min(count.saturating_sub(1));
    let selected_id = if key.code == KeyCode::Enter {
        if for_jump {
            if selected_idx == 0 {
                None
            } else {
                matches.get(selected_idx - 1).map(|c| c.id.clone())
            }
        } else {
            matches.get(selected_idx).map(|c| c.id.clone())
        }
    } else {
        None
    };
    let form = app.host_form.as_mut().unwrap();
    match key.code {
        KeyCode::Esc => {
            form.credential_picker = false;
            form.credential_picker_for_jump = false;
        }
        KeyCode::Up => form.credential_selected = form.credential_selected.saturating_sub(1),
        KeyCode::Down => {
            form.credential_selected = (form.credential_selected + 1).min(count.saturating_sub(1))
        }
        KeyCode::Enter => {
            if for_jump {
                form.jump_credential_id = selected_id;
                form.credential_picker = false;
                form.credential_picker_for_jump = false;
            } else if let Some(id) = selected_id {
                form.credential_id = Some(id);
                form.credential_changed = true;
                form.credential_picker = false;
            }
        }
        _ => {
            let mut query = std::mem::take(&mut form.credential_query);
            let consumed = crate::tui::input::handle(&mut query, &mut form.credential_cursor, key);
            form.credential_query = query;
            if consumed {
                form.credential_selected = 0;
            }
        }
    }
}

fn jump_matches<'a>(app: &'a App, form: &HostForm) -> Vec<&'a Host> {
    let query = form.jump_query.to_lowercase();
    app.hosts
        .iter()
        .filter(|h| {
            Some(&h.id) != form.editing_id.as_ref()
                && (h.name.to_lowercase().contains(&query) || h.ip.to_lowercase().contains(&query))
        })
        .collect()
}

fn draw_jump_picker(f: &mut Frame, app: &App, form: &HostForm) {
    let area = crate::tui::adaptive_centered_rect(60, 60, 50, 10, f.area());
    f.render_widget(Clear, area);
    let block = Block::default()
        .title(" Select jump host ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    let mut search = vec![Span::raw("Search: ")];
    search.extend(crate::tui::input::spans(
        &form.jump_query,
        form.jump_cursor,
        Style::default().fg(Color::White),
    ));
    f.render_widget(Paragraph::new(Line::from(search)), rows[0]);
    let matches = jump_matches(app, form);
    let selected = form.jump_selected.min(matches.len());
    let visible = rows[1].height as usize;
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for index in start..(start + visible).min(matches.len() + 1) {
        let text = if index == 0 {
            "(none)".to_string()
        } else {
            format!("{} ({})", matches[index - 1].name, matches[index - 1].ip)
        };
        let style = if index == selected {
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        f.render_widget(
            Paragraph::new(Span::styled(
                format!("{} {text}", if index == selected { "▶" } else { " " }),
                style,
            )),
            ratatui::layout::Rect {
                y: rows[1].y + (index - start) as u16,
                height: 1,
                ..rows[1]
            },
        );
    }
    f.render_widget(
        Paragraph::new("[↑/↓] select  [Enter] apply  [Esc] cancel"),
        rows[2],
    );
}

fn handle_jump_picker(app: &mut App, key: KeyEvent) {
    let count = jump_matches(app, app.host_form.as_ref().unwrap()).len() + 1;
    let selected_id = if key.code == KeyCode::Enter {
        let form = app.host_form.as_ref().unwrap();
        let selected = form.jump_selected;
        if selected == 0 {
            None
        } else {
            jump_matches(app, form)
                .get(selected - 1)
                .map(|h| h.id.clone())
        }
    } else {
        None
    };
    let form = app.host_form.as_mut().unwrap();
    match key.code {
        KeyCode::Esc => form.jump_picker = false,
        KeyCode::Up => form.jump_selected = form.jump_selected.saturating_sub(1),
        KeyCode::Down => form.jump_selected = (form.jump_selected + 1).min(count - 1),
        KeyCode::Enter => {
            if selected_id.is_none() {
                form.jump_credential_id = None;
            }
            form.jump_host_id = selected_id;
            form.jump_picker = false;
        }
        _ => {
            let mut query = std::mem::take(&mut form.jump_query);
            let consumed = crate::tui::input::handle(&mut query, &mut form.jump_cursor, key);
            form.jump_query = query;
            if consumed {
                form.jump_selected = 0;
            }
        }
    }
}

fn active_field(form: &mut HostForm) -> &mut String {
    match form.focused {
        0 => &mut form.name,
        1 => &mut form.ip,
        2 => &mut form.group,
        3 => &mut form.port,
        4 => &mut form.user,
        5 => &mut form.tags,
        8 => &mut form.port_forwards,
        9 => &mut form.socks_port,
        10 => &mut form.http_proxy_port,
        _ => &mut form.description,
    }
}

fn save_host(app: &mut App) -> Result<()> {
    let form = match &app.host_form {
        Some(f) => f.clone(),
        None => return Ok(()),
    };

    let set_error = |app: &mut App, err: String, focus: usize| {
        if let Some(ref mut f) = app.host_form {
            f.error_message = Some(err.clone());
            f.focused = focus;
            f.page = if matches!(focus, 8..=10) { 1 } else { 0 };
            f.cursor = crate::tui::input::end_of(active_field(f));
        }
        app.status_message = Some(crate::types::StatusMessage::error(err));
    };

    if form.name.trim().is_empty() {
        set_error(app, "Name is required.".into(), 0);
        return Ok(());
    }

    // Host name uniqueness validation
    let trimmed_name = form.name.trim();
    let name_conflict = app
        .hosts
        .iter()
        .any(|h| h.name == trimmed_name && Some(&h.id) != form.editing_id.as_ref());
    if name_conflict {
        set_error(
            app,
            format!("Host name '{trimmed_name}' already exists."),
            0,
        );
        return Ok(());
    }

    // Reserved group name validation
    let trimmed_group = form.group.trim().to_lowercase();
    if trimmed_group == "all" || trimmed_group == "_meta" {
        set_error(
            app,
            format!(
                "Group name '{}' is reserved by Ansible inventory and cannot be used.",
                form.group.trim()
            ),
            2,
        );
        return Ok(());
    }
    if form.ip.trim().is_empty() {
        set_error(app, "IP / Hostname is required.".into(), 1);
        return Ok(());
    }
    if let Err(e) = crate::ssh::validate_host(form.ip.trim()) {
        set_error(app, format!("Invalid IP / Hostname: {e}"), 1);
        return Ok(());
    }
    if !form.user.trim().is_empty() {
        if let Err(e) = crate::ssh::validate_username(form.user.trim()) {
            set_error(app, format!("Invalid Username: {e}"), 4);
            return Ok(());
        }
    }
    let port: u16 = match form.port.trim().parse() {
        Ok(p) if p > 0 => p,
        _ => {
            set_error(app, "Port must be a number between 1 and 65535.".into(), 3);
            return Ok(());
        }
    };
    let port_forwards: Vec<String> = form
        .port_forwards
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let socks_port = if form.socks_port.trim().is_empty() {
        None
    } else {
        match form.socks_port.trim().parse::<u16>() {
            Ok(port) => Some(port),
            Err(_) => {
                set_error(app, "SOCKS port must be between 1 and 65535.".into(), 9);
                return Ok(());
            }
        }
    };
    if let Err(e) = crate::ssh::port_forward_args(&port_forwards, socks_port) {
        set_error(app, format!("Invalid forwarding: {e}"), 8);
        return Ok(());
    }
    let http_proxy_port = if form.http_proxy_port.trim().is_empty() {
        None
    } else {
        match form.http_proxy_port.trim().parse::<u16>() {
            Ok(port) if port > 0 => Some(port),
            _ => {
                set_error(
                    app,
                    "HTTP proxy port must be between 1 and 65535.".into(),
                    10,
                );
                return Ok(());
            }
        }
    };
    if let Some(port) = http_proxy_port {
        if socks_port.is_none()
            || socks_port == Some(port)
            || port_forwards
                .iter()
                .any(|f| f.split_once(':').and_then(|(p, _)| p.parse::<u16>().ok()) == Some(port))
        {
            set_error(
                app,
                "HTTP proxy requires a different SOCKS port and no conflicting forward.".into(),
                10,
            );
            return Ok(());
        }
    }
    let tags: Vec<String> = form
        .tags
        .split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let jump_credential_id = if form.jump_host_id.is_some() {
        form.jump_credential_id.clone()
    } else {
        None
    };

    if let Some(ref edit_id) = form.editing_id {
        // Update existing host
        if let Some(h) = app.hosts.iter_mut().find(|h| &h.id == edit_id) {
            h.name = form.name.trim().to_string();
            h.ip = form.ip.trim().to_string();
            h.group = form.group.trim().to_string();
            h.port = port;
            h.user = if form.user.trim().is_empty() {
                None
            } else {
                Some(form.user.trim().to_string())
            };
            h.tags = tags;
            h.description = if form.description.trim().is_empty() {
                None
            } else {
                Some(form.description.trim().to_string())
            };
            h.jump_host_id = form.jump_host_id.clone();
            h.jump_credential_id = jump_credential_id;
            h.port_forwards = port_forwards.clone();
            h.socks_port = socks_port;
            h.http_proxy_port = http_proxy_port;
        }
        app.status_message = Some(crate::types::StatusMessage::success(format!(
            "Host '{}' updated.",
            form.name.trim()
        )));
    } else {
        // Add new host
        app.hosts.push(Host {
            id: Uuid::new_v4().to_string(),
            name: form.name.trim().to_string(),
            ip: form.ip.trim().to_string(),
            group: form.group.trim().to_string(),
            port,
            user: if form.user.trim().is_empty() {
                None
            } else {
                Some(form.user.trim().to_string())
            },
            tags,
            description: if form.description.trim().is_empty() {
                None
            } else {
                Some(form.description.trim().to_string())
            },
            jump_host_id: form.jump_host_id.clone(),
            jump_credential_id,
            port_forwards,
            socks_port,
            http_proxy_port,
        });
        app.status_message = Some(crate::types::StatusMessage::success(format!(
            "Host '{}' added.",
            form.name.trim()
        )));
    }

    app.save_hosts()?;
    if form.credential_changed {
        let host_id = form
            .editing_id
            .as_deref()
            .unwrap_or(&app.hosts.last().unwrap().id)
            .to_string();
        if let Some(ref credential_id) = form.credential_id {
            app.select_credential(&host_id, credential_id, true)?;
        } else {
            app.clear_pinned_credential(&host_id)?;
        }
    }
    if app.host_form_from_audit {
        if let Some(ref edit_id) = form.editing_id {
            if let Some(ref mut state) = app.audit_state {
                if let Some(idx) = state.results.iter().position(|r| &r.host_id == edit_id) {
                    if state.is_running {
                        app.start_audit();
                        if let Some(ref mut state) = app.audit_state {
                            state.selected = idx;
                        }
                    } else {
                        if let Some(host) = app.hosts.iter().find(|h| &h.id == edit_id) {
                            let result = &mut state.results[idx];
                            result.host_name = host.name.clone();
                            result.ip = host.ip.clone();
                            result.port = host.port;
                            result.group = host.group.clone();
                            result.latency_ms = None;
                            result.credential_name = None;
                        }
                        state.start_single(
                            idx,
                            app.hosts.clone(),
                            app.credentials.clone(),
                            app.server_records.clone(),
                            app.config.clone(),
                        );
                    }
                }
            }
        }
    }
    app.host_form = None;
    app.screen = if app.host_form_from_audit {
        Screen::Audit
    } else {
        Screen::Main
    };
    app.host_form_from_audit = false;
    Ok(())
}

pub fn handle_import_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if !matches!(key.code, KeyCode::Esc | KeyCode::Tab | KeyCode::Enter) {
        let (mut buf, mut cursor) = (
            std::mem::take(&mut app.import_path_input),
            app.import_cursor,
        );
        let consumed = crate::tui::input::handle(&mut buf, &mut cursor, key);
        app.import_path_input = buf;
        app.import_cursor = cursor;
        if consumed {
            return Ok(());
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            app.import_export_mode = (app.import_export_mode + 1) % 3;
        }
        KeyCode::Enter => {
            let raw = app.import_path_input.trim().to_string();
            if raw.is_empty() {
                app.screen = Screen::Main;
                return Ok(());
            }
            let path = crate::config::expand_tilde(&raw);
            if app.import_export_mode > 0 {
                let content = if app.import_export_mode == 1 {
                    crate::inventory::export_to_ini(&app.hosts)
                } else {
                    crate::inventory::export_to_ssh_config(&app.hosts)
                };
                match crate::config::write_secure_atomic_file(std::path::Path::new(&path), &content)
                {
                    Ok(()) => {
                        app.status_message = Some(crate::types::StatusMessage::success(format!(
                            "Exported {} hosts to {path}.",
                            app.hosts.len()
                        )));
                        app.screen = Screen::Main;
                    }
                    Err(e) => {
                        app.status_message = Some(crate::types::StatusMessage::error(format!(
                            "Export error: {e}"
                        )));
                    }
                }
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let count = crate::inventory::import_from_ini(&content, &mut app.hosts);
                        let updated = app.hosts.len();
                        app.save_hosts()?;
                        app.status_message = Some(crate::types::StatusMessage::success(format!(
                            "Import complete: {count} new hosts added ({updated} total)."
                        )));
                        app.screen = Screen::Main;
                    }
                    Err(e) => {
                        app.status_message = Some(crate::types::StatusMessage::error(format!(
                            "Error reading file: {e}"
                        )));
                    }
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

    fn app() -> App {
        let mut app = App::new(vec![], vec![], Default::default(), vec![]);
        app.host_form = Some(HostForm {
            name: "example".into(),
            ip: "example.org".into(),
            port: "22".into(),
            ..Default::default()
        });
        app
    }

    fn key(app: &mut App, code: KeyCode) {
        handle_form_key(app, KeyEvent::from(code)).unwrap();
    }

    #[test]
    fn tabs_navigation_and_picker_search() {
        let mut app = app();
        app.hosts.push(Host {
            id: "jump".into(),
            name: "gateway".into(),
            ip: "10.0.0.1".into(),
            group: String::new(),
            port: 22,
            user: None,
            tags: vec![],
            description: None,
            jump_host_id: None,
            jump_credential_id: None,
            port_forwards: vec![],
            socks_port: None,
            http_proxy_port: None,
        });
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 11);
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 7);
        key(&mut app, KeyCode::Enter);
        for c in "gate".chars() {
            key(&mut app, KeyCode::Char(c));
        }
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.host_form.as_ref().unwrap().jump_host_id.as_deref(),
            Some("jump")
        );
        key(&mut app, KeyCode::F(2));
        assert_eq!(app.host_form.as_ref().unwrap().focused, 8);
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 10);
    }

    #[test]
    fn small_terminal_shows_each_page_and_error() {
        let mut app = app();
        for page in 0..=1 {
            let form = app.host_form.as_mut().unwrap();
            form.page = page;
            form.focused = if page == 0 { 0 } else { 8 };
            form.error_message = Some("validation error".into());
            let mut terminal = Terminal::new(TestBackend::new(70, 16)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains(if page == 0 {
                "Credential"
            } else {
                "HTTP proxy port"
            }));
            assert!(text.contains("validation error"));
            assert!(text.contains("[Enter] save"));
        }
    }

    #[test]
    fn clicking_tabs_only_inside_form_switches_pages() {
        let mut app = app();
        let area = Rect::new(0, 0, 100, 30);
        let tabs = form_rows(form_area(area))[0];
        let click = |app: &mut App, x, y| {
            handle_mouse(
                app,
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: x,
                    row: y,
                    modifiers: crossterm::event::KeyModifiers::NONE,
                },
                area,
            );
        };
        click(&mut app, tabs.x + 14, tabs.y);
        assert_eq!(app.host_form.as_ref().unwrap().page, 1);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 8);
        click(&mut app, tabs.x + 12, tabs.y);
        click(&mut app, tabs.x + 14, tabs.y + 1);
        assert_eq!(app.host_form.as_ref().unwrap().page, 1);
        app.host_form.as_mut().unwrap().jump_picker = true;
        click(&mut app, tabs.x, tabs.y);
        assert_eq!(app.host_form.as_ref().unwrap().page, 1);
        app.host_form.as_mut().unwrap().jump_picker = false;
        click(&mut app, tabs.x, tabs.y);
        assert_eq!(app.host_form.as_ref().unwrap().page, 0);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 0);
        app.host_form.as_mut().unwrap().credential_picker = true;
        click(&mut app, tabs.x + 14, tabs.y);
        assert_eq!(app.host_form.as_ref().unwrap().page, 0);
    }

    #[test]
    fn credential_picker_search_and_cancel() {
        let mut app = app();
        for (id, name) in [("first", "alpha"), ("second", "beta")] {
            app.credentials.push(crate::types::Credential {
                id: id.into(),
                name: name.into(),
                username: "operator".into(),
                kind: crate::types::CredentialKind::Password,
                key_path: None,
            });
        }
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 11);
        key(&mut app, KeyCode::Char('x'));
        assert_eq!(app.host_form.as_ref().unwrap().description, "");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('b'));
        key(&mut app, KeyCode::Enter);
        let form = app.host_form.as_ref().unwrap();
        assert_eq!(form.credential_id.as_deref(), Some("second"));
        assert!(form.credential_changed);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('a'));
        key(&mut app, KeyCode::Esc);
        assert_eq!(
            app.host_form.as_ref().unwrap().credential_id.as_deref(),
            Some("second")
        );
        key(&mut app, KeyCode::Esc);
        assert!(app.host_form.is_none());
        assert!(app.server_records.is_empty());
    }

    #[test]
    fn credential_picker_moves_through_matches() {
        let mut app = app();
        for name in ["alpha", "beta"] {
            app.credentials.push(crate::types::Credential {
                id: name.into(),
                name: name.into(),
                username: "operator".into(),
                kind: crate::types::CredentialKind::Password,
                key_path: None,
            });
        }
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.host_form.as_ref().unwrap().credential_id.as_deref(),
            Some("beta")
        );
    }

    #[test]
    fn credential_field_shows_effective_and_new_selection() {
        let mut app = app();
        for name in ["alpha", "beta"] {
            app.credentials.push(crate::types::Credential {
                id: name.into(),
                name: name.into(),
                username: "operator".into(),
                kind: crate::types::CredentialKind::Password,
                key_path: None,
            });
        }
        app.config.default_credential_id = Some("alpha".into());
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("alpha (operator)"));
        app.host_form.as_mut().unwrap().credential_id = Some("beta".into());
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("beta (operator)"));
    }

    #[test]
    fn field_background_and_spacing_adapt_to_height() {
        let app = app();
        for (height, expected_step) in [(16, 1), (30, 2)] {
            let area = Rect::new(0, 0, 100, height);
            let mut terminal = Terminal::new(TestBackend::new(area.width, area.height)).unwrap();
            terminal.draw(|f| draw(f, &app)).unwrap();
            let field_area = form_rows(form_area(area))[1];
            let first = terminal
                .backend()
                .buffer()
                .cell((field_area.x + 19, field_area.y))
                .unwrap();
            assert_ne!(first.bg, Color::Reset);
            let next = field_area.y + expected_step;
            assert_eq!(
                terminal
                    .backend()
                    .buffer()
                    .cell((field_area.x, next))
                    .unwrap()
                    .symbol(),
                "I"
            );
        }
    }

    #[test]
    fn jump_credential_field_appears_selects_and_clears() {
        let mut app = app();
        app.hosts.push(Host {
            id: "bastion".into(),
            name: "bastion".into(),
            ip: "10.0.0.1".into(),
            group: String::new(),
            port: 22,
            user: None,
            tags: vec![],
            description: None,
            jump_host_id: None,
            jump_credential_id: None,
            port_forwards: vec![],
            socks_port: None,
            http_proxy_port: None,
        });
        for (id, name, user) in [
            ("jump-c", "bastion-key", "ops"),
            ("target-c", "app-pass", "deploy"),
        ] {
            app.credentials.push(crate::types::Credential {
                id: id.into(),
                name: name.into(),
                username: user.into(),
                kind: crate::types::CredentialKind::Password,
                key_path: None,
            });
        }

        // Select bastion as jump host (Up -> 11, Up -> 7, Enter -> Down -> Enter)
        key(&mut app, KeyCode::Up);
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 7);
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.host_form.as_ref().unwrap().jump_host_id.as_deref(),
            Some("bastion")
        );

        // Down from field 7 now focuses field 12 (Jump credential)
        key(&mut app, KeyCode::Down);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 12);

        // Open jump credential picker and pick bastion-key (row 1 after "(auto / from jump host)")
        key(&mut app, KeyCode::Enter);
        assert!(app.host_form.as_ref().unwrap().credential_picker_for_jump);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.host_form
                .as_ref()
                .unwrap()
                .jump_credential_id
                .as_deref(),
            Some("jump-c")
        );
        assert_eq!(app.host_form.as_ref().unwrap().credential_id, None);

        // Down moves to target Credential (field 11) and picks app-pass independently
        key(&mut app, KeyCode::Down);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 11);
        key(&mut app, KeyCode::Enter);
        assert!(!app.host_form.as_ref().unwrap().credential_picker_for_jump);
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Enter);
        assert_eq!(
            app.host_form.as_ref().unwrap().credential_id.as_deref(),
            Some("target-c")
        );
        assert_eq!(
            app.host_form
                .as_ref()
                .unwrap()
                .jump_credential_id
                .as_deref(),
            Some("jump-c")
        );

        // Rendered form displays Jump credential and selected credential
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Jump credential"));
        assert!(text.contains("bastion-key (ops)"));
        assert!(text.contains("app-pass (deploy)"));

        // Clearing jump host clears jump_credential_id without affecting target credential_id
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 12);
        key(&mut app, KeyCode::Up);
        assert_eq!(app.host_form.as_ref().unwrap().focused, 7);
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.host_form.as_ref().unwrap().jump_host_id, None);
        assert_eq!(app.host_form.as_ref().unwrap().jump_credential_id, None);
        assert_eq!(
            app.host_form.as_ref().unwrap().credential_id.as_deref(),
            Some("target-c")
        );
    }
}
