use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use uuid::Uuid;
use crate::tui::{App, Screen, Term};
use crate::types::{Host, HostForm};

pub fn draw(f: &mut Frame, app: &App) {
    let Some(form) = &app.host_form else { return };

    let area = centered_rect(60, 80, f.area());
    f.render_widget(Clear, area);

    let title = if form.editing_id.is_some() { " Edit Host " } else { " Add Host " };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Layout: 8 field rows + 1 hotkey row
    let constraints: Vec<Constraint> = (0..9).map(|i| {
        if i == 8 { Constraint::Length(1) } else { Constraint::Length(2) }
    }).collect();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .margin(1)
        .split(inner);

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
        let focused = form.focused == i;
        let label_style = if focused {
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let input_text = if focused {
            format!("{value}█")
        } else {
            value.to_string()
        };
        let input_style = if focused {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        let suffix = match i {
            3 => " (default 22)",
            5 => " (comma-separated)",
            _ => "",
        };
        let line = Line::from(vec![
            Span::styled(format!("{:<18}", label), label_style),
            Span::styled(input_text, input_style),
            Span::styled(suffix, Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), chunks[i]);
    }

    // Jump host selector (field 7): cycled with ←/→ among existing hosts
    {
        let focused = form.focused == 7;
        let label_style = if focused {
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value = form.jump_host_id.as_deref()
            .and_then(|id| app.hosts.iter().find(|h| h.id == id))
            .map(|h| format!("{} ({})", h.name, h.ip))
            .unwrap_or_else(|| "(none)".into());
        let input_style = if focused {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        let display = if focused { format!("◂ {value} ▸") } else { value };
        let line = Line::from(vec![
            Span::styled(format!("{:<18}", "Jump host"), label_style),
            Span::styled(display, input_style),
            Span::styled(" (←/→ to select)", Style::default().fg(Color::DarkGray)),
        ]);
        f.render_widget(Paragraph::new(line), chunks[7]);
    }

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(Color::Blue)),
        Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Enter]", Style::default().fg(Color::Blue)),
        Span::styled(" save  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Blue)),
        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[8]);
}

pub fn draw_import(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    let is_export = app.import_export_mode;
    let title = if is_export { " Export Ansible INI " } else { " Import Ansible INI " };
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
    let (imp_style, exp_style) = if is_export {
        (Style::default().fg(Color::DarkGray), Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
    } else {
        (Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD), Style::default().fg(Color::DarkGray))
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Import]", imp_style),
            Span::raw("  "),
            Span::styled("[Export]", exp_style),
            Span::styled("   Tab to switch", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let label = if is_export { "Save to path:" } else { "Path to inventory file:" };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(label, Style::default().fg(Color::DarkGray)))),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(&app.import_path_input, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Blue)),
        ])),
        chunks[4],
    );

    let action = if is_export { "export" } else { "import" };
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

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if app.host_form.is_none() {
        return Ok(());
    }

    const FIELD_COUNT: usize = 8;

    match key.code {
        KeyCode::Esc => {
            app.host_form = None;
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            let form = app.host_form.as_mut().unwrap();
            form.focused = (form.focused + 1) % FIELD_COUNT;
        }
        KeyCode::BackTab => {
            let form = app.host_form.as_mut().unwrap();
            form.focused = form.focused.checked_sub(1).unwrap_or(FIELD_COUNT - 1);
        }
        KeyCode::Left | KeyCode::Right if app.host_form.as_ref().unwrap().focused == 7 => {
            cycle_jump_host(app, key.code == KeyCode::Right);
        }
        KeyCode::Backspace => {
            let form = app.host_form.as_mut().unwrap();
            if form.focused == 7 {
                form.jump_host_id = None;
            } else {
                active_field(form).pop();
            }
        }
        KeyCode::Char(c) => {
            let form = app.host_form.as_mut().unwrap();
            if form.focused != 7 {
                active_field(form).push(c);
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
    let candidates: Vec<String> = app.hosts.iter()
        .filter(|h| Some(&h.id) != form.editing_id.as_ref())
        .map(|h| h.id.clone())
        .collect();
    if candidates.is_empty() {
        return;
    }
    // Options: None, candidates[0], candidates[1], ...
    let current = form.jump_host_id.as_ref()
        .and_then(|id| candidates.iter().position(|c| c == id))
        .map(|p| p + 1)
        .unwrap_or(0);
    let total = candidates.len() + 1;
    let next = if forward { (current + 1) % total } else { (current + total - 1) % total };
    let form = app.host_form.as_mut().unwrap();
    form.jump_host_id = if next == 0 { None } else { Some(candidates[next - 1].clone()) };
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
    let form = match &app.host_form {
        Some(f) => f.clone(),
        None => return Ok(()),
    };

    if form.name.trim().is_empty() {
        app.status_message = Some("Name is required.".into());
        return Ok(());
    }
    if form.ip.trim().is_empty() {
        app.status_message = Some("IP / Hostname is required.".into());
        return Ok(());
    }
    let port: u16 = form.port.trim().parse().unwrap_or(22);
    let tags: Vec<String> = form.tags.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();

    if let Some(ref edit_id) = form.editing_id {
        // Update existing host
        if let Some(h) = app.hosts.iter_mut().find(|h| &h.id == edit_id) {
            h.name = form.name.trim().to_string();
            h.ip = form.ip.trim().to_string();
            h.group = form.group.trim().to_string();
            h.port = port;
            h.user = if form.user.trim().is_empty() { None } else { Some(form.user.trim().to_string()) };
            h.tags = tags;
            h.description = if form.description.trim().is_empty() { None } else { Some(form.description.trim().to_string()) };
            h.jump_host_id = form.jump_host_id.clone();
        }
        app.status_message = Some(format!("Host '{}' updated.", form.name.trim()));
    } else {
        // Add new host
        app.hosts.push(Host {
            id: Uuid::new_v4().to_string(),
            name: form.name.trim().to_string(),
            ip: form.ip.trim().to_string(),
            group: form.group.trim().to_string(),
            port,
            user: if form.user.trim().is_empty() { None } else { Some(form.user.trim().to_string()) },
            tags,
            description: if form.description.trim().is_empty() { None } else { Some(form.description.trim().to_string()) },
            jump_host_id: form.jump_host_id.clone(),
        });
        app.status_message = Some(format!("Host '{}' added.", form.name.trim()));
    }

    app.save_hosts()?;
    app.host_form = None;
    app.screen = Screen::Main;
    Ok(())
}

pub fn handle_import_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            app.import_export_mode = !app.import_export_mode;
        }
        KeyCode::Backspace => {
            app.import_path_input.pop();
        }
        KeyCode::Char(c) => {
            app.import_path_input.push(c);
        }
        KeyCode::Enter => {
            let raw = app.import_path_input.trim().to_string();
            if raw.is_empty() {
                app.screen = Screen::Main;
                return Ok(());
            }
            let path = expand_tilde(&raw);
            if app.import_export_mode {
                let content = crate::inventory::export_to_ini(&app.hosts);
                match std::fs::write(&path, &content) {
                    Ok(()) => {
                        app.status_message = Some(format!("Exported {} hosts to {path}.", app.hosts.len()));
                        app.screen = Screen::Main;
                    }
                    Err(e) => {
                        app.status_message = Some(format!("Export error: {e}"));
                    }
                }
            } else {
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let count = crate::inventory::import_from_ini(&content, &mut app.hosts);
                        let updated = app.hosts.len();
                        app.save_hosts()?;
                        app.status_message = Some(format!(
                            "Import complete: {count} new hosts added ({updated} total)."
                        ));
                        app.screen = Screen::Main;
                    }
                    Err(e) => {
                        app.status_message = Some(format!("Error reading file: {e}"));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
