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

    // Layout: 7 field rows + 1 hotkey row
    let constraints: Vec<Constraint> = (0..8).map(|i| {
        if i == 7 { Constraint::Length(1) } else { Constraint::Length(2) }
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

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(Color::Blue)),
        Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Enter]", Style::default().fg(Color::Blue)),
        Span::styled(" save  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Blue)),
        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[7]);
}

pub fn draw_import(f: &mut Frame, app: &App) {
    let area = centered_rect(60, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Import Ansible INI ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(2), Constraint::Length(1)])
        .margin(1)
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Path to inventory file:",
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[0],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(&app.import_path_input, Style::default().fg(Color::White)),
            Span::styled("█", Style::default().fg(Color::Blue)),
        ])),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(Color::Blue)),
            Span::styled(" import  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc]", Style::default().fg(Color::Blue)),
            Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[2],
    );
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if app.host_form.is_none() {
        return Ok(());
    }

    const FIELD_COUNT: usize = 7;

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
        KeyCode::Backspace => {
            let form = app.host_form.as_mut().unwrap();
            active_field(form).pop();
        }
        KeyCode::Char(c) => {
            let form = app.host_form.as_mut().unwrap();
            active_field(form).push(c);
        }
        KeyCode::Enter => {
            save_host(app)?;
        }
        _ => {}
    }
    Ok(())
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
        KeyCode::Backspace => {
            app.import_path_input.pop();
        }
        KeyCode::Char(c) => {
            app.import_path_input.push(c);
        }
        KeyCode::Enter => {
            let path = app.import_path_input.trim().to_string();
            if path.is_empty() {
                app.screen = Screen::Main;
                return Ok(());
            }
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
                    // Stay on ImportHosts screen so user can correct the path
                }
            }
        }
        _ => {}
    }
    Ok(())
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
