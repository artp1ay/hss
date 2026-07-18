use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let Some(form) = &app.copy_id_form else { return };
    let host_name = app.hosts.get(form.host_idx).map(|h| h.name.as_str()).unwrap_or("?");

    let area = centered_rect(64, 70, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(format!(" ssh-copy-id → {host_name} "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let key_rows = form.keys.len().max(1) as u16;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),          // keys label
            Constraint::Length(key_rows),   // key list
            Constraint::Length(1),          // spacer
            Constraint::Length(1),          // user
            Constraint::Length(1),          // password
            Constraint::Min(0),
            Constraint::Length(1),          // hotkeys
        ])
        .margin(1)
        .split(inner);

    let keys_label_style = if form.focused == 0 {
        Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled("Keys to copy (Space to toggle):", keys_label_style))),
        chunks[0],
    );

    if form.keys.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("No public keys found in ~/.ssh", Style::default().fg(Color::Red))),
            chunks[1],
        );
    } else {
        let home = dirs::home_dir().map(|h| h.display().to_string()).unwrap_or_default();
        let lines: Vec<Line> = form.keys.iter().enumerate().map(|(i, (path, selected))| {
            let cursor = form.focused == 0 && i == form.key_cursor;
            let mark = if *selected { "[x]" } else { "[ ]" };
            let display_path = path.strip_prefix(&home).map(|p| format!("~{p}")).unwrap_or_else(|| path.clone());
            let style = if cursor {
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
            } else if *selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Gray)
            };
            Line::from(Span::styled(
                format!("{} {mark} {display_path}", if cursor { "▶" } else { " " }),
                style,
            ))
        }).collect();
        f.render_widget(Paragraph::new(lines), chunks[1]);
    }

    for (idx, chunk, label, value, mask) in [
        (1usize, chunks[3], "User:    ", form.user.clone(), false),
        (2, chunks[4], "Password:", "•".repeat(form.password.chars().count()), true),
    ] {
        let focused = form.focused == idx;
        let label_style = if focused {
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let text = if focused { format!("{value}█") } else { value };
        let hint = if mask && form.password.is_empty() && !focused {
            "  (empty = use agent/existing keys)"
        } else {
            ""
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("{label} "), label_style),
                Span::styled(text, Style::default().fg(Color::White)),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ])),
            chunk,
        );
    }

    let hotkeys = Line::from(vec![
        Span::styled("[Tab]", Style::default().fg(Color::Blue)),
        Span::styled(" section  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Space]", Style::default().fg(Color::Blue)),
        Span::styled(" toggle  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Enter]", Style::default().fg(Color::Blue)),
        Span::styled(" copy  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Blue)),
        Span::styled(" cancel", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[6]);
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(form) = app.copy_id_form.as_mut() else { return Ok(()) };

    match key.code {
        KeyCode::Esc => {
            app.copy_id_form = None;
            app.screen = Screen::Main;
        }
        KeyCode::Tab => form.focused = (form.focused + 1) % 3,
        KeyCode::BackTab => form.focused = (form.focused + 2) % 3,
        KeyCode::Up if form.focused == 0 => form.key_cursor = form.key_cursor.saturating_sub(1),
        KeyCode::Down if form.focused == 0 => {
            if !form.keys.is_empty() {
                form.key_cursor = (form.key_cursor + 1).min(form.keys.len() - 1);
            }
        }
        KeyCode::Char(' ') if form.focused == 0 => {
            if let Some(k) = form.keys.get_mut(form.key_cursor) {
                k.1 = !k.1;
            }
        }
        KeyCode::Backspace => {
            match form.focused {
                1 => { form.user.pop(); }
                2 => { form.password.pop(); }
                _ => {}
            }
        }
        KeyCode::Char(c) if form.focused == 1 => form.user.push(c),
        KeyCode::Char(c) if form.focused == 2 => form.password.push(c),
        KeyCode::Enter => run_copy(app)?,
        _ => {}
    }
    Ok(())
}

fn run_copy(app: &mut App) -> Result<()> {
    let Some(form) = app.copy_id_form.clone() else { return Ok(()) };
    let Some(host) = app.hosts.get(form.host_idx).cloned() else { return Ok(()) };

    let selected: Vec<&String> = form.keys.iter().filter(|(_, s)| *s).map(|(p, _)| p).collect();
    if selected.is_empty() {
        app.status_message = Some("Select at least one key (Space).".into());
        return Ok(());
    }
    let user = form.user.trim();
    if user.is_empty() {
        app.status_message = Some("User is required.".into());
        return Ok(());
    }
    let password = if form.password.is_empty() { None } else { Some(form.password.as_str()) };

    let mut copied = 0;
    let mut errors: Vec<String> = Vec::new();
    for key in &selected {
        match crate::ssh::copy_id(&host.ip, host.port, user, key, password, &app.config) {
            Ok(out) if out.status.success() => copied += 1,
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let last = stderr.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed");
                errors.push(format!("{}: {last}", short_name(key)));
            }
            Err(e) => errors.push(format!("{}: {e}", short_name(key))),
        }
    }

    if errors.is_empty() {
        app.status_message = Some(format!("Copied {copied} key(s) to {}@{}.", user, host.name));
        app.copy_id_form = None;
        app.screen = Screen::Main;
    } else {
        // Keep the form open so the user can fix the password and retry
        app.status_message = Some(format!("Copied {copied}, failed {}: {}", errors.len(), errors.join("; ")));
    }
    Ok(())
}

fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
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
