use crate::tui::{App, Screen};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

// Field indices
const FIELD_DEFAULT_USER: usize = 0;
const FIELD_DEFAULT_PORT: usize = 1;
const FIELD_DEFAULT_GROUP: usize = 2;
const FIELD_CONNECT_TIMEOUT: usize = 3;
const FIELD_STRICT_HOST: usize = 4;
const FIELD_SSH_EXTRA_ARGS: usize = 5;
const FIELD_AUTO_SAVE: usize = 6;
const FIELD_EXEC_TIMEOUT: usize = 7;
const FIELD_MCP_PORT: usize = 8;
const FIELD_MCP_TOKEN: usize = 9;
const FIELD_MCP_READ_ONLY: usize = 10;
const FIELD_MCP_ALLOWED_HOSTS: usize = 11;
const FIELD_MCP_DLP_FILTER: usize = 12;
const FIELD_MCP_LOG_FILE: usize = 13;
const FIELD_AUDIT_TIMEOUT: usize = 14;
const FIELD_COUNT: usize = 15;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let outer = Block::default()
        .title(" hss · settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let hotkey_pairs = &[
        ("Tab", "next"),
        ("BackTab", "prev"),
        ("Space", "toggle"),
        ("F2", "reveal/hide token"),
        ("Enter", "save"),
        ("Esc", "cancel"),
    ];
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkey_pairs, inner.width);
    let hotkey_height = hotkey_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0), // fields
            Constraint::Length(if app.status_message.is_some() { 1 } else { 0 }),
            Constraint::Length(hotkey_height), // hotkeys
        ])
        .split(inner);

    // Field definitions: (label, hint)
    let field_defs: &[(&str, &str)] = &[
        ("Default User", "fallback when host has no user set"),
        ("Default Port", "port when host has no port set"),
        ("Default Group", "default group for newly created hosts"),
        ("Connect Timeout", "seconds before SSH gives up"),
        ("Strict Host Check", "accept-new / yes / no  [Space] cycle"),
        ("SSH Extra Args", "appended to all SSH commands"),
        (
            "Auto-Save Creds",
            "remember last-used credential per host  [Space] toggle",
        ),
        (
            "Exec Timeout",
            "max seconds for MCP command (0=no limit, default 300)",
        ),
        ("MCP Port", "HTTP port for MCP server (default 8822)"),
        (
            "MCP Token",
            "bearer token for MCP auth (empty = no auth required)",
        ),
        (
            "MCP Read-Only",
            "block dangerous/destructive commands  [Space] toggle",
        ),
        (
            "MCP Allowed Hosts",
            "comma-separated host names or patterns (* = all)",
        ),
        (
            "MCP DLP Filter",
            "redact private keys/secrets in outputs  [Space] toggle",
        ),
        ("MCP Log File", "file path for MCP request/response logging"),
        (
            "Audit Timeout",
            "seconds for ping & connection check (default 3)",
        ),
    ];

    let rows: Vec<Row> = field_defs
        .iter()
        .enumerate()
        .map(|(i, (label, hint))| {
            let focused = app.settings_focused_field == i;
            let value = app.settings_inputs.get(i).map(|s| s.as_str()).unwrap_or("");
            let label_style = if focused {
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let value_style = if focused {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Gray)
            };
            let is_token_field = i == FIELD_MCP_TOKEN;
            let display_value = if is_token_field && !app.settings_reveal_token {
                if value.is_empty() {
                    "".to_string()
                } else {
                    "•".repeat(value.chars().count())
                }
            } else {
                value.to_string()
            };

            let value_cell = if focused {
                Cell::from(Line::from(crate::tui::input::spans(
                    &display_value,
                    app.settings_cursor,
                    value_style,
                )))
            } else {
                Cell::from(Span::styled(display_value, value_style))
            };
            Row::new(vec![
                Cell::from(Span::styled(*label, label_style)),
                value_cell,
                Cell::from(Span::styled(*hint, Style::default().fg(Color::DarkGray))),
            ])
        })
        .collect();

    let focused_idx = app.settings_focused_field;
    let mut state = TableState::default().with_selected(Some(focused_idx));

    let table = Table::new(
        rows,
        [
            Constraint::Length(20),
            Constraint::Length(24),
            Constraint::Min(0),
        ],
    )
    .row_highlight_style(Style::default().bg(Color::Rgb(20, 30, 45)));

    f.render_stateful_widget(table, chunks[0], &mut state);

    if let Some(ref msg) = app.status_message {
        let color = match msg.kind {
            crate::types::StatusKind::Success => Color::Green,
            crate::types::StatusKind::Info => Color::Cyan,
            crate::types::StatusKind::Warning => Color::Yellow,
            crate::types::StatusKind::Error => Color::Red,
        };
        f.render_widget(
            Paragraph::new(Span::styled(&msg.text, Style::default().fg(color))),
            chunks[1],
        );
    }

    f.render_widget(Paragraph::new(hotkey_lines), chunks[2]);
}

fn validate_and_build_config(app: &App) -> Result<crate::config::AppConfig> {
    let inputs = &app.settings_inputs;
    if inputs.len() != FIELD_COUNT {
        anyhow::bail!("Settings state is incomplete.");
    }

    let mut cfg = app.config.clone();

    let default_user = inputs[FIELD_DEFAULT_USER].trim();
    cfg.default_user = if default_user.is_empty() {
        None
    } else {
        crate::ssh::validate_username(default_user)?;
        Some(default_user.to_string())
    };

    let default_port: u16 = inputs[FIELD_DEFAULT_PORT]
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Default Port must be a number between 1 and 65535."))?;
    anyhow::ensure!(
        default_port > 0,
        "Default Port must be between 1 and 65535."
    );
    cfg.default_port = default_port;

    let default_group = inputs[FIELD_DEFAULT_GROUP].trim();
    if default_group.eq_ignore_ascii_case("all") || default_group.eq_ignore_ascii_case("_meta") {
        anyhow::bail!("Default Group '{default_group}' is reserved by Ansible inventory.");
    }
    cfg.default_group = if default_group.is_empty() {
        "ungrouped".to_string()
    } else {
        default_group.to_string()
    };

    cfg.connect_timeout = inputs[FIELD_CONNECT_TIMEOUT]
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Connect Timeout must be a non-negative integer."))?;

    let strict_host = inputs[FIELD_STRICT_HOST].trim();
    if !strict_host.is_empty() {
        crate::config::validate_strict_host_checking(strict_host)?;
    }
    cfg.strict_host_checking = strict_host.to_string();

    cfg.ssh_extra_args = inputs[FIELD_SSH_EXTRA_ARGS].trim().to_string();
    cfg.auto_save_credential = inputs[FIELD_AUTO_SAVE] == "yes";

    cfg.exec_timeout = inputs[FIELD_EXEC_TIMEOUT]
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Exec Timeout must be a non-negative integer."))?;

    cfg.mcp_port = inputs[FIELD_MCP_PORT]
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("MCP Port must be a number between 0 and 65535."))?;

    cfg.mcp_token = if inputs[FIELD_MCP_TOKEN].trim().is_empty() {
        None
    } else {
        Some(inputs[FIELD_MCP_TOKEN].trim().to_string())
    };
    cfg.mcp_read_only = inputs[FIELD_MCP_READ_ONLY] == "yes";
    cfg.mcp_allowed_hosts = if inputs[FIELD_MCP_ALLOWED_HOSTS].trim().is_empty() {
        "*".to_string()
    } else {
        inputs[FIELD_MCP_ALLOWED_HOSTS].trim().to_string()
    };
    cfg.mcp_dlp_filter = inputs[FIELD_MCP_DLP_FILTER] == "yes";
    cfg.mcp_log_file = if inputs[FIELD_MCP_LOG_FILE].trim().is_empty() {
        None
    } else {
        Some(inputs[FIELD_MCP_LOG_FILE].trim().to_string())
    };

    let audit_timeout: u16 = inputs[FIELD_AUDIT_TIMEOUT]
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Audit Timeout must be a positive integer."))?;
    anyhow::ensure!(
        audit_timeout > 0,
        "Audit Timeout must be at least 1 second."
    );
    cfg.audit_timeout = audit_timeout;

    Ok(cfg)
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Text fields get full line editing; Space stays a toggle on the two boolean-ish fields.
    let toggle_field = matches!(
        app.settings_focused_field,
        FIELD_STRICT_HOST | FIELD_AUTO_SAVE | FIELD_MCP_READ_ONLY | FIELD_MCP_DLP_FILTER
    );
    let is_toggle_space = toggle_field && key.code == KeyCode::Char(' ');
    let is_reveal_toggle = key.code == KeyCode::F(2);
    if !is_toggle_space
        && !is_reveal_toggle
        && !matches!(
            key.code,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab
        )
    {
        if let Some(field) = app.settings_inputs.get_mut(app.settings_focused_field) {
            let mut cursor = app.settings_cursor;
            let mut buf = std::mem::take(field);
            let consumed = crate::tui::input::handle(&mut buf, &mut cursor, key);
            app.settings_inputs[app.settings_focused_field] = buf;
            app.settings_cursor = cursor;
            if consumed {
                return Ok(());
            }
        }
    }

    match key.code {
        KeyCode::Enter => match validate_and_build_config(app) {
            Ok(new_cfg) => {
                crate::config::save_config(&new_cfg)?;
                app.config = new_cfg;
                app.settings_inputs.clear();
                app.settings_focused_field = 0;
                app.status_message = Some(crate::types::StatusMessage::success("Settings saved."));
                app.screen = Screen::Main;
            }
            Err(e) => {
                app.status_message = Some(crate::types::StatusMessage::error(format!("{e}")));
            }
        },
        KeyCode::Esc => {
            // Discard changes without saving
            app.settings_inputs.clear();
            app.settings_focused_field = 0;
            app.status_message = Some(crate::types::StatusMessage::info("Settings discarded."));
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            app.settings_focused_field = (app.settings_focused_field + 1) % FIELD_COUNT;
            app.settings_cursor = focused_len(app);
        }
        KeyCode::BackTab => {
            app.settings_focused_field = app
                .settings_focused_field
                .checked_sub(1)
                .unwrap_or(FIELD_COUNT - 1);
            app.settings_cursor = focused_len(app);
        }
        KeyCode::F(2) => {
            app.settings_reveal_token = !app.settings_reveal_token;
        }
        KeyCode::Char(' ') => {
            match app.settings_focused_field {
                FIELD_STRICT_HOST => {
                    app.settings_inputs[FIELD_STRICT_HOST] =
                        match app.settings_inputs[FIELD_STRICT_HOST].as_str() {
                            "accept-new" => "yes",
                            "yes" => "no",
                            _ => "accept-new",
                        }
                        .to_string();
                }
                FIELD_AUTO_SAVE => {
                    app.settings_inputs[FIELD_AUTO_SAVE] =
                        if app.settings_inputs[FIELD_AUTO_SAVE] == "yes" {
                            "no".to_string()
                        } else {
                            "yes".to_string()
                        };
                }
                FIELD_MCP_READ_ONLY => {
                    app.settings_inputs[FIELD_MCP_READ_ONLY] =
                        if app.settings_inputs[FIELD_MCP_READ_ONLY] == "yes" {
                            "no".to_string()
                        } else {
                            "yes".to_string()
                        };
                }
                FIELD_MCP_DLP_FILTER => {
                    app.settings_inputs[FIELD_MCP_DLP_FILTER] =
                        if app.settings_inputs[FIELD_MCP_DLP_FILTER] == "yes" {
                            "no".to_string()
                        } else {
                            "yes".to_string()
                        };
                }
                _ => {}
            }
            app.settings_cursor = focused_len(app);
        }
        _ => {}
    }
    Ok(())
}

fn focused_len(app: &App) -> usize {
    app.settings_inputs
        .get(app.settings_focused_field)
        .map(|s| crate::tui::input::end_of(s))
        .unwrap_or(0)
}
