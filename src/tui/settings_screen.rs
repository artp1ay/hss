use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen};

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
const FIELD_MCP_LOG_FILE: usize = 10;
const FIELD_AUDIT_TIMEOUT: usize = 11;
const FIELD_COUNT: usize = 12;

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
        ("Enter/Esc", "save & back"),
    ];
    let hotkey_lines = crate::tui::wrap_hotkey_lines(hotkey_pairs, inner.width);
    let hotkey_height = hotkey_lines.len() as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // fields
            Constraint::Length(hotkey_height), // hotkeys
        ])
        .split(inner);

    // Field definitions: (label, hint)
    let field_defs: &[(&str, &str)] = &[
        ("Default User",      "fallback when host has no user set"),
        ("Default Port",      "port when host has no port set"),
        ("Default Group",     "default group for newly created hosts"),
        ("Connect Timeout",   "seconds before SSH gives up"),
        ("Strict Host Check", "accept-new / yes / no  [Space] cycle"),
        ("SSH Extra Args",    "appended to all SSH commands"),
        ("Auto-Save Creds",   "remember last-used credential per host  [Space] toggle"),
        ("Exec Timeout",      "max seconds for MCP command (0=no limit, default 300)"),
        ("MCP Port",          "HTTP port for MCP server (default 8822)"),
        ("MCP Token",         "bearer token for MCP auth (empty = no auth required)"),
        ("MCP Log File",      "file path for MCP request/response logging"),
        ("Audit Timeout",     "seconds for ping & connection check (default 3)"),
    ];

    let rows: Vec<Row> = field_defs.iter().enumerate().map(|(i, (label, hint))| {
        let focused = app.settings_focused_field == i;
        let value = app.settings_inputs.get(i).map(|s| s.as_str()).unwrap_or("");
        let label_style = if focused {
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value_style = if focused {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        let value_cell = if focused {
            Cell::from(Line::from(crate::tui::input::spans(value, app.settings_cursor, value_style)))
        } else {
            Cell::from(Span::styled(value.to_string(), value_style))
        };
        Row::new(vec![
            Cell::from(Span::styled(*label, label_style)),
            value_cell,
            Cell::from(Span::styled(*hint, Style::default().fg(Color::DarkGray))),
        ])
    }).collect();

    let focused_idx = app.settings_focused_field;
    let mut state = TableState::default().with_selected(Some(focused_idx));

    let table = Table::new(rows, [
        Constraint::Length(20),
        Constraint::Length(24),
        Constraint::Min(0),
    ])
    .row_highlight_style(Style::default().bg(Color::Rgb(20, 30, 45)));

    f.render_stateful_widget(table, chunks[0], &mut state);

    f.render_widget(Paragraph::new(hotkey_lines), chunks[1]);
}

fn apply_inputs_to_config(app: &mut App) {
    let inputs = &app.settings_inputs;
    if inputs.len() != FIELD_COUNT { return; }
    app.config.default_user = if inputs[FIELD_DEFAULT_USER].trim().is_empty() {
        None
    } else {
        Some(inputs[FIELD_DEFAULT_USER].trim().to_string())
    };
    app.config.default_port = inputs[FIELD_DEFAULT_PORT].trim().parse().unwrap_or(22);
    app.config.default_group = if inputs[FIELD_DEFAULT_GROUP].trim().is_empty() {
        "ungrouped".to_string()
    } else {
        inputs[FIELD_DEFAULT_GROUP].trim().to_string()
    };
    app.config.connect_timeout = inputs[FIELD_CONNECT_TIMEOUT].trim().parse().unwrap_or(10);
    app.config.strict_host_checking = inputs[FIELD_STRICT_HOST].trim().to_string();
    app.config.ssh_extra_args = inputs[FIELD_SSH_EXTRA_ARGS].trim().to_string();
    app.config.auto_save_credential = inputs[FIELD_AUTO_SAVE] == "yes";
    app.config.exec_timeout = inputs[FIELD_EXEC_TIMEOUT].trim().parse().unwrap_or(300);
    app.config.mcp_port = inputs[FIELD_MCP_PORT].trim().parse().unwrap_or(8822);
    app.config.mcp_token = if inputs[FIELD_MCP_TOKEN].trim().is_empty() {
        None
    } else {
        Some(inputs[FIELD_MCP_TOKEN].trim().to_string())
    };
    app.config.mcp_log_file = if inputs[FIELD_MCP_LOG_FILE].trim().is_empty() {
        None
    } else {
        Some(inputs[FIELD_MCP_LOG_FILE].trim().to_string())
    };
    app.config.audit_timeout = inputs[FIELD_AUDIT_TIMEOUT].trim().parse().unwrap_or(3);
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // Text fields get full line editing; Space stays a toggle on the two boolean-ish fields.
    let toggle_field = matches!(app.settings_focused_field, FIELD_STRICT_HOST | FIELD_AUTO_SAVE);
    let is_toggle_space = toggle_field && key.code == KeyCode::Char(' ');
    if !is_toggle_space
        && !matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab)
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
        KeyCode::Esc | KeyCode::Enter => {
            apply_inputs_to_config(app);
            crate::config::save_config(&app.config)?;
            app.settings_inputs.clear();
            app.settings_focused_field = 0;
            app.screen = Screen::Main;
        }
        KeyCode::Tab => {
            app.settings_focused_field = (app.settings_focused_field + 1) % FIELD_COUNT;
            app.settings_cursor = focused_len(app);
        }
        KeyCode::BackTab => {
            app.settings_focused_field = app.settings_focused_field
                .checked_sub(1)
                .unwrap_or(FIELD_COUNT - 1);
            app.settings_cursor = focused_len(app);
        }
        KeyCode::Char(' ') => {
            match app.settings_focused_field {
                FIELD_STRICT_HOST => {
                    app.settings_inputs[FIELD_STRICT_HOST] = match app.settings_inputs[FIELD_STRICT_HOST].as_str() {
                        "accept-new" => "yes",
                        "yes" => "no",
                        _ => "accept-new",
                    }.to_string();
                }
                FIELD_AUTO_SAVE => {
                    app.settings_inputs[FIELD_AUTO_SAVE] = if app.settings_inputs[FIELD_AUTO_SAVE] == "yes" {
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
