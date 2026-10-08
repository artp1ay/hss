use crate::mcp::Level;
use crate::tui::{App, Screen, Term};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let Some(ref mcp) = app.mcp else { return };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // status
            Constraint::Min(0),    // log
            Constraint::Length(1), // hotkeys
        ])
        .split(f.area());

    // Status block
    let status_block = Block::default()
        .title(" MCP Server ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));
    let log = mcp.log.lock().unwrap();
    let uptime = mcp.started.elapsed().as_secs();
    let status_lines = vec![
        Line::from(vec![
            Span::styled(
                "● Running  ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(mcp.server_url(), Style::default().fg(Color::White)),
            Span::styled(
                format!("  up {}m{:02}s", uptime / 60, uptime % 60),
                Style::default().fg(Color::DarkGray),
            ),
            if let Some(ref path) = log.log_file {
                Span::styled(
                    format!("  log: {}", path.display()),
                    Style::default().fg(Color::Cyan),
                )
            } else {
                Span::raw("")
            },
        ]),
        Line::from(vec![
            Span::styled("Client: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                log.client
                    .clone()
                    .unwrap_or_else(|| "waiting for connection…".into()),
                Style::default().fg(if log.client.is_some() {
                    Color::Green
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled("   requests ", Style::default().fg(Color::DarkGray)),
            Span::styled(log.requests.to_string(), Style::default().fg(Color::White)),
            Span::styled("   tool calls ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                log.tool_calls.to_string(),
                Style::default().fg(Color::White),
            ),
            Span::styled("   errors ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                log.errors.to_string(),
                Style::default().fg(if log.errors > 0 {
                    Color::Red
                } else {
                    Color::White
                }),
            ),
        ]),
        Line::from(Span::styled(
            format!(
                "Add to client:  claude mcp add --transport http hss {}",
                mcp.server_url()
            ),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(status_lines).block(status_block), chunks[0]);

    // Log block: structured clear view with scrolling support
    let log_block = Block::default()
        .title(" Activity Log (↑/↓/PageUp/PageDown to scroll) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner_height = chunks[1].height.saturating_sub(2) as usize;

    let mut all_rendered_lines: Vec<Line> = Vec::new();
    for e in log.entries.iter() {
        let (fg, marker) = match e.level {
            Level::Info => (Color::Blue, "ℹ"),
            Level::Req => (Color::Cyan, "▶"),
            Level::Ok => (Color::Green, "✓"),
            Level::Err => (Color::Red, "✗"),
        };

        all_rendered_lines.push(Line::from(vec![
            Span::styled(
                format!("[{:>6.2}s] ", e.at),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{marker} {:<4} ", e.level.label()),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<8} ", e.tag), Style::default().fg(Color::Yellow)),
            Span::styled(
                e.msg.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));

        for d in &e.detail {
            all_rendered_lines.push(Line::from(vec![
                Span::styled("           │ ", Style::default().fg(Color::DarkGray)),
                Span::styled(d.clone(), Style::default().fg(Color::Gray)),
            ]));
        }
        all_rendered_lines.push(Line::from(""));
    }

    if all_rendered_lines.is_empty() {
        all_rendered_lines.push(Line::from(Span::styled(
            "No activity yet — connect an MCP client to see calls here.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let total_lines = all_rendered_lines.len();
    let max_scroll = total_lines.saturating_sub(inner_height);
    let scroll = app.mcp_scroll_offset.min(max_scroll);

    let visible_lines: Vec<Line> = all_rendered_lines
        .into_iter()
        .skip(scroll)
        .take(inner_height)
        .collect();

    drop(log);
    f.render_widget(Paragraph::new(visible_lines).block(log_block), chunks[1]);

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("[↑/↓/PgUp/PgDn]", Style::default().fg(Color::Blue)),
        Span::styled(" scroll  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[End]", Style::default().fg(Color::Blue)),
        Span::styled(" bottom  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Home]", Style::default().fg(Color::Blue)),
        Span::styled(" top  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc/Q]", Style::default().fg(Color::Blue)),
        Span::styled(" stop server & back", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[2]);
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.mcp_scroll_offset = app.mcp_scroll_offset.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.mcp_scroll_offset = app.mcp_scroll_offset.saturating_add(1);
        }
        KeyCode::PageUp => {
            app.mcp_scroll_offset = app.mcp_scroll_offset.saturating_sub(10);
        }
        KeyCode::PageDown => {
            app.mcp_scroll_offset = app.mcp_scroll_offset.saturating_add(10);
        }
        KeyCode::Home => {
            app.mcp_scroll_offset = 0;
        }
        KeyCode::End => {
            app.mcp_scroll_offset = usize::MAX / 2;
        }
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            if let Some(mcp) = app.mcp.take() {
                mcp.stop();
            }
            app.mcp_scroll_offset = 0;
            app.screen = Screen::Main;
            app.status_message = Some(crate::types::StatusMessage::info("MCP server stopped."));
        }
        _ => {}
    }
    Ok(())
}
