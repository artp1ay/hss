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

    // Log block: render newest entries that fit (each entry = 1 header + N detail lines)
    let log_block = Block::default()
        .title(" Activity ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner_height = chunks[1].height.saturating_sub(2) as usize;

    let mut lines: Vec<Line> = Vec::new();
    for e in log.entries.iter().rev() {
        let (fg, marker) = match e.level {
            Level::Info => (Color::Blue, "·"),
            Level::Req => (Color::Cyan, "→"),
            Level::Ok => (Color::Green, "✓"),
            Level::Err => (Color::Red, "✗"),
        };
        let mut block: Vec<Line> = vec![Line::from(vec![
            Span::styled(
                format!("{:>7.2}s ", e.at),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!("{marker} {} ", e.level.label()),
                Style::default().fg(fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<8}", e.tag),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(e.msg.clone(), Style::default().fg(Color::White)),
        ])];
        for d in &e.detail {
            block.push(Line::from(Span::styled(
                format!("{:>9} │ {d}", ""),
                Style::default().fg(Color::DarkGray),
            )));
        }
        if lines.len() + block.len() > inner_height {
            break;
        }
        block.extend(lines);
        lines = block;
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No activity yet — connect an MCP client to see calls here.",
            Style::default().fg(Color::DarkGray),
        )));
    }
    drop(log);
    f.render_widget(Paragraph::new(lines).block(log_block), chunks[1]);

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("[Esc/Q]", Style::default().fg(Color::Blue)),
        Span::styled(" stop server & back", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[2]);
}

pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            if let Some(mcp) = app.mcp.take() {
                mcp.stop();
            }
            app.screen = Screen::Main;
            app.status_message = Some(crate::types::StatusMessage::info("MCP server stopped."));
        }
        _ => {} // modal: everything else is unavailable while the server runs
    }
    Ok(())
}
