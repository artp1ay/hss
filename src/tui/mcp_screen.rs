use crate::mcp::{Level, Log, LogState};
use crate::tui::{theme, App, Screen, Term};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::time::Instant;

pub fn draw(f: &mut Frame, app: &App) {
    let Some(ref mcp) = app.mcp else { return };
    draw_server(
        f,
        &mcp.server_url(),
        mcp.started,
        &mcp.log,
        app.mcp_scroll_offset,
        crate::tui::help::content_area(app, f.area()),
    );
}

fn block(title: &'static str) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::MUTED))
}

struct ServerView {
    areas: [Rect; 4],
    status: Vec<Line<'static>>,
    activity: Vec<Line<'static>>,
    hotkeys: Vec<Line<'static>>,
}

impl ServerView {
    fn max_scroll(&self) -> usize {
        self.activity.len().saturating_sub(usize::from(
            block(" Activity log ").inner(self.areas[2]).height.max(1),
        ))
    }
}

fn server_view(url: &str, started: Instant, log: &LogState, area: Rect) -> ServerView {
    let hotkeys = crate::tui::wrap_hotkey_lines(
        &[
            ("↑/↓/PgUp/PgDn", "scroll"),
            ("Home/End", "top/bottom"),
            ("Esc/Q", "stop & back"),
        ],
        area.width,
    );
    let uptime = started.elapsed().as_secs();
    let status = vec![
        Line::from(vec![
            Span::styled(
                "● Running  ",
                Style::default()
                    .fg(theme::SUCCESS)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(url.to_owned(), Style::default().fg(theme::TEXT)),
            Span::styled(
                format!("  up {}m{:02}s", uptime / 60, uptime % 60),
                Style::default().fg(theme::MUTED),
            ),
        ]),
        Line::styled(
            "Stop blocks new requests; active calls continue.",
            Style::default().fg(theme::WARNING),
        ),
        Line::from(vec![
            Span::styled("requests ", Style::default().fg(theme::MUTED)),
            Span::styled(log.requests.to_string(), Style::default().fg(theme::TEXT)),
            Span::styled("   tool calls ", Style::default().fg(theme::MUTED)),
            Span::styled(log.tool_calls.to_string(), Style::default().fg(theme::TEXT)),
            Span::styled(
                format!("   active {}", log.active_calls),
                Style::default().fg(theme::INFO),
            ),
            Span::styled("   errors ", Style::default().fg(theme::MUTED)),
            Span::styled(
                log.errors.to_string(),
                Style::default().fg(if log.errors > 0 {
                    theme::ERROR
                } else {
                    theme::TEXT
                }),
            ),
        ]),
        Line::styled("Add to client:", Style::default().fg(theme::MUTED)),
        Line::styled(
            format!("claude mcp add --transport http hss {url}"),
            Style::default().fg(theme::TEXT),
        ),
    ];
    let width = area.width.saturating_sub(2);
    let status = super::help::visual_lines(status, width);
    // Keep a log viewport even when the terminal cannot fit the full status.
    let status_height = (status.len().saturating_add(2).min(usize::from(u16::MAX)) as u16)
        .min(area.height.saturating_sub(1 + hotkeys.len() as u16 + 3));
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(status_height),
            Constraint::Min(0),
            Constraint::Length(hotkeys.len() as u16),
        ])
        .split(area);

    // Long client names and log paths belong in the scrollable view, not a
    // fixed status box where they could hide the setup command or stop warning.
    let mut activity = vec![
        Line::from(vec![
            Span::styled("Client: ", Style::default().fg(theme::MUTED)),
            Span::styled(
                log.client
                    .clone()
                    .unwrap_or_else(|| "waiting for connection…".into()),
                Style::default().fg(if log.client.is_some() {
                    theme::SUCCESS
                } else {
                    theme::MUTED
                }),
            ),
        ]),
        Line::styled(
            log.log_file
                .as_ref()
                .map(|p| format!("Log: {}", p.display()))
                .unwrap_or_else(|| "Log file: disabled".into()),
            Style::default().fg(theme::MUTED),
        ),
        Line::raw(""),
    ];
    for e in &log.entries {
        let (fg, marker) = match e.level {
            Level::Info => (theme::INFO, "ℹ"),
            Level::Req => (theme::INFO, "▶"),
            Level::Ok => (theme::SUCCESS, "✓"),
            Level::Err => (theme::ERROR, "✗"),
        };
        for (index, message) in e.msg.split('\n').enumerate() {
            let mut spans = Vec::new();
            if index == 0 {
                spans.extend([
                    Span::styled(
                        format!("[{:>6.2}s] ", e.at),
                        Style::default().fg(theme::MUTED),
                    ),
                    Span::styled(
                        format!("{marker} {:<4} ", e.level.label()),
                        Style::default().fg(fg).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{:<8} ", e.tag), Style::default().fg(theme::FOCUS)),
                ]);
            }
            spans.push(Span::styled(
                message.to_owned(),
                Style::default().fg(theme::TEXT),
            ));
            activity.push(Line::from(spans));
        }
        for detail in &e.detail {
            for line in detail.split('\n') {
                activity.push(Line::styled(
                    format!("           │ {line}"),
                    Style::default().fg(theme::MUTED),
                ));
            }
        }
        activity.push(Line::raw(""));
    }
    if log.entries.is_empty() {
        activity.push(Line::styled(
            "No activity yet — connect an MCP client to see calls here.",
            Style::default().fg(theme::MUTED),
        ));
    } else {
        // Do not end the scroll range on an entry separator alone.
        activity.pop();
    }
    let mut activity = super::help::visual_lines(activity, width);
    if status.len() > usize::from(chunks[1].height.saturating_sub(2)) {
        let mut overflow = status.clone();
        overflow.push(Line::raw(""));
        overflow.append(&mut activity);
        activity = overflow;
    }
    ServerView {
        areas: [chunks[0], chunks[1], chunks[2], chunks[3]],
        status,
        activity,
        hotkeys,
    }
}

fn draw_server(
    f: &mut Frame,
    url: &str,
    started: Instant,
    log: &Log,
    scroll_offset: usize,
    area: Rect,
) {
    let view = server_view(url, started, &log.lock().unwrap(), area);
    let scroll = scroll_offset.min(view.max_scroll());
    let height = usize::from(block(" Activity log ").inner(view.areas[2]).height);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "hss",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · MCP server", Style::default().fg(theme::MUTED)),
        ])),
        view.areas[0],
    );
    f.render_widget(
        Paragraph::new(view.status).block(block(" Server status ")),
        view.areas[1],
    );
    f.render_widget(
        Paragraph::new(
            view.activity
                .into_iter()
                .skip(scroll)
                .take(height)
                .collect::<Vec<_>>(),
        )
        .block(block(" Activity log ")),
        view.areas[2],
    );
    f.render_widget(Paragraph::new(view.hotkeys), view.areas[3]);
}

fn scroll_key(offset: &mut usize, key: KeyCode, max_scroll: usize) {
    *offset = (*offset).min(max_scroll);
    *offset = match key {
        KeyCode::Up | KeyCode::Char('k') => offset.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => offset.saturating_add(1),
        KeyCode::PageUp => offset.saturating_sub(10),
        KeyCode::PageDown => offset.saturating_add(10),
        KeyCode::Home => 0,
        KeyCode::End => max_scroll,
        _ => *offset,
    }
    .min(max_scroll);
}

pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if let Some(mcp) = &app.mcp {
        let size = terminal.size()?;
        let area = super::help::content_area(app, Rect::new(0, 0, size.width, size.height));
        let view = server_view(
            &mcp.server_url(),
            mcp.started,
            &mcp.log.lock().unwrap(),
            area,
        );
        scroll_key(&mut app.mcp_scroll_offset, key.code, view.max_scroll());
    } else {
        app.mcp_scroll_offset = 0;
    }
    if matches!(
        key.code,
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q')
    ) {
        if let Some(mcp) = app.mcp.take() {
            mcp.stop();
        }
        app.mcp_scroll_offset = 0;
        app.screen = Screen::Main;
        app.status_message = Some(crate::types::StatusMessage::info(
            "MCP stopped accepting requests; active calls continue.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::LogEntry;
    use ratatui::{backend::TestBackend, Terminal};
    use std::sync::{Arc, Mutex};

    fn text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn compact_mcp_render_keeps_core_status_and_complete_setup_command() {
        let log = Arc::new(Mutex::new(LogState {
            active_calls: 2,
            client: Some("long-client-name/".repeat(20)),
            log_file: Some(std::path::PathBuf::from(format!(
                "/{}mcp.log",
                "long-directory/".repeat(20)
            ))),
            ..Default::default()
        }));
        for (width, height) in [(60, 20), (80, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| {
                    draw_server(
                        f,
                        "http://127.0.0.1:8822",
                        Instant::now(),
                        &log,
                        0,
                        f.area(),
                    )
                })
                .unwrap();
            let rendered = text(&terminal);
            for expected in [
                "hss",
                "MCP server",
                "Running",
                "active 2",
                "Stop blocks new requests; active calls continue.",
                "claude mcp add --transport http hss",
                "Esc/Q",
                "stop & back",
            ] {
                assert!(
                    rendered.contains(expected),
                    "{width}x{height}: missing {expected}"
                );
            }
            let view = server_view(
                "http://127.0.0.1:8822",
                Instant::now(),
                &log.lock().unwrap(),
                Rect::new(0, 0, width, height),
            );
            let command: String = view.status[4..]
                .iter()
                .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
                .collect();
            assert_eq!(
                command,
                "claude mcp add --transport http hss http://127.0.0.1:8822"
            );
            assert_eq!(terminal.backend().buffer()[(0, 1)].fg, theme::MUTED);
        }
    }

    #[test]
    fn end_then_up_scrolls_immediately_and_long_log_tails_are_accessible() {
        let log = Arc::new(Mutex::new(LogState {
            entries: vec![LogEntry {
                at: 0.0,
                level: Level::Err,
                tag: "execute".into(),
                msg: format!("{}MESSAGE_TAIL", "你好 failure ".repeat(80)),
                detail: vec![format!("{}DETAIL_TAIL", "/long/path".repeat(80))],
            }],
            ..Default::default()
        }));
        for (width, height) in [(60, 20), (80, 24)] {
            let started = Instant::now();
            let view = server_view(
                "http://127.0.0.1:8822",
                started,
                &log.lock().unwrap(),
                Rect::new(0, 0, width, height),
            );
            assert!(view.activity.len() > 20);
            let max = view.max_scroll();
            let mut offset = usize::MAX;
            scroll_key(&mut offset, KeyCode::End, max);
            assert_eq!(offset, max);
            scroll_key(&mut offset, KeyCode::Up, max);
            assert_eq!(offset, max - 1);
            for _ in 0..100 {
                scroll_key(&mut offset, KeyCode::PageDown, max);
            }
            assert_eq!(offset, max);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|f| draw_server(f, "http://127.0.0.1:8822", started, &log, offset, f.area()))
                .unwrap();
            // A tail can straddle wrapped rows, so inspect without padding/borders.
            let rendered: String = text(&terminal)
                .chars()
                .filter(|c| !c.is_whitespace() && !"│─┌┐└┘".contains(*c))
                .collect();
            assert!(
                rendered.contains("DETAIL_TAIL"),
                "{width}x{height}: {rendered}"
            );
            scroll_key(&mut offset, KeyCode::Up, 2);
            assert_eq!(offset, 1, "stale offset must clamp before moving up");
        }
    }
}
