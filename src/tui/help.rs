use super::{theme, App, Screen};
use std::cell::Cell;
use unicode_width::UnicodeWidthStr;

thread_local! {
    // Global help key handling has no terminal argument; use the last drawn viewport.
    static VIEWPORT: Cell<Rect> = const { Cell::new(Rect::new(0, 0, 0, 0)) };
}
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

const HELP: &[&str] = &[
    "HOSTS",
    "↑/↓ or j/k select · PgUp/PgDn page · Home/End first/last",
    "Enter connect · R connect as another credential",
    "/ search names, groups, addresses, tags, descriptions",
    "Search: Esc clears text; Esc again leaves search · Enter leaves search",
    "N add · E edit · D delete · P copy public keys",
    "I import/export · C credentials · S settings · A audit · M MCP · Q quit",
    "",
    "FORMS",
    "Tab/Shift+Tab next/previous field · ←/→ move cursor · Home/End",
    "Ctrl+A/Ctrl+E start/end · Ctrl+U remove text before cursor",
    "Space toggles only a focused option · Enter save · Esc back",
    "Settings: Esc with changes offers Save / Discard / Continue",
    "Credential type changes require confirmation; empty existing password keeps it",
    "Delete: Y confirms · Enter/N/Esc cancel; host and credential skip are separate",
    "Bastions with dependent hosts cannot be deleted until routes are reassigned",
    "Export: existing files require confirmation; cancelling leaves them untouched",
    "",
    "COPY ID",
    "Space select key · Tab select user/password · Enter start",
    "While copying: Esc back (continues) · P reopen · Ctrl+C stop from Copy ID",
    "Installed keys cannot be undone; an interrupted key may already be installed",
    "Quitting cancels the local Copy ID process tree",
    "",
    "AUDIT / MCP",
    "Audit: R rerun all · Enter/Space retry selected · C connect · Esc/Q back",
    "An active audit cannot be replaced by a second run",
    "MCP Stop blocks new requests; active calls continue",
    "Read-only is a best-effort command filter, not a security sandbox",
    "Logs always redact detected PEM blocks, even when key output is enabled",
    "",
    "NOTIFICATIONS",
    "Errors and warnings remain until replaced or dismissed with Ctrl+L",
    "Success/info clear after 4 seconds · F1 shows the full current notification",
    "F1 help anywhere · ? help in host list (not while editing search)",
];

fn status_height(app: &App, area: Rect) -> u16 {
    if app.status_message.is_some() {
        area.height.min(4)
    } else {
        0
    }
}

pub fn content_area(app: &App, area: Rect) -> Rect {
    let height = status_height(app, area);
    Rect::new(
        area.x,
        area.y + height,
        area.width,
        area.height.saturating_sub(height),
    )
}

pub fn draw_status(f: &mut Frame, app: &App) {
    let Some(message) = &app.status_message else {
        return;
    };
    let (label, color) = match message.kind {
        crate::types::StatusKind::Success => ("OK", theme::SUCCESS),
        crate::types::StatusKind::Info => ("INFO", theme::INFO),
        crate::types::StatusKind::Warning => ("WARNING", theme::WARNING),
        crate::types::StatusKind::Error => ("ERROR", theme::ERROR),
    };
    let area = Rect::new(
        f.area().x,
        f.area().y,
        f.area().width,
        status_height(app, f.area()),
    );
    f.render_widget(
        Paragraph::new(message.text.as_str())
            .style(Style::default().fg(color))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .title(format!(" {label} · Ctrl+L dismiss · F1 full text ")),
            ),
        area,
    );
}

fn screen_error(app: &App) -> Option<&str> {
    match app.screen {
        Screen::Settings => app.settings_error.as_deref(),
        Screen::HostForm => app.host_form.as_ref()?.error_message.as_deref(),
        Screen::Credentials => app.cred_form.as_ref()?.error_message.as_deref(),
        Screen::CopyId => app.copy_id_form.as_ref()?.error_message.as_deref(),
        Screen::ImportHosts => app.import_error.as_deref(),
        _ => None,
    }
}

// Wrap once at grapheme boundaries, then render and scroll these exact visual rows.
// This also keeps unbroken commands/paths and wide Unicode characters accessible.
pub(super) fn visual_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    for line in lines {
        let mut spans = Vec::new();
        let mut used = 0;
        for grapheme in line.styled_graphemes(Style::default()) {
            let cells = grapheme.symbol.width();
            if used + cells > usize::from(width) && !spans.is_empty() {
                rows.push(Line::from(std::mem::take(&mut spans)));
                used = 0;
            }
            spans.push(Span::styled(grapheme.symbol.to_owned(), grapheme.style));
            used += cells;
        }
        rows.push(Line::from(spans));
    }
    rows
}

fn lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some(error) = screen_error(app) {
        lines.push(Line::styled(
            "CURRENT SCREEN ERROR",
            Style::default()
                .fg(theme::ERROR)
                .add_modifier(Modifier::BOLD),
        ));
        lines.extend(error.lines().map(|line| Line::raw(line.to_owned())));
        lines.push(Line::raw(""));
    }
    if let Some(message) = &app.status_message {
        lines.push(Line::styled(
            "CURRENT NOTIFICATION",
            Style::default()
                .fg(theme::FOCUS)
                .add_modifier(Modifier::BOLD),
        ));
        // Preserve arbitrary line breaks so all error details can be inspected.
        lines.extend(message.text.lines().map(|line| Line::raw(line.to_owned())));
        lines.push(Line::raw(""));
    }
    lines.extend(HELP.iter().map(|text| {
        let heading = !text.is_empty() && !text.chars().any(char::is_lowercase);
        Line::from(Span::styled(
            *text,
            Style::default().fg(if heading { theme::FOCUS } else { theme::TEXT }),
        ))
    }));
    lines
}

fn help_block() -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::MUTED))
        .title(Line::from(vec![
            Span::styled(
                " hss",
                Style::default()
                    .fg(theme::TEXT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · keyboard help ", Style::default().fg(theme::MUTED)),
        ]))
        .title_bottom(" ↑↓ / PgUp/PgDn / Home/End · Esc/F1/? close ")
}

fn max_scroll(app: &App, inner: Rect) -> u16 {
    visual_lines(lines(app), inner.width)
        .len()
        .saturating_sub(usize::from(inner.height.max(1)))
        .min(usize::from(u16::MAX)) as u16
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = super::adaptive_centered_rect(90, 90, 60, 20, f.area());
    let block = help_block();
    let inner = block.inner(area);
    VIEWPORT.set(inner);
    let rows = visual_lines(lines(app), inner.width);
    let max_scroll = rows.len().saturating_sub(usize::from(inner.height.max(1)));
    let scroll = usize::from(app.help_scroll).min(max_scroll);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(
            rows.into_iter()
                .skip(scroll)
                .take(usize::from(inner.height))
                .collect::<Vec<_>>(),
        )
        .style(Style::default().fg(theme::TEXT))
        .block(block),
        area,
    );
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    let max_scroll = max_scroll(app, VIEWPORT.get());
    app.help_scroll = app.help_scroll.min(max_scroll);
    match key.code {
        KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => app.help_open = false,
        KeyCode::Up | KeyCode::Char('k') => app.help_scroll = app.help_scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => app.help_scroll = app.help_scroll.saturating_add(1),
        KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
        KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(10),
        KeyCode::Home => app.help_scroll = 0,
        KeyCode::End => app.help_scroll = max_scroll,
        _ => {}
    }
    app.help_scroll = app.help_scroll.min(max_scroll);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn notification_is_reserved_without_covering_the_screen_and_help_wraps() {
        let mut app = App::new(vec![], vec![], crate::config::AppConfig::default(), vec![]);
        app.status_message = Some(crate::types::StatusMessage::error(
            "Ошибка 你好: a long notification that stays visible until explicitly dismissed",
        ));
        for (width, height) in [(60, 20), (80, 24), (100, 30), (160, 40), (20, 8)] {
            let area = content_area(&app, Rect::new(0, 0, width, height));
            assert_eq!(area.y, 4);
            assert_eq!(area.height, height - 4);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| super::super::draw(f, &app)).unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("ERROR"));
            app.help_open = true;
            terminal.draw(|f| draw(f, &app)).unwrap();
            app.help_open = false;
        }
    }
}
