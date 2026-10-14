use crate::tui::{theme, App, Screen, Term};
use crate::types::{Host, HostForm};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use uuid::Uuid;

pub fn draw(f: &mut Frame, app: &App) {
    let Some(form) = &app.host_form else { return };

    let area = crate::tui::adaptive_centered_rect(
        60,
        80,
        70,
        22,
        crate::tui::help::content_area(app, f.area()),
    );
    f.render_widget(Clear, area);

    let title = if form.editing_id.is_some() {
        " Edit Host "
    } else {
        " Add Host "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::FOCUS));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(if form.focused == 4 { 2 } else { 0 }), // User hint
            Constraint::Length(1),                                     // validation error
            Constraint::Length(1),                                     // hotkeys
        ])
        .margin(1)
        .split(inner);
    let fields_area = chunks[0];
    let (first_field, visible_fields, row_height) =
        field_viewport(fields_area.height, form.focused);
    let field_area = |index: usize| -> Option<Rect> {
        if !(first_field..first_field + visible_fields).contains(&index) {
            return None;
        }
        Some(Rect::new(
            fields_area.x,
            fields_area.y + ((index - first_field) as u16 * row_height),
            fields_area.width,
            1,
        ))
    };
    let label_width = 18u16.min(fields_area.width.saturating_sub(1));
    let input_width = fields_area.width.saturating_sub(label_width) as usize;

    let fields = [
        ("Name *", &form.name),
        ("IP / Hostname *", &form.ip),
        ("Group", &form.group),
        ("Port", &form.port),
        ("User", &form.user),
        ("Tags", &form.tags),
        ("Description", &form.description),
    ];

    for (i, (label, value)) in fields.iter().enumerate() {
        let Some(row) = field_area(i) else { continue };
        let focused = form.focused == i;
        let label_style = if focused {
            Style::default()
                .fg(theme::FOCUS)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        };
        let input_style = if focused {
            Style::default().fg(theme::TEXT)
        } else {
            Style::default().fg(theme::MUTED)
        };
        let suffix = match i {
            3 if value.is_empty() => " (default 22)",
            5 if value.is_empty() => " (comma-separated)",
            _ => "",
        };
        f.render_widget(
            Paragraph::new(Span::styled(*label, label_style)),
            Rect::new(row.x, row.y, label_width, 1),
        );
        let mut spans = if focused {
            crate::tui::input::spans_with_width(value, form.cursor, input_style, input_width)
        } else {
            vec![Span::styled(value.to_string(), input_style)]
        };
        spans.push(Span::styled(suffix, Style::default().fg(theme::MUTED)));
        f.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(row.x + label_width, row.y, input_width as u16, 1),
        );
    }

    // Jump host selector (field 7): cycled with ←/→ among existing hosts
    if let Some(row) = field_area(7) {
        let focused = form.focused == 7;
        let label_style = if focused {
            Style::default()
                .fg(theme::FOCUS)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        };
        let value = form
            .jump_host_id
            .as_deref()
            .and_then(|id| app.hosts.iter().find(|h| h.id == id))
            .map(|h| format!("{} ({})", h.name, h.ip))
            .unwrap_or_else(|| "(none)".into());
        let input_style = if focused {
            Style::default().fg(theme::TEXT)
        } else {
            Style::default().fg(theme::MUTED)
        };
        let display = if focused {
            format!("◂ {value} ▸")
        } else {
            value
        };
        f.render_widget(
            Paragraph::new(Span::styled("Jump host", label_style)),
            Rect::new(row.x, row.y, label_width, 1),
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(display, input_style),
                Span::styled(
                    if focused { " (←/→ to select)" } else { "" },
                    Style::default().fg(theme::MUTED),
                ),
            ])),
            Rect::new(row.x + label_width, row.y, input_width as u16, 1),
        );
    }

    if let Some(row) = field_area(8) {
        let focused = form.focused == 8;
        let label_style = if focused {
            Style::default()
                .fg(theme::FOCUS)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        };
        let (value, missing) = match form.credential_id.as_deref() {
            None => ("Automatic".into(), false),
            Some(id) => match app.credentials.iter().find(|c| c.id == id) {
                Some(credential) => (
                    format!("{} ({})", credential.name, credential.username),
                    false,
                ),
                None => (format!("Missing credential: {id}"), true),
            },
        };
        let input_style = Style::default().fg(if missing {
            theme::ERROR
        } else if focused {
            theme::TEXT
        } else {
            theme::MUTED
        });
        let display = if focused {
            format!("◂ {value} ▸")
        } else {
            value
        };
        f.render_widget(
            Paragraph::new(Span::styled("Credentials", label_style)),
            Rect::new(row.x, row.y, label_width, 1),
        );
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(display, input_style),
                Span::styled(
                    if focused {
                        " (←/→ select, Backspace reset)"
                    } else {
                        ""
                    },
                    Style::default().fg(theme::MUTED),
                ),
            ])),
            Rect::new(row.x + label_width, row.y, input_width as u16, 1),
        );
    }

    if form.focused == 4 {
        f.render_widget(
            Paragraph::new("User: Copy ID / jump / export.\nNormal SSH: credential username.")
                .style(Style::default().fg(theme::MUTED))
                .wrap(Wrap { trim: false }),
            chunks[1],
        );
    }

    // Inline validation / save error message
    if let Some(ref err) = form.error_message {
        let err_line = Line::from(vec![
            Span::styled(
                "✗ ",
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(err, Style::default().fg(theme::ERROR)),
        ]);
        f.render_widget(Paragraph::new(err_line), chunks[2]);
    }

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(theme::FOCUS)),
        Span::styled(" next  ", Style::default().fg(theme::MUTED)),
        Span::styled("[Enter]", Style::default().fg(theme::FOCUS)),
        Span::styled(" save  ", Style::default().fg(theme::MUTED)),
        Span::styled("[Esc]", Style::default().fg(theme::FOCUS)),
        Span::styled(" cancel", Style::default().fg(theme::MUTED)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[3]);
}

fn field_viewport(height: u16, focused: usize) -> (usize, usize, u16) {
    let row_height = if height >= 18 { 2 } else { 1 };
    let visible = (height / row_height).min(9) as usize;
    let first = focused.min(8).saturating_sub(visible.saturating_sub(1));
    (first, visible, row_height)
}

pub fn draw_import(f: &mut Frame, app: &App) {
    let area = crate::tui::adaptive_centered_rect(
        60,
        50,
        60,
        16,
        crate::tui::help::content_area(app, f.area()),
    );
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
        .border_style(Style::default().fg(theme::FOCUS));
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
                .fg(theme::FOCUS)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Import INI]", style_for(0)),
            Span::raw("  "),
            Span::styled("[Export INI]", style_for(1)),
            Span::raw("  "),
            Span::styled("[Export SSH config]", style_for(2)),
            Span::styled("   Tab to switch", Style::default().fg(theme::MUTED)),
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
            Style::default().fg(theme::MUTED),
        ))),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(crate::tui::input::spans_with_width(
            &app.import_path_input,
            app.import_cursor,
            Style::default().fg(theme::TEXT),
            chunks[4].width as usize,
        ))),
        chunks[4],
    );

    if app.export_confirm_overwrite {
        f.render_widget(
            Paragraph::new("Destination exists. Replace it? Existing contents will be lost.")
                .style(Style::default().fg(theme::WARNING))
                .wrap(Wrap { trim: false }),
            chunks[5],
        );
        f.render_widget(
            Paragraph::new("[Y] overwrite  [N/Enter/Esc] cancel")
                .style(Style::default().fg(theme::WARNING)),
            chunks[6],
        );
        return;
    }
    if let Some(error) = &app.import_error {
        f.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(theme::ERROR))
                .wrap(Wrap { trim: false }),
            chunks[5],
        );
    }

    let action = if mode == 0 { "import" } else { "export" };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(theme::FOCUS)),
            Span::styled(format!(" {action}  "), Style::default().fg(theme::MUTED)),
            Span::styled("[Tab]", Style::default().fg(theme::FOCUS)),
            Span::styled(" switch  ", Style::default().fg(theme::MUTED)),
            Span::styled("[Esc]", Style::default().fg(theme::FOCUS)),
            Span::styled(" cancel", Style::default().fg(theme::MUTED)),
        ])),
        chunks[6],
    );
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    handle_form_key(app, key)
}

fn handle_form_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if app.host_form.is_none() {
        return Ok(());
    }

    const FIELD_COUNT: usize = 9;

    // Text fields (0..=6) get full line editing; fields 7 and 8 are selectors.
    if app.host_form.as_ref().unwrap().focused < 7
        && !matches!(
            key.code,
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter
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

    match key.code {
        KeyCode::Esc => {
            app.host_form = None;
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            let form = app.host_form.as_mut().unwrap();
            form.focused = (form.focused + 1) % FIELD_COUNT;
            form.cursor = field_cursor(form);
        }
        KeyCode::BackTab => {
            let form = app.host_form.as_mut().unwrap();
            form.focused = form.focused.checked_sub(1).unwrap_or(FIELD_COUNT - 1);
            form.cursor = field_cursor(form);
        }
        KeyCode::Left | KeyCode::Right => match app.host_form.as_ref().unwrap().focused {
            7 => cycle_jump_host(app, key.code == KeyCode::Right),
            8 => cycle_credential(app, key.code == KeyCode::Right),
            _ => {}
        },
        KeyCode::Backspace => {
            let form = app.host_form.as_mut().unwrap();
            match form.focused {
                7 => form.jump_host_id = None,
                8 => form.credential_id = None,
                _ => {}
            }
        }
        KeyCode::Enter => {
            save_host(app)?;
        }
        _ => {}
    }
    Ok(())
}

/// Cycle jump host selection through "(none)" + all hosts except the one being edited.
fn cycle_jump_host(app: &mut App, forward: bool) {
    let form = app.host_form.as_ref().unwrap();
    let candidates: Vec<String> = app
        .hosts
        .iter()
        .filter(|h| Some(&h.id) != form.editing_id.as_ref())
        .map(|h| h.id.clone())
        .collect();
    if candidates.is_empty() {
        return;
    }
    // Options: None, candidates[0], candidates[1], ...
    let current = form
        .jump_host_id
        .as_ref()
        .and_then(|id| candidates.iter().position(|c| c == id))
        .map(|p| p + 1)
        .unwrap_or(0);
    let total = candidates.len() + 1;
    let next = if forward {
        (current + 1) % total
    } else {
        (current + total - 1) % total
    };
    let form = app.host_form.as_mut().unwrap();
    form.jump_host_id = if next == 0 {
        None
    } else {
        Some(candidates[next - 1].clone())
    };
}

/// Cycle Automatic + saved credentials. A removed selection stays visible until changed.
fn cycle_credential(app: &mut App, forward: bool) {
    let form = app.host_form.as_mut().unwrap();
    let current = form
        .credential_id
        .as_ref()
        .and_then(|id| {
            app.credentials
                .iter()
                .position(|credential| &credential.id == id)
        })
        .map(|index| index + 1)
        .unwrap_or(0);
    let total = app.credentials.len() + 1;
    let next = if forward {
        (current + 1) % total
    } else {
        (current + total - 1) % total
    };
    form.credential_id = next
        .checked_sub(1)
        .map(|index| app.credentials[index].id.clone());
}

fn field_cursor(form: &mut HostForm) -> usize {
    if form.focused < 7 {
        crate::tui::input::end_of(active_field(form))
    } else {
        0
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
        _ => &mut form.description,
    }
}

fn save_host(app: &mut App) -> Result<()> {
    save_host_with_writer(app, crate::config::save_hosts)
}

fn save_host_with_writer(
    app: &mut App,
    write_hosts: impl FnOnce(&[Host]) -> Result<()>,
) -> Result<()> {
    let form = match &app.host_form {
        Some(f) => f.clone(),
        None => return Ok(()),
    };

    let set_error = |app: &mut App, err: String, focus: usize| {
        if let Some(ref mut f) = app.host_form {
            f.error_message = Some(err.clone());
            f.focused = focus;
            f.cursor = field_cursor(f);
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
    if let Some(id) = form.credential_id.as_deref() {
        if !app.credentials.iter().any(|credential| credential.id == id) {
            set_error(
                app,
                format!("Selected credential '{id}' is missing. Select another or Automatic."),
                8,
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

    let mut hosts = app.hosts.clone();
    if let Some(ref edit_id) = form.editing_id {
        // Update existing host
        if let Some(h) = hosts.iter_mut().find(|h| &h.id == edit_id) {
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
            h.credential_id = form.credential_id.clone();
        }
    } else {
        // Add new host
        hosts.push(Host {
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
            credential_id: form.credential_id.clone(),
        });
    }

    if let Err(err) = write_hosts(&hosts) {
        if let Some(f) = &mut app.host_form {
            f.error_message = Some(format!("Could not save host: {err}"));
        }
        return Ok(());
    }

    app.hosts = hosts;
    let action = if form.editing_id.is_some() {
        "updated"
    } else {
        "added"
    };
    app.status_message = Some(crate::types::StatusMessage::success(format!(
        "Host '{}' {action}.",
        form.name.trim()
    )));
    app.host_form = None;
    app.screen = Screen::Main;
    Ok(())
}

pub fn handle_import_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    handle_path_key(app, key)
}

fn handle_path_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if app.export_confirm_overwrite {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                app.export_confirm_overwrite = false;
                export_hosts(app, true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Enter | KeyCode::Esc => {
                app.export_confirm_overwrite = false;
            }
            KeyCode::Tab => {
                app.export_confirm_overwrite = false;
                app.import_error = None;
                app.import_export_mode = (app.import_export_mode + 1) % 3;
            }
            _ => {}
        }
        return Ok(());
    }

    if !matches!(key.code, KeyCode::Esc | KeyCode::Tab | KeyCode::Enter) {
        let (mut buf, mut cursor) = (
            std::mem::take(&mut app.import_path_input),
            app.import_cursor,
        );
        let consumed = crate::tui::input::handle(&mut buf, &mut cursor, key);
        app.import_path_input = buf;
        app.import_cursor = cursor;
        if consumed {
            app.import_error = None;
            return Ok(());
        }
    }

    match key.code {
        KeyCode::Esc => {
            app.import_error = None;
            app.export_confirm_overwrite = false;
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            app.import_error = None;
            app.export_confirm_overwrite = false;
            app.import_export_mode = (app.import_export_mode + 1) % 3;
        }
        KeyCode::Enter => {
            let raw = app.import_path_input.trim().to_string();
            if raw.is_empty() {
                set_path_error(app, "Enter a file path.".into());
                return Ok(());
            }
            app.import_error = None;
            let path = expand_tilde(&raw);
            if app.import_export_mode > 0 {
                match std::fs::symlink_metadata(&path) {
                    Ok(metadata) if metadata.is_file() => {
                        app.export_confirm_overwrite = true;
                    }
                    Ok(_) => {
                        set_path_error(
                            app,
                            format!("Export error: '{path}' is not a regular file."),
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        export_hosts(app, false);
                    }
                    Err(error) => {
                        set_path_error(app, format!("Cannot inspect '{path}': {error}"));
                    }
                }
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let previous_hosts = app.hosts.clone();
                        let count = crate::inventory::import_from_ini(&content, &mut app.hosts);
                        let updated = app.hosts.len();
                        if let Err(error) = app.save_hosts() {
                            app.hosts = previous_hosts;
                            set_path_error(app, format!("Import could not be saved: {error}"));
                            return Ok(());
                        }
                        app.status_message = Some(crate::types::StatusMessage::success(format!(
                            "Import complete: {count} new hosts added ({updated} total)."
                        )));
                        app.screen = Screen::Main;
                    }
                    Err(e) => {
                        set_path_error(app, format!("Error reading '{path}': {e}"));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn set_path_error(app: &mut App, error: String) {
    app.import_error = Some(error.clone());
    app.status_message = Some(crate::types::StatusMessage::error(error));
}

fn export_hosts(app: &mut App, overwrite: bool) {
    let path = expand_tilde(app.import_path_input.trim());
    let content = if app.import_export_mode == 1 {
        crate::inventory::export_to_ini(&app.hosts)
    } else {
        crate::inventory::export_to_ssh_config(&app.hosts)
    };
    match atomic_export(std::path::Path::new(&path), &content, overwrite) {
        Ok(()) => {
            app.import_error = None;
            app.status_message = Some(crate::types::StatusMessage::success(format!(
                "Exported {} hosts to {path}.",
                app.hosts.len()
            )));
            app.screen = Screen::Main;
        }
        Err(error) => set_path_error(app, format!("Export to '{path}' failed: {error}")),
    }
}

/// Stage beside the destination so installation is atomic, even across mount points.
/// A hard link installs a new destination without clobbering a file created meanwhile.
fn atomic_export(path: &std::path::Path, content: &str, overwrite: bool) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    anyhow::ensure!(
        path.file_name().is_some(),
        "Destination must be a file path."
    );
    let temporary = parent.join(format!(".hss-export-{}.tmp", Uuid::new_v4()));
    // Do not create missing parent directories or change permissions on user directories.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        if overwrite {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) => {
                    anyhow::ensure!(metadata.is_file(), "Destination is not a regular file.");
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            std::fs::rename(&temporary, path)?;
        } else {
            std::fs::hard_link(&temporary, path)?;
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;

    fn app() -> App {
        App::new(vec![], vec![], crate::config::AppConfig::default(), vec![])
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("hss-export-test-{}", Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn host(id: &str) -> Host {
        Host {
            id: id.into(),
            name: id.into(),
            ip: "192.0.2.1".into(),
            group: "original".into(),
            port: 22,
            user: Some("original-user".into()),
            tags: vec!["original-tag".into()],
            description: Some("original description".into()),
            jump_host_id: None,
            credential_id: None,
        }
    }

    fn assert_failed_save_and_retry(editing: bool) {
        let mut app = app();
        app.hosts = vec![host("existing"), host("jump")];
        app.credentials = vec![crate::types::Credential {
            id: "password-credential".into(),
            name: "credential".into(),
            username: "ssh-user".into(),
            kind: crate::types::CredentialKind::Password,
            key_path: None,
        }];
        app.server_records = vec![crate::types::ServerRecord {
            host_id: "existing".into(),
            last_credential_id: Some("password-credential".into()),
        }];
        app.config.default_credential_id = Some("password-credential".into());
        app.screen = Screen::HostForm;
        app.selected_row = 1;
        app.status_message = Some(crate::types::StatusMessage::info("previous status"));
        app.host_form = Some(HostForm {
            editing_id: editing.then(|| "existing".into()),
            name: " saved-host ".into(),
            ip: " 192.0.2.2 ".into(),
            group: " updated ".into(),
            port: " 2222 ".into(),
            user: " copy-user ".into(),
            tags: " new, tags ".into(),
            description: " updated description ".into(),
            jump_host_id: Some("jump".into()),
            credential_id: Some("password-credential".into()),
            focused: 6,
            cursor: 3,
            error_message: None,
        });
        let original_hosts = serde_json::to_value(&app.hosts).unwrap();
        let original_form = app.host_form.clone().unwrap();
        let original_status = app.status_message.clone();
        let original_metadata =
            serde_json::to_value((&app.credentials, &app.config, &app.server_records)).unwrap();
        let expected_count = if editing { 2 } else { 3 };
        let saved_index = if editing { 0 } else { 2 };
        let mut write_calls = 0;

        // No real config writer or keychain access is needed to exercise either path.
        for _ in 0..2 {
            save_host_with_writer(&mut app, |draft| {
                write_calls += 1;
                assert_eq!(draft.len(), expected_count);
                assert_eq!(draft[saved_index].name, "saved-host");
                assert_eq!(
                    draft[saved_index].credential_id.as_deref(),
                    Some("password-credential")
                );
                if editing {
                    assert_eq!(draft[saved_index].id, "existing");
                }
                anyhow::bail!("disk full")
            })
            .unwrap();
            assert_eq!(serde_json::to_value(&app.hosts).unwrap(), original_hosts);
            assert_eq!(app.screen, Screen::HostForm);
            assert_eq!(app.selected_row, 1);
            assert_eq!(app.status_message, original_status);
            let form = app.host_form.as_ref().unwrap();
            assert_eq!(
                form.error_message.as_deref(),
                Some("Could not save host: disk full")
            );
            let mut expected_form = original_form.clone();
            expected_form.error_message = form.error_message.clone();
            assert_eq!(format!("{form:?}"), format!("{expected_form:?}"));
            assert_eq!(
                serde_json::to_value((&app.credentials, &app.config, &app.server_records)).unwrap(),
                original_metadata
            );
        }
        assert_eq!(
            write_calls, 2,
            "retry must reach the writer, not fail uniqueness"
        );

        let mut persisted = None;
        save_host_with_writer(&mut app, |draft| {
            persisted = Some(serde_json::to_value(draft).unwrap());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            serde_json::to_value(&app.hosts).unwrap(),
            persisted.unwrap()
        );
        assert_eq!(app.hosts.len(), expected_count);
        let saved = &app.hosts[saved_index];
        assert_eq!(saved.name, "saved-host");
        assert_eq!(saved.ip, "192.0.2.2");
        assert_eq!(saved.group, "updated");
        assert_eq!(saved.port, 2222);
        assert_eq!(saved.user.as_deref(), Some("copy-user"));
        assert_eq!(saved.tags, ["new", "tags"]);
        assert_eq!(saved.description.as_deref(), Some("updated description"));
        assert_eq!(saved.jump_host_id.as_deref(), Some("jump"));
        assert_eq!(saved.credential_id.as_deref(), Some("password-credential"));
        if editing {
            assert_eq!(saved.id, "existing");
        }
        assert!(app.host_form.is_none());
        assert_eq!(app.screen, Screen::Main);
        let status = app.status_message.as_ref().unwrap();
        assert_eq!(status.kind, crate::types::StatusKind::Success);
        assert_eq!(
            status.text,
            if editing {
                "Host 'saved-host' updated."
            } else {
                "Host 'saved-host' added."
            }
        );
        assert_eq!(
            serde_json::to_value((&app.credentials, &app.config, &app.server_records)).unwrap(),
            original_metadata
        );
    }

    fn credential(id: &str) -> crate::types::Credential {
        crate::types::Credential {
            id: id.into(),
            name: id.into(),
            username: format!("{id}-user"),
            kind: crate::types::CredentialKind::Key,
            key_path: None,
        }
    }

    #[test]
    fn credentials_selector_cycles_resets_and_traverses_without_editing_text() {
        let mut app = app();
        app.credentials = vec![credential("one"), credential("two")];
        app.host_form = Some(HostForm {
            focused: 7,
            description: "untouched".into(),
            jump_host_id: Some("bastion".into()),
            ..Default::default()
        });
        handle_form_key(&mut app, key(KeyCode::Tab)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().focused, 8);
        for (code, expected) in [
            (KeyCode::Right, Some("one")),
            (KeyCode::Right, Some("two")),
            (KeyCode::Right, None),
            (KeyCode::Left, Some("two")),
            (KeyCode::Backspace, None),
        ] {
            handle_form_key(&mut app, key(code)).unwrap();
            assert_eq!(
                app.host_form.as_ref().unwrap().credential_id.as_deref(),
                expected
            );
        }
        handle_form_key(&mut app, key(KeyCode::Char('x'))).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().description, "untouched");
        assert_eq!(
            app.host_form.as_ref().unwrap().jump_host_id.as_deref(),
            Some("bastion")
        );
        handle_form_key(&mut app, key(KeyCode::Tab)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().focused, 0);
        handle_form_key(&mut app, key(KeyCode::BackTab)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().focused, 8);
        handle_form_key(&mut app, key(KeyCode::BackTab)).unwrap();
        assert_eq!(app.host_form.as_ref().unwrap().focused, 7);
        app.host_form.as_mut().unwrap().focused = 8;
        app.host_form.as_mut().unwrap().credential_id = Some("deleted".into());
        app.credentials.clear();
        handle_form_key(&mut app, key(KeyCode::Right)).unwrap();
        assert!(app.host_form.as_ref().unwrap().credential_id.is_none());
        handle_form_key(&mut app, key(KeyCode::Left)).unwrap();
        assert!(app.host_form.as_ref().unwrap().credential_id.is_none());
    }

    #[test]
    fn missing_draft_credential_blocks_writer_and_reset_saves_automatic() {
        let mut app = app();
        let mut selected = host("existing");
        selected.credential_id = Some("deleted".into());
        app.hosts = vec![selected];
        app.screen = Screen::HostForm;
        app.host_form = Some(HostForm {
            editing_id: Some("existing".into()),
            name: "existing".into(),
            ip: "192.0.2.1".into(),
            port: "22".into(),
            credential_id: Some("deleted".into()),
            ..Default::default()
        });
        let original = serde_json::to_value(&app.hosts).unwrap();
        save_host_with_writer(&mut app, |_| panic!("invalid draft reached writer")).unwrap();
        assert_eq!(serde_json::to_value(&app.hosts).unwrap(), original);
        let form = app.host_form.as_ref().unwrap();
        assert_eq!(form.focused, 8);
        assert_eq!(form.cursor, 0);
        assert!(form.error_message.as_ref().unwrap().contains("deleted"));
        app.host_form.as_mut().unwrap().credential_id = None;
        save_host_with_writer(&mut app, |draft| {
            assert!(draft[0].credential_id.is_none());
            Ok(())
        })
        .unwrap();
        assert!(app.hosts[0].credential_id.is_none());
        assert!(app.host_form.is_none());
    }

    fn form_screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn form_hints_only_render_for_empty_fields_and_focused_selectors() {
        let mut app = app();
        app.credentials = vec![credential("production")];
        app.host_form = Some(HostForm {
            port: "2222".into(),
            tags: "web, prod".into(),
            credential_id: Some("production".into()),
            ..Default::default()
        });
        let text = form_screen(&app, 120, 40);
        assert!(text.contains("web, prod"));
        assert!(text.contains("production (production-user)"));
        assert!(!text.contains("comma-separated"));
        assert!(!text.contains("default 22"));
        assert!(!text.contains("←/→"));
        let form = app.host_form.as_mut().unwrap();
        form.port.clear();
        form.tags.clear();
        form.credential_id = None;
        form.focused = 8;
        let text = form_screen(&app, 120, 40);
        assert!(text.contains("comma-separated"));
        assert!(text.contains("default 22"));
        assert!(text.contains("Backspace reset"));
        for (width, height) in [(60, 20), (80, 24), (40, 12)] {
            let text = form_screen(&app, width, height);
            assert!(
                text.contains("Credentials"),
                "selector not visible at {width}x{height}"
            );
        }
        app.host_form.as_mut().unwrap().credential_id = Some("deleted".into());
        assert!(form_screen(&app, 120, 40).contains("Missing credential: deleted"));
    }

    #[test]
    fn failed_add_save_preserves_state_and_can_retry() {
        assert_failed_save_and_retry(false);
    }

    #[test]
    fn failed_edit_save_preserves_state_and_can_retry() {
        assert_failed_save_and_retry(true);
    }

    #[test]
    fn user_hint_explains_credential_username_without_changing_the_field() {
        for (width, height) in [(60, 20), (80, 24), (40, 12)] {
            let mut app = app();
            app.host_form = Some(HostForm {
                user: "copy-user".into(),
                focused: 4,
                cursor: 3,
                ..HostForm::default()
            });
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            for expected in [
                "copy-user",
                "Copy ID / jump / export",
                "Normal SSH: credential username.",
            ] {
                assert!(
                    text.contains(expected),
                    "missing {expected} at {width}x{height}"
                );
            }
        }
    }

    #[test]
    fn export_requires_explicit_overwrite_and_enter_cancels() {
        let scratch = Scratch::new();
        let path = scratch.0.join("inventory.ini");
        std::fs::write(&path, "original").unwrap();
        let mut app = app();
        app.screen = Screen::ImportHosts;
        app.import_export_mode = 1;
        app.import_path_input = path.to_string_lossy().into_owned();
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert!(app.export_confirm_overwrite);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert!(!app.export_confirm_overwrite);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        // Confirmation owns input; Y must not be inserted into the file path.
        let original_path = app.import_path_input.clone();
        handle_path_key(&mut app, key(KeyCode::Char('x'))).unwrap();
        assert_eq!(app.import_path_input, original_path);
        handle_path_key(&mut app, key(KeyCode::Char('Y'))).unwrap();
        assert_eq!(app.screen, Screen::Main);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
    }

    #[test]
    fn atomic_export_preserves_existing_files_and_cleans_failed_staging() {
        let scratch = Scratch::new();
        let path = scratch.0.join("inventory.ini");
        std::fs::write(&path, "original").unwrap();
        assert!(atomic_export(&path, "replacement", false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
        atomic_export(&path, "replacement", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
        let directory = scratch.0.join("directory");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("keep"), "original").unwrap();
        assert!(atomic_export(&directory, "replacement", true).is_err());
        assert_eq!(
            std::fs::read_to_string(directory.join("keep")).unwrap(),
            "original"
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 2);
        assert!(atomic_export(&scratch.0.join("missing/file"), "data", false).is_err());
        assert!(!scratch.0.join("missing").exists());
    }

    #[test]
    fn export_refuses_symlinks_without_touching_target() {
        use std::os::unix::fs::symlink;
        let scratch = Scratch::new();
        let target = scratch.0.join("target");
        let link = scratch.0.join("link");
        std::fs::write(&target, "original").unwrap();
        symlink(&target, &link).unwrap();
        let mut app = app();
        app.screen = Screen::ImportHosts;
        app.import_export_mode = 1;
        app.import_path_input = link.to_string_lossy().into_owned();
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert!(app.import_error.is_some());
        assert!(!app.export_confirm_overwrite);
        assert!(atomic_export(&link, "replacement", true).is_err());
        assert_eq!(std::fs::read_to_string(target).unwrap(), "original");
    }

    #[test]
    fn failed_export_remains_inline_and_retry_requires_fresh_confirmation() {
        let scratch = Scratch::new();
        let path = scratch.0.join("inventory.ini");
        std::fs::write(&path, "original").unwrap();
        let mut app = app();
        app.screen = Screen::ImportHosts;
        app.import_export_mode = 1;
        app.import_path_input = path.to_string_lossy().into_owned();
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert!(app.export_confirm_overwrite);
        // A destination can change while the user is reading the confirmation.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("keep"), "original").unwrap();
        handle_path_key(&mut app, key(KeyCode::Char('y'))).unwrap();
        assert_eq!(app.screen, Screen::ImportHosts);
        assert!(app
            .import_error
            .as_ref()
            .unwrap()
            .contains("not a regular file"));
        assert!(!app.export_confirm_overwrite);
        assert_eq!(
            std::fs::read_to_string(path.join("keep")).unwrap(),
            "original"
        );
        assert_eq!(std::fs::read_dir(&scratch.0).unwrap().count(), 1);
        std::fs::remove_dir_all(&path).unwrap();
        std::fs::write(&path, "original").unwrap();
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert!(app.export_confirm_overwrite);
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn path_errors_remain_inline_and_clear_on_edit_or_mode_change() {
        let mut app = app();
        app.screen = Screen::ImportHosts;
        handle_path_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert_eq!(app.screen, Screen::ImportHosts);
        assert_eq!(app.import_error.as_deref(), Some("Enter a file path."));
        handle_path_key(&mut app, key(KeyCode::Char('x'))).unwrap();
        assert!(app.import_error.is_none());
        app.import_error = Some("old error".into());
        handle_path_key(&mut app, key(KeyCode::Tab)).unwrap();
        assert!(app.import_error.is_none());
    }

    #[test]
    fn focused_fields_and_long_input_cursor_render_at_small_sizes() {
        let labels = [
            "Name *",
            "IP / Hostname *",
            "Group",
            "Port",
            "User",
            "Tags",
            "Description",
            "Jump host",
        ];
        for (width, height) in [(60, 20), (80, 24), (40, 12)] {
            for (focused, label) in labels.iter().enumerate() {
                let mut app = app();
                app.host_form = Some(HostForm {
                    focused,
                    ..HostForm::default()
                });
                let form = app.host_form.as_mut().unwrap();
                if focused < 7 {
                    *active_field(form) = format!("{}END", "界long".repeat(30));
                    form.cursor = crate::tui::input::end_of(active_field(form));
                }
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|frame| draw(frame, &app)).unwrap();
                let buffer = terminal.backend().buffer();
                let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains(label), "missing {label} at {width}x{height}");
                assert!(
                    text.contains("[Enter]"),
                    "missing save action at {width}x{height}"
                );
                if focused < 7 {
                    assert!(
                        text.contains("END"),
                        "long input end hidden at {width}x{height}"
                    );
                    assert!(
                        buffer
                            .content
                            .iter()
                            .any(|cell| cell.modifier.contains(Modifier::REVERSED)),
                        "cursor hidden for {label} at {width}x{height}"
                    );
                }
            }
        }
    }

    #[test]
    fn path_cursor_errors_and_overwrite_prompt_render_at_small_sizes() {
        for (width, height) in [(60, 20), (80, 24)] {
            let mut app = app();
            app.import_export_mode = 1;
            app.import_path_input = format!("{}END", "long/path/".repeat(30));
            app.import_cursor = crate::tui::input::end_of(&app.import_path_input);
            app.import_error = Some("Export could not be saved".into());
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw_import(frame, &app)).unwrap();
            let buffer = terminal.backend().buffer();
            let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
            assert!(text.contains("END"));
            assert!(text.contains("Export could not be saved"));
            assert!(buffer
                .content
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::REVERSED)));
            app.export_confirm_overwrite = true;
            terminal.draw(|frame| draw_import(frame, &app)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("Destination exists."));
            assert!(text.contains("[N/Enter/Esc] cancel"));
        }
    }
}
