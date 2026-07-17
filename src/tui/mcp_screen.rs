use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::mcp::McpServer;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let Some(ref mcp) = app.mcp else { return };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),  // status
            Constraint::Min(0),     // log
            Constraint::Length(1),  // hotkeys
        ])
        .split(f.area());

    // Status block
    let status_block = Block::default()
        .title(" MCP Server ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));
    let status_lines = vec![
        Line::from(vec![
            Span::styled("● Running  ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(McpServer::url(), Style::default().fg(Color::White)),
            Span::styled("  (streamable HTTP)", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("Tools: ", Style::default().fg(Color::DarkGray)),
            Span::styled("list-servers, execute-command", Style::default().fg(Color::Gray)),
        ]),
        Line::from(Span::styled(
            format!("Add to client:  claude mcp add --transport http hss {}", McpServer::url()),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(status_lines).block(status_block), chunks[0]);

    // Log block: show the last lines that fit
    let log_block = Block::default()
        .title(" Log ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner_height = chunks[1].height.saturating_sub(2) as usize;
    let log = mcp.log.lock().unwrap();
    let start = log.len().saturating_sub(inner_height);
    let lines: Vec<Line> = log[start..].iter()
        .map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(Color::Gray))))
        .collect();
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
            app.status_message = Some("MCP server stopped.".into());
        }
        _ => {} // modal: everything else is unavailable while the server runs
    }
    Ok(())
}
