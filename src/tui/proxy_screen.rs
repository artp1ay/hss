use crate::tui::{App, Screen, Term};
use crate::types::Host;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

pub fn has_proxy_config(host: &Host) -> bool {
    host.socks_port.is_some() || host.http_proxy_port.is_some() || !host.port_forwards.is_empty()
}

pub fn draw(f: &mut Frame, app: &App) {
    let Screen::Proxy { host_idx } = &app.screen else {
        return;
    };
    draw_for_host(f, app, *host_idx);
}

pub fn draw_for_host(f: &mut Frame, app: &App, host_idx: usize) {
    let Some(host) = app.hosts.get(host_idx) else {
        return;
    };

    let active_here = app.proxy.as_ref().filter(|p| p.host_id == host.id);
    let starting_here = app
        .proxy_starting
        .as_ref()
        .filter(|s| s.summary.host_id == host.id);
    let error_here = app
        .proxy_last_error
        .as_ref()
        .filter(|e| e.host_id == host.id);
    let active_elsewhere = app
        .proxy
        .as_ref()
        .filter(|p| p.host_id != host.id)
        .map(|p| p.summary.host_name.as_str())
        .or_else(|| {
            app.proxy_starting
                .as_ref()
                .filter(|s| s.summary.host_id != host.id)
                .map(|s| s.summary.host_name.as_str())
        });

    let configured = has_proxy_config(host);

    // Prepare hotkeys first so bottom bar height adapts cleanly to terminal width
    let primary_label = if active_here.is_some() {
        "stop"
    } else if starting_here.is_some() {
        "cancel"
    } else if active_elsewhere.is_some() {
        "switch here"
    } else {
        "start"
    };
    let close_label = if app.proxy.is_some() || app.proxy_starting.is_some() {
        "back (keeps running)"
    } else {
        "back"
    };

    let mut pairs: Vec<(&str, &str)> = Vec::new();
    if configured || active_here.is_some() || starting_here.is_some() {
        pairs.push(("Enter/Space", primary_label));
    }
    if active_here.is_some() {
        pairs.push(("T", "test tunnel"));
    }
    if active_elsewhere.is_some() {
        pairs.push(("S", "stop active"));
    }
    pairs.push(("R", "cred"));
    pairs.push(("E", "ports (F2)"));
    pairs.push(("C", "clear"));
    pairs.push(("Esc/Q", close_label));

    let area = f.area();
    let hotkey_lines = crate::tui::wrap_hotkey_lines(&pairs, area.width);
    let hotkey_height = hotkey_lines.len().max(1) as u16;

    // Full-screen MCP-style layout: compact 5-row header + large activity log + hotkeys
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(hotkey_height),
        ])
        .split(area);

    // 1. Compact Status & Endpoints Card
    let border_color = if active_here.is_some() {
        Color::Green
    } else if starting_here.is_some() {
        Color::Yellow
    } else if error_here.is_some() {
        Color::Red
    } else if configured {
        Color::Blue
    } else {
        Color::DarkGray
    };

    let record = app.record_for(&host.id);
    let resolved_cred = crate::ssh::resolve_credential(
        &app.credentials,
        &app.config,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )
    .ok()
    .flatten()
    .map(|c| format!("{} ({})", c.name, c.username))
    .unwrap_or_else(|| "none ([R] to pick)".into());

    let jump_text = host
        .jump_host_id
        .as_deref()
        .and_then(|jid| app.hosts.iter().find(|h| h.id == jid))
        .map(|jh| {
            let jc = crate::ssh::resolve_jump_auth(
                &app.hosts,
                host,
                &app.credentials,
                &app.server_records,
            )
            .ok()
            .flatten()
            .and_then(|ja| ja.cred)
            .map(|c| format!(" ({})", c.name))
            .unwrap_or_default();
            format!("{}{jc}", jh.name)
        });

    // Row 1: State badge + target + uptime/error + auth + jump
    let mut row1 = Vec::new();
    if let Some(proxy) = active_here {
        let up = proxy.started_at.elapsed().as_secs();
        row1.push(Span::styled(
            "● Active  ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
        row1.push(Span::styled(
            format!("{}:{}", host.ip, host.port),
            Style::default().fg(Color::White),
        ));
        row1.push(Span::styled(
            format!("  up {}m{:02}s", up / 60, up % 60),
            Style::default().fg(Color::DarkGray),
        ));
        row1.push(Span::styled("   cred ", Style::default().fg(Color::DarkGray)));
        row1.push(Span::styled(
            resolved_cred.clone(),
            Style::default().fg(Color::Cyan),
        ));
        if let Some(ref jump) = jump_text {
            row1.push(Span::styled("   via ", Style::default().fg(Color::DarkGray)));
            row1.push(Span::styled(jump.clone(), Style::default().fg(Color::Yellow)));
        }
    } else if let Some(starting) = starting_here {
        let elapsed = starting.started_at.elapsed().as_secs();
        row1.push(Span::styled(
            "◌ Starting…  ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        row1.push(Span::styled(
            format!("{}:{}", host.ip, host.port),
            Style::default().fg(Color::White),
        ));
        row1.push(Span::styled(
            format!("  connecting ({elapsed}s)"),
            Style::default().fg(Color::DarkGray),
        ));
        row1.push(Span::styled("   cred ", Style::default().fg(Color::DarkGray)));
        row1.push(Span::styled(
            resolved_cred.clone(),
            Style::default().fg(Color::Cyan),
        ));
        if let Some(ref jump) = jump_text {
            row1.push(Span::styled("   via ", Style::default().fg(Color::DarkGray)));
            row1.push(Span::styled(jump.clone(), Style::default().fg(Color::Yellow)));
        }
    } else if let Some(err) = error_here {
        row1.push(Span::styled(
            "✗ Failed  ",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
        row1.push(Span::styled(
            format!("{}:{}", host.ip, host.port),
            Style::default().fg(Color::White),
        ));
        row1.push(Span::styled(
            format!("  {}", err.message),
            Style::default().fg(Color::Red),
        ));
    } else {
        row1.push(Span::styled(
            "○ Stopped  ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ));
        row1.push(Span::styled(
            format!("{}:{}", host.ip, host.port),
            Style::default().fg(Color::White),
        ));
        row1.push(Span::styled("   cred ", Style::default().fg(Color::DarkGray)));
        row1.push(Span::styled(
            resolved_cred.clone(),
            Style::default().fg(Color::Cyan),
        ));
        if let Some(ref jump) = jump_text {
            row1.push(Span::styled("   via ", Style::default().fg(Color::DarkGray)));
            row1.push(Span::styled(jump.clone(), Style::default().fg(Color::Yellow)));
        }
    }

    // Row 2: Live endpoints & HTTP traffic counters on one dense line
    let ep_style = if active_here.is_some() {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if starting_here.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    };

    let row2 = if !configured {
        Line::from(vec![
            Span::styled(
                "No SOCKS5, HTTP proxy, or port forwards configured — press ",
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                "[E]",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " to open [F2] Ports & Proxy.",
                Style::default().fg(Color::White),
            ),
        ])
    } else {
        let mut spans: Vec<Span> = Vec::new();
        let push_sep = |s: &mut Vec<Span>| {
            if !s.is_empty() {
                s.push(Span::styled("   ", Style::default().fg(Color::DarkGray)));
            }
        };

        if let Some(socks_port) = host.socks_port {
            push_sep(&mut spans);
            spans.push(Span::styled("SOCKS5 ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("socks5h://127.0.0.1:{socks_port}"),
                ep_style,
            ));
        } else if let Some(internal) = active_here
            .and_then(|p| p.summary.internal_socks_port)
            .or_else(|| starting_here.and_then(|s| s.summary.internal_socks_port))
        {
            push_sep(&mut spans);
            spans.push(Span::styled(
                format!("SOCKS5(int) 127.0.0.1:{internal}"),
                Style::default().fg(Color::DarkGray),
            ));
        }

        if let Some(http_port) = host.http_proxy_port {
            let actual_port = active_here
                .and_then(crate::tui::ProxySession::http_bound_port)
                .unwrap_or(http_port);
            push_sep(&mut spans);
            spans.push(Span::styled("HTTP ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(
                format!("http://127.0.0.1:{actual_port}"),
                ep_style,
            ));
            if let Some(metrics) = active_here.and_then(crate::tui::ProxySession::http_metrics) {
                spans.push(Span::styled("  reqs ", Style::default().fg(Color::DarkGray)));
                spans.push(Span::styled(
                    metrics.total_requests.to_string(),
                    Style::default().fg(Color::White),
                ));
                spans.push(Span::styled(
                    "  active ",
                    Style::default().fg(Color::DarkGray),
                ));
                spans.push(Span::styled(
                    metrics.active_conns.to_string(),
                    Style::default().fg(if metrics.active_conns > 0 {
                        Color::Cyan
                    } else {
                        Color::White
                    }),
                ));
            }
        }

        if !host.port_forwards.is_empty() {
            let formatted: Vec<String> = host
                .port_forwards
                .iter()
                .map(|fwd| {
                    fwd.split_once(':')
                        .map(|(l, r)| format!("127.0.0.1:{l}→{r}"))
                        .unwrap_or_else(|| fwd.clone())
                })
                .collect();
            push_sep(&mut spans);
            spans.push(Span::styled("FWD ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(formatted.join(", "), ep_style));
        }

        Line::from(spans)
    };

    // Row 3: Contextual warning / last error / copyable curl test command
    let http_err = active_here
        .and_then(crate::tui::ProxySession::http_metrics)
        .and_then(|m| m.last_error);

    let row3 = if let Some(other) = active_elsewhere {
        Line::from(Span::styled(
            format!("! Active proxy on '{other}' — [Enter] switches here, [S] stops it"),
            Style::default().fg(Color::Yellow),
        ))
    } else if let Some(err) = http_err {
        Line::from(vec![
            Span::styled("Last HTTP error: ", Style::default().fg(Color::Yellow)),
            Span::styled(err, Style::default().fg(Color::Red)),
        ])
    } else if error_here.is_some() {
        let mut spans = vec![
            Span::styled("Cred: ", Style::default().fg(Color::DarkGray)),
            Span::styled(resolved_cred, Style::default().fg(Color::Cyan)),
        ];
        if let Some(jump) = jump_text {
            spans.push(Span::styled("   via ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(jump, Style::default().fg(Color::Yellow)));
        }
        spans.push(Span::styled(
            "   [Enter] retry  [R] switch cred  [E] ports",
            Style::default().fg(Color::DarkGray),
        ));
        Line::from(spans)
    } else if configured {
        let try_cmd = match (host.http_proxy_port, host.socks_port) {
            (Some(hp), Some(sp)) => {
                let actual_hp = active_here
                    .and_then(crate::tui::ProxySession::http_bound_port)
                    .unwrap_or(hp);
                format!(
                    "Try:  curl -x http://127.0.0.1:{actual_hp} https://ifconfig.me   |   curl -x socks5h://127.0.0.1:{sp} https://ifconfig.me"
                )
            }
            (Some(hp), None) => {
                let actual_hp = active_here
                    .and_then(crate::tui::ProxySession::http_bound_port)
                    .unwrap_or(hp);
                format!("Try:  curl -x http://127.0.0.1:{actual_hp} https://ifconfig.me")
            }
            (None, Some(sp)) => {
                format!("Try:  curl -x socks5h://127.0.0.1:{sp} https://ifconfig.me")
            }
            (None, None) => {
                let first = host.port_forwards.first().map(String::as_str).unwrap_or("");
                let (l, r) = first.split_once(':').unwrap_or((first, ""));
                format!("Connect local client to 127.0.0.1:{l} -> remote:{r}")
            }
        };
        Line::from(Span::styled(try_cmd, Style::default().fg(Color::DarkGray)))
    } else {
        Line::from(Span::styled(
            "Listeners bind 127.0.0.1 only and stay alive in the background while hss is open.",
            Style::default().fg(Color::DarkGray),
        ))
    };

    let status_block = Block::default()
        .title(format!(" Proxy & Tunnels — {} ", host.name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    f.render_widget(
        Paragraph::new(vec![Line::from(row1), row2, row3]).block(status_block),
        chunks[0],
    );

    // 2. Full-width Activity & SSH Debug Log (takes ~80% of terminal height)
    let log_lines: Vec<String> = if let Some(proxy) = active_here {
        proxy.log.lock().map(|g| g.snapshot()).unwrap_or_default()
    } else if let Some(starting) = starting_here {
        starting
            .log
            .lock()
            .map(|g| g.snapshot())
            .unwrap_or_default()
    } else if let Some(err) = error_here {
        err.stderr_lines.clone()
    } else {
        Vec::new()
    };

    let log_inner_h = chunks[1].height.saturating_sub(2) as usize;
    let max_scroll = log_lines.len().saturating_sub(log_inner_h);
    let is_live = app.proxy_log_scroll == usize::MAX || app.proxy_log_scroll >= max_scroll;
    let scroll = if app.proxy_log_scroll == usize::MAX {
        max_scroll
    } else {
        app.proxy_log_scroll.min(max_scroll)
    };

    let log_title = if log_lines.is_empty() {
        " Activity Log ".to_string()
    } else if is_live {
        format!(
            " Activity Log ({} lines · LIVE · ↑/↓/PgUp/PgDn/Home/End) ",
            log_lines.len()
        )
    } else {
        format!(
            " Activity Log ({}/{} · paused · End=LIVE) ",
            scroll + 1,
            log_lines.len()
        )
    };

    let log_block = Block::default()
        .title(log_title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let rendered_log: Vec<Line> = if log_lines.is_empty() {
        vec![Line::from(Span::styled(
            if active_here.is_some() {
                "Tunnel is active — press [T] to run an end-to-end proxy test."
            } else if starting_here.is_some() {
                "Waiting for SSH handshake and local port binding…"
            } else if configured {
                "No activity yet — press [Enter] to start proxy."
            } else {
                "No proxy ports configured — press [E] to set up SOCKS5, HTTP proxy, or port forwards."
            },
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        log_lines
            .iter()
            .skip(scroll)
            .take(log_inner_h.max(1))
            .map(|line| format_log_line(line))
            .collect()
    };
    f.render_widget(Paragraph::new(rendered_log).block(log_block), chunks[1]);

    // 3. Bottom Hotkeys
    f.render_widget(Paragraph::new(hotkey_lines), chunks[2]);
}

/// Parses `[HH:MM:SS] [tag] body` emitted by `ProxyStderrLog`, tolerating plain stderr lines.
fn parse_log_line(raw: &str) -> (Option<&str>, Option<&str>, &str) {
    let mut rest = raw;
    let mut ts = None;

    if let Some(after_open) = rest.strip_prefix('[') {
        if let Some((candidate, after_close)) = after_open.split_once("] ") {
            let b = candidate.as_bytes();
            if b.len() == 8 && b[2] == b':' && b[5] == b':' {
                ts = Some(candidate);
                rest = after_close;
            }
        }
    }

    let mut tag = None;
    if let Some(after_open) = rest.strip_prefix('[') {
        if let Some((candidate, after_close)) = after_open.split_once("] ") {
            if !candidate.is_empty()
                && candidate.len() <= 8
                && candidate.bytes().all(|c| c.is_ascii_alphanumeric())
            {
                tag = Some(candidate);
                rest = after_close;
            }
        }
    }

    (ts, tag, rest)
}

fn format_log_line(raw: &str) -> Line<'static> {
    let (ts, tag, body) = parse_log_line(raw);
    let lower = body.to_lowercase();

    let is_err = lower.contains("failed")
        || lower.contains("denied")
        || lower.contains("error")
        || lower.contains("cannot")
        || lower.contains("refused")
        || lower.contains("prohibited")
        || lower.contains("timed out");
    let is_ok = !is_err
        && (lower.starts_with("ok ")
            || lower.contains("tunnel ready")
            || lower.contains("authenticated to")
            || lower.starts_with("connected")
            || lower.contains("socks5 listening")
            || lower.contains("http proxy listening"));
    let is_warn = !is_err
        && !is_ok
        && (lower.contains("warning") || lower.contains("permanently added"));

    let (marker, badge_color, body_color) = if is_err {
        ("✗", Color::Red, Color::Red)
    } else if is_ok {
        ("✓", Color::Green, Color::Green)
    } else if is_warn {
        ("!", Color::Yellow, Color::Yellow)
    } else {
        match tag {
            Some("http" | "socks" | "test") => ("▶", Color::Cyan, Color::White),
            Some("proxy") => ("ℹ", Color::Blue, Color::White),
            _ => ("·", Color::DarkGray, Color::Gray),
        }
    };

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(3);
    if let Some(ts) = ts {
        spans.push(Span::styled(
            format!("[{ts}] "),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let tag_label = tag.unwrap_or("log");
    let badge_style = if tag == Some("ssh") && !is_err && !is_ok && !is_warn {
        Style::default().fg(badge_color)
    } else {
        Style::default()
            .fg(badge_color)
            .add_modifier(Modifier::BOLD)
    };
    spans.push(Span::styled(
        format!("{marker} {tag_label:<5} "),
        badge_style,
    ));
    spans.push(Span::styled(
        body.to_string(),
        Style::default().fg(body_color),
    ));

    Line::from(spans)
}

pub fn handle_key(
    terminal: &mut Term,
    app: &mut App,
    key: KeyEvent,
    host_idx: usize,
) -> Result<()> {
    let Some(host) = app.hosts.get(host_idx).cloned() else {
        app.screen = Screen::Main;
        return Ok(());
    };

    match key.code {
        KeyCode::Esc
        | KeyCode::Char('q')
        | KeyCode::Char('Q')
        | KeyCode::Char('x')
        | KeyCode::Char('X') => {
            app.screen = Screen::Main;
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if app.is_proxy_active_or_starting_for(&host.id) {
                crate::tui::stop_proxy(app);
            } else if !has_proxy_config(&host) {
                open_proxy_config_editor(app, host_idx, &host);
            } else if let Err(e) = crate::tui::start_proxy(app, &host) {
                let err_msg = format!("{e:#}");
                app.proxy_last_error = Some(crate::tui::ProxyErrorReport {
                    host_id: host.id.clone(),
                    message: err_msg.clone(),
                    stderr_lines: Vec::new(),
                });
                app.status_message = Some(crate::types::StatusMessage::error(err_msg));
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if app.proxy.is_some() || app.proxy_starting.is_some() {
                crate::tui::stop_proxy(app);
            }
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            open_proxy_config_editor(app, host_idx, &host);
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.popup_selected = app.picker_selection(&host.id);
            app.screen = Screen::CredentialPicker {
                host_idx,
                after_failure: false,
                return_to_proxy: true,
            };
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            if let Some(ref p) = app.proxy {
                if let Ok(mut g) = p.log.lock() {
                    g.clear();
                }
            } else if let Some(ref s) = app.proxy_starting {
                if let Ok(mut g) = s.log.lock() {
                    g.clear();
                }
            } else if let Some(ref mut e) = app.proxy_last_error {
                e.stderr_lines.clear();
            }
            app.proxy_log_scroll = usize::MAX;
        }
        KeyCode::Char('t') | KeyCode::Char('T') => {
            if let Some(ref proxy) = app.proxy {
                if proxy.host_id == host.id {
                    run_proxy_self_test(proxy);
                    app.proxy_log_scroll = usize::MAX;
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let max_scroll = current_max_scroll(terminal, app, &host.id);
            let cur = if app.proxy_log_scroll == usize::MAX {
                max_scroll
            } else {
                app.proxy_log_scroll.min(max_scroll)
            };
            app.proxy_log_scroll = cur.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.proxy_log_scroll != usize::MAX {
                let max_scroll = current_max_scroll(terminal, app, &host.id);
                let next = app.proxy_log_scroll.saturating_add(1);
                app.proxy_log_scroll = if next >= max_scroll { usize::MAX } else { next };
            }
        }
        KeyCode::PageUp => {
            let visible = log_viewport_rows(terminal);
            let max_scroll = current_log_len(app, &host.id).saturating_sub(visible);
            let step = visible.saturating_sub(2).max(1);
            let cur = if app.proxy_log_scroll == usize::MAX {
                max_scroll
            } else {
                app.proxy_log_scroll.min(max_scroll)
            };
            app.proxy_log_scroll = cur.saturating_sub(step);
        }
        KeyCode::PageDown => {
            if app.proxy_log_scroll != usize::MAX {
                let visible = log_viewport_rows(terminal);
                let max_scroll = current_log_len(app, &host.id).saturating_sub(visible);
                let step = visible.saturating_sub(2).max(1);
                let next = app.proxy_log_scroll.saturating_add(step);
                app.proxy_log_scroll = if next >= max_scroll { usize::MAX } else { next };
            }
        }
        KeyCode::Home => {
            app.proxy_log_scroll = 0;
        }
        KeyCode::End => {
            app.proxy_log_scroll = usize::MAX;
        }
        _ => {}
    }
    Ok(())
}

fn log_viewport_rows(terminal: &Term) -> usize {
    // 5 rows status card + 1 row hotkeys + 2 rows log border = 8
    terminal
        .size()
        .map(|s| s.height.saturating_sub(8) as usize)
        .unwrap_or(16)
        .max(1)
}

fn current_max_scroll(terminal: &Term, app: &App, host_id: &str) -> usize {
    let total = current_log_len(app, host_id);
    total.saturating_sub(log_viewport_rows(terminal))
}

fn current_log_len(app: &App, host_id: &str) -> usize {
    if let Some(ref p) = app.proxy {
        if p.host_id == host_id {
            return p.log.lock().map(|g| g.snapshot().len()).unwrap_or(0);
        }
    }
    if let Some(ref s) = app.proxy_starting {
        if s.summary.host_id == host_id {
            return s.log.lock().map(|g| g.snapshot().len()).unwrap_or(0);
        }
    }
    if let Some(ref e) = app.proxy_last_error {
        if e.host_id == host_id {
            return e.stderr_lines.len();
        }
    }
    0
}

fn run_proxy_self_test(proxy: &crate::tui::ProxySession) {
    let log = std::sync::Arc::clone(&proxy.log);
    let socks_port = proxy.summary.effective_socks_port();
    let http_port = proxy.http_bound_port().or(proxy.summary.http_proxy_port);
    let forwards = proxy.summary.port_forwards.clone();
    std::thread::spawn(move || {
        if let Some(sp) = socks_port {
            if let Ok(mut g) = log.lock() {
                g.push_event(
                    "test",
                    format!("Testing SOCKS5 127.0.0.1:{sp} -> external 1.1.1.1:443 via remote host…"),
                );
            }
            let t0 = std::time::Instant::now();
            match crate::tui::http_proxy::test_socks_connect(sp, "1.1.1.1", 443) {
                Ok(()) => {
                    if let Ok(mut g) = log.lock() {
                        g.push_event(
                            "test",
                            format!("OK SOCKS5 -> 1.1.1.1:443 ({} ms)", t0.elapsed().as_millis()),
                        );
                    }
                }
                Err(e) => {
                    if let Ok(mut g) = log.lock() {
                        g.push_event("test", format!("FAILED SOCKS5 -> 1.1.1.1:443: {e:#}"));
                    }
                }
            }
        }
        if let Some(hp) = http_port {
            let t0 = std::time::Instant::now();
            match crate::tui::http_proxy::test_http_connect(hp, "1.1.1.1", 443) {
                Ok(()) => {
                    if let Ok(mut g) = log.lock() {
                        g.push_event(
                            "test",
                            format!(
                                "OK HTTP CONNECT 127.0.0.1:{hp} -> 1.1.1.1:443 ({} ms)",
                                t0.elapsed().as_millis()
                            ),
                        );
                    }
                }
                Err(e) => {
                    if let Ok(mut g) = log.lock() {
                        g.push_event(
                            "test",
                            format!("FAILED HTTP CONNECT 127.0.0.1:{hp} -> 1.1.1.1:443: {e:#}"),
                        );
                    }
                }
            }
        }
        for fwd in &forwards {
            if let Some((l, r)) = fwd.split_once(':') {
                if let Ok(lp) = l.parse::<u16>() {
                    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], lp));
                    let t0 = std::time::Instant::now();
                    match std::net::TcpStream::connect_timeout(
                        &addr,
                        std::time::Duration::from_secs(3),
                    ) {
                        Ok(_) => {
                            if let Ok(mut g) = log.lock() {
                                g.push_event(
                                    "test",
                                    format!(
                                        "OK local forward 127.0.0.1:{lp} -> remote:{r} ({} ms)",
                                        t0.elapsed().as_millis()
                                    ),
                                );
                            }
                        }
                        Err(e) => {
                            if let Ok(mut g) = log.lock() {
                                g.push_event(
                                    "test",
                                    format!("FAILED local forward 127.0.0.1:{lp}: {e}"),
                                );
                            }
                        }
                    }
                }
            }
        }
    });
}

fn open_proxy_config_editor(app: &mut App, host_idx: usize, host: &Host) {
    let mut form = crate::types::HostForm::from_host(host);
    form.page = 1;
    form.focused = 8;
    form.cursor = crate::tui::input::end_of(&form.port_forwards);
    app.host_form = Some(form);
    app.host_form_from_audit = false;
    app.host_form_return_to_proxy = Some(host_idx);
    app.screen = Screen::HostForm;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn sample_host(socks: Option<u16>, http: Option<u16>, forwards: Vec<&str>) -> Host {
        Host {
            id: "h1".into(),
            name: "edge".into(),
            ip: "10.0.0.5".into(),
            group: "prod".into(),
            port: 22,
            user: Some("ops".into()),
            tags: vec![],
            description: None,
            jump_host_id: None,
            jump_credential_id: None,
            port_forwards: forwards.into_iter().map(String::from).collect(),
            socks_port: socks,
            http_proxy_port: http,
        }
    }

    #[test]
    fn renders_configured_endpoints_and_error_diagnostics() {
        let host = sample_host(Some(1080), Some(8081), vec!["15432:5432"]);
        let mut app = App::new(vec![host], vec![], Default::default(), vec![]);
        app.screen = Screen::Proxy { host_idx: 0 };
        app.proxy_last_error = Some(crate::tui::ProxyErrorReport {
            host_id: "h1".into(),
            message: "Permission denied".into(),
            stderr_lines: vec![
                "[12:34:56] [ssh] Permission denied (publickey).".into(),
                "[12:34:57] [test] OK SOCKS5 -> 1.1.1.1:443 (42 ms)".into(),
            ],
        });

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(text.contains("Proxy & Tunnels — edge"));
        assert!(text.contains("Failed"));
        assert!(text.contains("socks5h://127.0.0.1:1080"));
        assert!(text.contains("http://127.0.0.1:8081"));
        assert!(text.contains("127.0.0.1:15432→5432"));
        assert!(text.contains("Activity Log"));
        assert!(text.contains("Permission denied (publickey)."));
        assert!(text.contains("OK SOCKS5 -> 1.1.1.1:443 (42 ms)"));
    }

    #[test]
    fn parses_timestamp_and_tag_from_log_lines() {
        assert_eq!(
            parse_log_line("[12:34:56] [http] CONNECT example.com:443"),
            (Some("12:34:56"), Some("http"), "CONNECT example.com:443")
        );
        assert_eq!(
            parse_log_line("[08:00:01] plain message"),
            (Some("08:00:01"), None, "plain message")
        );
        assert_eq!(
            parse_log_line("Permission denied (publickey)."),
            (None, None, "Permission denied (publickey).")
        );
    }

    #[test]
    fn unconfigured_host_prompts_f2_configuration() {
        let host = sample_host(None, None, vec![]);
        let mut app = App::new(vec![host], vec![], Default::default(), vec![]);
        app.screen = Screen::Proxy { host_idx: 0 };

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(text.contains("No SOCKS5, HTTP proxy, or port forwards configured"));
        assert!(text.contains("[F2] Ports & Proxy"));
    }

    #[test]
    fn pressing_e_opens_host_form_on_f2_and_returns_to_proxy() {
        let host = sample_host(Some(1080), None, vec![]);
        let mut app = App::new(vec![host.clone()], vec![], Default::default(), vec![]);
        app.screen = Screen::Proxy { host_idx: 0 };
        open_proxy_config_editor(&mut app, 0, &host);

        assert_eq!(app.screen, Screen::HostForm);
        assert_eq!(app.host_form_return_to_proxy, Some(0));
        let form = app.host_form.as_ref().unwrap();
        assert_eq!(form.page, 1);
        assert_eq!(form.focused, 8);
    }
}
