use crate::tui::{theme, App, Screen, Term};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph, Wrap},
    Frame,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

pub fn draw(f: &mut Frame, app: &App) {
    let Some(form) = &app.copy_id_form else {
        return;
    };
    let host_name = app
        .hosts
        .iter()
        .find(|host| host.id == form.host_id)
        .map(|h| h.name.as_str())
        .unwrap_or("?");

    let area = theme::dialog_area(crate::tui::help::content_area(app, f.area()), 22);
    f.render_widget(Clear, area);

    let block = theme::dialog_block(format!("ssh-copy-id → {host_name}"));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let active = is_active(app);
    let hotkeys: &[(&str, &str)] = if active {
        &[("Esc", "back"), ("Ctrl+C", "stop")]
    } else {
        &[
            ("Tab", "section"),
            ("Space", "toggle"),
            ("Enter", "copy"),
            ("Esc", "back"),
        ]
    };
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkeys, inner.width.saturating_sub(2));
    let error_rows = form
        .error_message
        .as_deref()
        .map(|message| {
            crate::tui::credentials_screen::wrap_message(message, inner.width.saturating_sub(2))
                .len()
                .min(4) as u16
        })
        .unwrap_or(1);
    let key_rows = form.keys.len().clamp(1, 8) as u16;
    let key_rows = key_rows.min(
        inner
            .height
            .saturating_sub(9 + error_rows + hotkey_lines.len() as u16),
    );
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),          // keys label
            Constraint::Length(key_rows),   // key list
            Constraint::Length(1),          // spacer
            Constraint::Length(1),          // user
            Constraint::Length(1),          // password
            Constraint::Length(error_rows), // error / progress
            Constraint::Min(0),
            Constraint::Length(2), // cancellation warning
            Constraint::Length(hotkey_lines.len() as u16), // hotkeys
        ])
        .margin(1)
        .split(inner);

    let keys_label_style = if form.focused == 0 && !active {
        Style::default()
            .fg(theme::FOCUS)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::MUTED)
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if active {
                "Keys to copy (read-only while running):"
            } else {
                "Keys to copy (Space to toggle):"
            },
            keys_label_style,
        ))),
        chunks[0],
    );

    if form.keys.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "No public keys found in ~/.ssh",
                Style::default().fg(theme::ERROR),
            )),
            chunks[1],
        );
    } else {
        let home = dirs::home_dir()
            .map(|h| h.display().to_string())
            .unwrap_or_default();
        let viewport = key_viewport(form.key_cursor, form.keys.len(), chunks[1].height as usize);
        let lines: Vec<Line> = form
            .keys
            .iter()
            .enumerate()
            .skip(viewport.start)
            .take(viewport.len())
            .map(|(i, (path, selected))| {
                let cursor = !active && form.focused == 0 && i == form.key_cursor;
                let mark = if *selected { "[x]" } else { "[ ]" };
                let display_path = path
                    .strip_prefix(&home)
                    .map(|p| format!("~{p}"))
                    .unwrap_or_else(|| path.clone());
                let style = if cursor {
                    Style::default()
                        .fg(theme::FOCUS)
                        .bg(theme::SELECTION)
                        .add_modifier(Modifier::BOLD)
                } else if *selected {
                    Style::default().fg(theme::FOCUS).bg(theme::SELECTION)
                } else {
                    Style::default().fg(theme::MUTED)
                };
                Line::from(Span::styled(
                    format!("{} {mark} {display_path}", if cursor { "▶" } else { " " }),
                    style,
                ))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), chunks[1]);
    }

    for (idx, chunk, label, value, mask) in [
        (1usize, chunks[3], "User:", form.user.clone(), false),
        (
            2,
            chunks[4],
            "Password:",
            "•".repeat(form.password.chars().count()),
            true,
        ),
    ] {
        let focused = !active && form.focused == idx;
        let label_style = theme::field_label(focused);
        let label_width = theme::FORM_LABEL_WIDTH.min(chunk.width.saturating_sub(1));
        let hint = if mask && form.password.is_empty() && !focused {
            "  (empty = use agent/existing keys)"
        } else {
            ""
        };
        let mut spans = vec![Span::styled(
            format!("{label:width$}", width = label_width as usize),
            label_style,
        )];
        if focused {
            spans.extend(crate::tui::input::spans_with_width(
                &value,
                form.cursor,
                theme::field_value(),
                usize::from(chunk.width).saturating_sub(label_width as usize),
            ));
        } else {
            spans.push(Span::styled(value, Style::default().fg(theme::TEXT)));
        }
        spans.push(Span::styled(hint, Style::default().fg(theme::MUTED)));
        f.render_widget(Paragraph::new(Line::from(spans)), chunk);
    }

    if let Some(ref progress) = form.progress_status {
        let prog_line = Line::from(vec![
            Span::styled("⏳ ", Style::default().fg(theme::INFO)),
            Span::styled(progress, Style::default().fg(theme::INFO)),
        ]);
        f.render_widget(Paragraph::new(prog_line), chunks[5]);
    } else if let Some(ref err) = form.error_message {
        let err_line = Line::from(vec![
            Span::styled(
                "✗ ",
                Style::default()
                    .fg(theme::ERROR)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(err, Style::default().fg(theme::ERROR)),
        ]);
        f.render_widget(
            Paragraph::new(err_line).wrap(Wrap { trim: false }),
            chunks[5],
        );
    }

    f.render_widget(
        Paragraph::new(vec![
            Line::from("Installed keys cannot be undone."),
            Line::from("Stopping only cancels remaining work."),
        ])
        .style(Style::default().fg(theme::WARNING)),
        chunks[7],
    );

    f.render_widget(Paragraph::new(hotkey_lines), chunks[8]);
}

fn is_active(app: &App) -> bool {
    app.copy_id_rx.is_some()
        || app.copy_id_worker.is_some()
        || app.copy_id_form.as_ref().is_some_and(|f| f.in_progress)
}

fn key_viewport(cursor: usize, count: usize, rows: usize) -> std::ops::Range<usize> {
    let rows = rows.min(count);
    let start = cursor
        .min(count.saturating_sub(1))
        .saturating_sub(rows / 2)
        .min(count.saturating_sub(rows));
    start..start + rows
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    handle_form_key(app, key)
}

fn handle_form_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let active = is_active(app);
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'C'))
    {
        if let Some(cancel) = &app.copy_id_cancel {
            cancel.store(true, Ordering::Release);
            if let Some(form) = &mut app.copy_id_form {
                form.progress_status = Some("Stopping... installed keys cannot be undone.".into());
            }
        }
        return Ok(());
    }
    if key.code == KeyCode::Esc {
        if !active {
            app.copy_id_form = None;
        }
        app.screen = Screen::Main;
        return Ok(());
    }
    if active {
        return Ok(());
    }
    let Some(form) = app.copy_id_form.as_mut() else {
        return Ok(());
    };

    // Text fields (1=user, 2=password) get full line editing; field 0 is the key list.
    if form.focused > 0
        && !matches!(
            key.code,
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab | KeyCode::Enter
        )
    {
        let field = if form.focused == 1 {
            &mut form.user
        } else {
            &mut form.password
        };
        let mut cursor = form.cursor;
        let mut buf = std::mem::take(field);
        let consumed = crate::tui::input::handle(&mut buf, &mut cursor, key);
        *(if form.focused == 1 {
            &mut form.user
        } else {
            &mut form.password
        }) = buf;
        form.cursor = cursor;
        if consumed {
            return Ok(());
        }
    }

    match key.code {
        KeyCode::Tab | KeyCode::BackTab => {
            form.focused = if key.code == KeyCode::Tab {
                (form.focused + 1) % 3
            } else {
                (form.focused + 2) % 3
            };
            form.cursor = match form.focused {
                1 => crate::tui::input::end_of(&form.user),
                2 => crate::tui::input::end_of(&form.password),
                _ => 0,
            };
        }
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
        KeyCode::Enter => run_copy(app)?,
        _ => {}
    }
    Ok(())
}

fn run_copy(app: &mut App) -> Result<()> {
    if is_active(app) {
        return Ok(());
    }
    let Some(form) = app.copy_id_form.clone() else {
        return Ok(());
    };
    let Some(host) = app
        .hosts
        .iter()
        .find(|host| host.id == form.host_id)
        .cloned()
    else {
        if let Some(form) = &mut app.copy_id_form {
            form.error_message =
                Some("Host no longer exists. Close Copy ID and select another host.".into());
        }
        return Ok(());
    };

    if host.jump_host_id.is_some() {
        let message = "Copy ID does not support jump-host routes. No keys were copied. Install the key manually using SSH with the configured jump host.";
        if let Some(form) = &mut app.copy_id_form {
            form.error_message = Some(message.into());
        }
        app.status_message = Some(crate::types::StatusMessage::error(message));
        return Ok(());
    }

    let selected: Vec<&String> = form
        .keys
        .iter()
        .filter(|(_, s)| *s)
        .map(|(p, _)| p)
        .collect();
    if selected.is_empty() {
        if let Some(ref mut f) = app.copy_id_form {
            f.error_message = Some("Select at least one key (Space).".into());
            f.focused = 0;
        }
        app.status_message = Some(crate::types::StatusMessage::warning(
            "Select at least one key (Space).",
        ));
        return Ok(());
    }
    let user = form.user.trim();
    if user.is_empty() {
        if let Some(ref mut f) = app.copy_id_form {
            f.error_message = Some("User is required.".into());
            f.focused = 1;
        }
        app.status_message = Some(crate::types::StatusMessage::warning("User is required."));
        return Ok(());
    }
    if let Err(e) = crate::ssh::validate_username(user) {
        if let Some(ref mut f) = app.copy_id_form {
            f.error_message = Some(format!("Invalid Username: {e}"));
            f.focused = 1;
        }
        app.status_message = Some(crate::types::StatusMessage::error(format!(
            "Invalid Username: {e}"
        )));
        return Ok(());
    }
    let user = user.to_string();
    let password = if form.password.is_empty() {
        None
    } else {
        Some(form.password.clone())
    };

    let selected_keys: Vec<String> = selected.into_iter().cloned().collect();
    let total = selected_keys.len();

    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let cfg = app.config.clone();
    let worker = std::thread::Builder::new().name("hss-copy-id".into()).spawn(move || {
        let mut copied = 0;
        let mut errors: Vec<String> = Vec::new();
        for (idx, key) in selected_keys.iter().enumerate() {
            if worker_cancel.load(Ordering::Acquire) {
                break;
            }
            let _ = tx.send(crate::tui::CopyIdEvent::Progress(format!(
                "Copying {} of {} key(s)...",
                idx + 1,
                total
            )));
            match crate::ssh::copy_id_cancellable(
                &host.ip,
                host.port,
                &user,
                key,
                password.as_deref(),
                &cfg,
                &worker_cancel,
            ) {
                Ok(out) if out.status.success() => copied += 1,
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let last = stderr
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or("failed");
                    errors.push(format!("{}: {last}", short_name(key)));
                }
                Err(e) => errors.push(format!("{}: {e}", short_name(key))),
            }
        }

        if worker_cancel.load(Ordering::Acquire) {
            let _ = tx.send(crate::tui::CopyIdEvent::Cancelled(format!(
                "Cancelled: {copied} key(s) confirmed installed on {}@{}. Installed keys cannot be undone; the interrupted key may also be installed.",
                user, host.name
            )));
        } else if errors.is_empty() {
            let _ = tx.send(crate::tui::CopyIdEvent::Success(format!(
                "Copied {copied} key(s) to {}@{}.",
                user, host.name
            )));
        } else {
            let _ = tx.send(crate::tui::CopyIdEvent::Failure(format!(
                "Copied {copied}, failed {}: {}",
                errors.len(),
                errors.join("; ")
            )));
        }
    });
    match worker {
        Ok(worker) => {
            app.copy_id_rx = Some(rx);
            app.copy_id_cancel = Some(cancel);
            app.copy_id_worker = Some(worker);
            if let Some(f) = &mut app.copy_id_form {
                f.in_progress = true;
                f.error_message = None;
                f.progress_status = Some(format!("Copying 1 of {total} key(s)..."));
            }
        }
        Err(error) => {
            let message = format!("Could not start Copy ID worker: {error}");
            if let Some(f) = &mut app.copy_id_form {
                f.error_message = Some(message.clone());
            }
            app.status_message = Some(crate::types::StatusMessage::error(message));
        }
    }

    Ok(())
}

fn short_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let host = crate::types::Host {
            id: "test".into(),
            name: "test".into(),
            // Invalid by design: a lifecycle regression must never invoke SSH.
            ip: "invalid host".into(),
            group: String::new(),
            port: 22,
            user: None,
            tags: Vec::new(),
            description: None,
            jump_host_id: None,
            credential_id: None,
        };
        let mut app = App::new(vec![host], Vec::new(), Default::default(), Vec::new());
        app.screen = Screen::CopyId;
        app.copy_id_form = Some(crate::types::CopyIdForm {
            host_id: "test".into(),
            keys: vec![("test.pub".into(), true)],
            user: "user".into(),
            password: "secret".into(),
            ..Default::default()
        });
        app
    }

    #[test]
    fn active_form_is_read_only_and_escape_preserves_it() {
        let mut app = app();
        app.copy_id_form.as_mut().unwrap().in_progress = true;
        for focus in 0..3 {
            app.copy_id_form.as_mut().unwrap().focused = focus;
            for code in [
                KeyCode::Char(' '),
                KeyCode::Char('x'),
                KeyCode::Backspace,
                KeyCode::Tab,
                KeyCode::Down,
                KeyCode::Enter,
            ] {
                handle_form_key(&mut app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
            }
            let form = app.copy_id_form.as_ref().unwrap();
            assert_eq!(form.focused, focus);
            assert_eq!(form.key_cursor, 0);
            assert!(form.keys[0].1);
            assert_eq!(form.user, "user");
            assert_eq!(form.password, "secret");
        }
        handle_form_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
        assert!(matches!(app.screen, Screen::Main));
        assert!(app.copy_id_form.as_ref().unwrap().in_progress);
        assert!(!app.should_quit);
    }

    #[test]
    fn ctrl_c_requests_cancellation_without_quitting_or_editing() {
        let mut app = app();
        app.copy_id_form.as_mut().unwrap().focused = 2;
        let cancel = Arc::new(AtomicBool::new(false));
        app.copy_id_cancel = Some(cancel.clone());
        handle_form_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert!(cancel.load(Ordering::Acquire));
        assert!(!app.should_quit);
        assert!(matches!(app.screen, Screen::CopyId));
        assert_eq!(app.copy_id_form.as_ref().unwrap().password, "secret");
    }

    #[test]
    fn duplicate_submit_preserves_receiver_and_cancel_flag() {
        let mut app = app();
        app.copy_id_form.as_mut().unwrap().keys.clear();
        let (tx, rx) = std::sync::mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        app.copy_id_rx = Some(rx);
        app.copy_id_cancel = Some(cancel.clone());
        run_copy(&mut app).unwrap();
        assert!(app.copy_id_form.as_ref().unwrap().error_message.is_none());
        assert!(Arc::ptr_eq(app.copy_id_cancel.as_ref().unwrap(), &cancel));
        tx.send(crate::tui::CopyIdEvent::Progress("existing".into()))
            .unwrap();
        assert!(
            matches!(app.copy_id_rx.as_ref().unwrap().try_recv().unwrap(), crate::tui::CopyIdEvent::Progress(msg) if msg == "existing")
        );
    }

    #[test]
    fn worker_handle_blocks_resubmission_until_parent_cleanup() {
        let mut app = app();
        app.copy_id_form.as_mut().unwrap().keys.clear();
        app.copy_id_worker = Some(std::thread::spawn(|| {}));
        run_copy(&mut app).unwrap();
        assert!(app.copy_id_form.as_ref().unwrap().error_message.is_none());
        assert!(app.copy_id_rx.is_none());
        app.copy_id_worker.take().unwrap().join().unwrap();
    }

    #[test]
    fn deleting_host_never_redirects_copy_to_another_row() {
        let mut app = app();
        let mut other = app.hosts[0].clone();
        other.id = "other".into();
        other.name = "other".into();
        app.hosts = vec![other];
        run_copy(&mut app).unwrap();
        assert!(app.copy_id_rx.is_none());
        assert!(app.copy_id_worker.is_none());
        assert!(app
            .copy_id_form
            .as_ref()
            .unwrap()
            .error_message
            .as_ref()
            .unwrap()
            .contains("no longer exists"));
    }

    #[test]
    fn key_viewport_is_bounded_and_keeps_cursor_visible() {
        for count in [0, 1, 8, 1000] {
            for rows in [0, 1, 3, 8] {
                for cursor in 0..count {
                    let viewport = key_viewport(cursor, count, rows);
                    assert!(viewport.len() <= rows);
                    assert!(viewport.end <= count);
                    if rows > 0 {
                        assert!(viewport.contains(&cursor));
                    }
                }
            }
        }
        assert_eq!(key_viewport(0, 0, 8), 0..0);
    }

    #[test]
    fn draw_handles_small_terminals_and_large_key_lists() {
        let mut app = app();
        let form = app.copy_id_form.as_mut().unwrap();
        form.keys = (0..1000).map(|i| (format!("key-{i}.pub"), false)).collect();
        form.key_cursor = 999;
        form.focused = 1;
        form.user = "界".repeat(100);
        form.cursor = 100;
        for (width, height) in [(1, 1), (20, 8), (80, 24)] {
            let backend = ratatui::backend::TestBackend::new(width, height);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }
    }
}
