use crate::tui::{App, Screen, Term};
use crate::types::Host;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn has_proxy_config(host: &Host) -> bool {
    host.socks_port.is_some() || host.http_proxy_port.is_some() || !host.port_forwards.is_empty()
}

pub fn draw(f: &mut Frame, app: &App) {
    let Screen::Proxy { host_idx } = &app.screen else {
        return;
    };
    let Some(host) = app.hosts.get(*host_idx) else {
        return;
    };

    let area = crate::tui::adaptive_centered_rect(78, 84, 72, 18, f.area());
    f.render_widget(Clear, area);

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

    let border_color = if active_here.is_some() {
        Color::Green
    } else if starting_here.is_some() {
        Color::Yellow
    } else if error_here.is_some() {
        Color::Red
    } else {
        Color::Blue
    };

    let block = Block::default()
        .title(format!(" Proxy & Tunnels — {} ", host.name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5), // status & connection info
            Constraint::Length(8), // endpoints & usage
            Constraint::Min(4),    // SSH / proxy diagnostics log
            Constraint::Length(1), // hotkeys
        ])
        .split(inner);

    // 1. Status & connection metadata
    let (status_badge, status_color, status_detail) = if let Some(proxy) = active_here {
        let up = proxy.started_at.elapsed().as_secs();
        (
            "● ACTIVE",
            Color::Green,
            format!("up {}m{:02}s", up / 60, up % 60),
        )
    } else if let Some(starting) = starting_here {
        let elapsed = starting.started_at.elapsed().as_secs();
        (
            "◌ STARTING…",
            Color::Yellow,
            format!("connecting ({elapsed}s)"),
        )
    } else if let Some(err) = error_here {
        ("✗ FAILED", Color::Red, err.message.clone())
    } else {
        ("○ STOPPED", Color::DarkGray, "not running".into())
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
    .unwrap_or_else(|| "none — press [R] on main list".into());

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
            .map(|c| format!(" · cred {}", c.name))
            .unwrap_or_default();
            format!("{} ({}){jc}", jh.name, jh.ip)
        });

    let mut status_lines = vec![
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                status_badge,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {status_detail}"),
                Style::default().fg(if error_here.is_some() {
                    Color::Red
                } else {
                    Color::Gray
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Target: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}:{}", host.ip, host.port),
                Style::default().fg(Color::White),
            ),
            Span::styled("   Cred: ", Style::default().fg(Color::DarkGray)),
            Span::styled(resolved_cred, Style::default().fg(Color::Cyan)),
        ]),
    ];

    if let Some(jump_desc) = jump_text {
        status_lines.push(Line::from(vec![
            Span::styled("Jump:   ", Style::default().fg(Color::DarkGray)),
            Span::styled(jump_desc, Style::default().fg(Color::Yellow)),
        ]));
    } else if let Some(other) = active_elsewhere {
        status_lines.push(Line::from(vec![
            Span::styled("Notice: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!(
                    "Proxy currently running on '{other}' — [Enter] switches here, [S] stops it"
                ),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    } else {
        status_lines.push(Line::from(Span::styled(
            "Listeners bind 127.0.0.1 only and stay alive while hss is open.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    if let (Some(_), Some(other)) = (host.jump_host_id.as_deref(), active_elsewhere) {
        status_lines.push(Line::from(vec![
            Span::styled("Notice: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("Active proxy on '{other}' will be stopped if started here"),
                Style::default().fg(Color::Yellow),
            ),
        ]));
    }

    f.render_widget(
        Paragraph::new(status_lines)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: true }),
        chunks[0],
    );

    // 2. Configured & live endpoints
    let mut ep_lines: Vec<Line> = Vec::new();
    let configured = has_proxy_config(host);
    if !configured {
        ep_lines.push(Line::from(Span::styled(
            "No SOCKS5, HTTP proxy, or port forwards configured for this host yet.",
            Style::default().fg(Color::Yellow),
        )));
        ep_lines.push(Line::from(""));
        ep_lines.push(Line::from(vec![
            Span::styled("Press ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "[E]",
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " to open [F2] Ports & Proxy and configure ports.",
                Style::default().fg(Color::White),
            ),
        ]));
    } else {
        let ep_state_label = if active_here.is_some() {
            ("listening", Color::Green)
        } else if starting_here.is_some() {
            ("starting", Color::Yellow)
        } else {
            ("stopped", Color::DarkGray)
        };

        if let Some(socks_port) = host.socks_port {
            ep_lines.push(Line::from(vec![
                Span::styled("SOCKS5 Proxy   : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("socks5h://127.0.0.1:{socks_port}"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  [{}]", ep_state_label.0),
                    Style::default().fg(ep_state_label.1),
                ),
            ]));
        } else if let Some(internal) = active_here
            .and_then(|p| p.summary.internal_socks_port)
            .or_else(|| starting_here.and_then(|s| s.summary.internal_socks_port))
        {
            ep_lines.push(Line::from(vec![
                Span::styled("SOCKS5 Backend : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("127.0.0.1:{internal} (auto internal for HTTP)"),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }

        if let Some(http_port) = host.http_proxy_port {
            let actual_port = active_here
                .and_then(crate::tui::ProxySession::http_bound_port)
                .unwrap_or(http_port);
            let metrics_suffix = active_here
                .and_then(crate::tui::ProxySession::http_metrics)
                .map(|m| format!("  ({} reqs, {} active)", m.total_requests, m.active_conns))
                .unwrap_or_default();
            ep_lines.push(Line::from(vec![
                Span::styled("HTTP / HTTPS   : ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("http://127.0.0.1:{actual_port}"),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  [{}]", ep_state_label.0),
                    Style::default().fg(ep_state_label.1),
                ),
                Span::styled(metrics_suffix, Style::default().fg(Color::Cyan)),
            ]));
        }

        if !host.port_forwards.is_empty() {
            let formatted: Vec<String> = host
                .port_forwards
                .iter()
                .map(|f| {
                    f.split_once(':')
                        .map(|(l, r)| format!("127.0.0.1:{l} -> remote:{r}"))
                        .unwrap_or_else(|| f.clone())
                })
                .collect();
            ep_lines.push(Line::from(vec![
                Span::styled("Port Forwards  : ", Style::default().fg(Color::DarkGray)),
                Span::styled(formatted.join(", "), Style::default().fg(Color::White)),
                Span::styled(
                    format!("  [{}]", ep_state_label.0),
                    Style::default().fg(ep_state_label.1),
                ),
            ]));
        }

        // Example usage for all configured proxy types
        let mut examples = Vec::new();
        if let Some(socks_port) = host.socks_port {
            examples.push(format!(
                "SOCKS5 cmd : curl -x socks5h://127.0.0.1:{socks_port} https://ifconfig.me"
            ));
        }
        if let Some(http_port) = host.http_proxy_port {
            examples.push(format!(
                "HTTP/S cmd : curl -x http://127.0.0.1:{http_port} https://ifconfig.me"
            ));
        }
        if examples.is_empty() {
            if let Some(first) = host.port_forwards.first() {
                let local = first.split_once(':').map(|(l, _)| l).unwrap_or(first);
                examples.push(format!(
                    "Example: connect local client to 127.0.0.1:{local}"
                ));
            }
        }
        if !examples.is_empty() {
            ep_lines.push(Line::from(""));
            for ex in examples {
                ep_lines.push(Line::from(Span::styled(
                    ex,
                    Style::default().fg(Color::Cyan),
                )));
            }
        }

        if let Some(http_err) = active_here
            .and_then(crate::tui::ProxySession::http_metrics)
            .and_then(|m| m.last_error)
        {
            ep_lines.push(Line::from(vec![
                Span::styled("Last HTTP err  : ", Style::default().fg(Color::Yellow)),
                Span::styled(http_err, Style::default().fg(Color::Red)),
            ]));
        }
    }

    f.render_widget(
        Paragraph::new(ep_lines)
            .block(
                Block::default()
                    .title(" Endpoints ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );

    // 3. SSH stderr / diagnostics log
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

    let log_block = Block::default()
        .title(format!(
            " Proxy & SSH Debug Log ({} lines · ↑/↓/PgUp/PgDn scroll · [T] test · [C] clear) ",
            log_lines.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let log_inner_h = chunks[2].height.saturating_sub(2) as usize;
    let rendered_log: Vec<Line> = if log_lines.is_empty() {
        vec![Line::from(Span::styled(
            if active_here.is_some() {
                "Tunnel is active — press [T] to run an end-to-end proxy test."
            } else if starting_here.is_some() {
                "Waiting for SSH handshake and local port binding…"
            } else {
                "No proxy log yet. Press [Enter] to start proxy."
            },
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        let max_scroll = log_lines.len().saturating_sub(log_inner_h);
        // Auto-follow bottom when scroll == usize::MAX, otherwise clamp
        let scroll = if app.proxy_log_scroll == usize::MAX {
            max_scroll
        } else {
            app.proxy_log_scroll.min(max_scroll)
        };
        log_lines
            .iter()
            .skip(scroll)
            .take(log_inner_h.max(1))
            .map(|l| {
                let lower = l.to_lowercase();
                let color = if lower.contains("failed")
                    || lower.contains("denied")
                    || lower.contains("error")
                    || lower.contains("cannot")
                    || lower.contains("refused")
                    || lower.contains("prohibited")
                {
                    Color::Red
                } else if lower.contains("warning") || lower.contains("permanently added") {
                    Color::Yellow
                } else if lower.contains("[socks] connected")
                    || lower.contains("tunnel ready")
                    || lower.contains("authenticated to")
                    || lower.contains("[test] ok")
                {
                    Color::Green
                } else if lower.contains("[http]")
                    || lower.contains("[socks]")
                    || lower.contains("[proxy]")
                    || lower.contains("[test]")
                {
                    Color::Cyan
                } else {
                    Color::Gray
                };
                Line::from(Span::styled(l.clone(), Style::default().fg(color)))
            })
            .collect()
    };
    f.render_widget(Paragraph::new(rendered_log).block(log_block), chunks[2]);

    // 4. Hotkeys
    let primary_label = if active_here.is_some() {
        "stop proxy"
    } else if starting_here.is_some() {
        "cancel startup"
    } else if active_elsewhere.is_some() {
        "switch proxy here"
    } else {
        "start proxy"
    };
    let close_label = if app.proxy.is_some() || app.proxy_starting.is_some() {
        "close (keeps running)"
    } else {
        "close"
    };

    let mut pairs: Vec<(&str, &str)> = Vec::new();
    if configured || active_here.is_some() || starting_here.is_some() {
        pairs.push(("Enter/Space", primary_label));
    }
    if active_here.is_some() {
        pairs.push(("T", "test tunnel"));
    }
    if active_elsewhere.is_some() {
        pairs.push(("S", "stop active proxy"));
    }
    pairs.push(("R", "switch cred"));
    pairs.push(("C", "clear log"));
    pairs.push(("E", "configure ports (F2)"));
    pairs.push(("Esc", close_label));

    let hotkey_lines = crate::tui::wrap_hotkey_lines(&pairs, chunks[3].width);
    f.render_widget(Paragraph::new(hotkey_lines), chunks[3]);
}

pub fn handle_key(
    _terminal: &mut Term,
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
            let total = current_log_len(app, &host.id);
            let cur = if app.proxy_log_scroll == usize::MAX {
                total.saturating_sub(6)
            } else {
                app.proxy_log_scroll
            };
            app.proxy_log_scroll = cur.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.proxy_log_scroll != usize::MAX {
                app.proxy_log_scroll = app.proxy_log_scroll.saturating_add(1);
            }
        }
        KeyCode::PageUp => {
            let total = current_log_len(app, &host.id);
            let cur = if app.proxy_log_scroll == usize::MAX {
                total.saturating_sub(6)
            } else {
                app.proxy_log_scroll
            };
            app.proxy_log_scroll = cur.saturating_sub(8);
        }
        KeyCode::PageDown => {
            app.proxy_log_scroll = usize::MAX;
        }
        _ => {}
    }
    Ok(())
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
                    format!(
                        "Testing SOCKS5 127.0.0.1:{sp} -> external 1.1.1.1:443 via remote host…"
                    ),
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
            stderr_lines: vec!["Permission denied (publickey).".into()],
        });

        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(text.contains("Proxy & Tunnels"));
        assert!(text.contains("FAILED"));
        assert!(text.contains("socks5h://127.0.0.1:1080"));
        assert!(text.contains("http://127.0.0.1:8081"));
        assert!(text.contains("127.0.0.1:15432 -> remote:5432"));
        assert!(text.contains("Permission denied (publickey)."));
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
