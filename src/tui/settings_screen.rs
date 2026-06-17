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
const FIELD_CONNECT_TIMEOUT: usize = 2;
const FIELD_STRICT_HOST: usize = 3;
const FIELD_SSH_EXTRA_ARGS: usize = 4;
const FIELD_AUTO_SAVE: usize = 5;
const FIELD_COUNT: usize = 6;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let outer = Block::default()
        .title(" hss · settings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // fields
            Constraint::Length(1), // hotkeys
        ])
        .split(inner);

    // Field definitions: (label, hint)
    let field_defs: &[(&str, &str)] = &[
        ("Default User",      "fallback when host has no user set"),
        ("Default Port",      "port when host has no port set"),
        ("Connect Timeout",   "seconds before SSH gives up"),
        ("Strict Host Check", "accept-new / yes / no  [Space] cycle"),
        ("SSH Extra Args",    "appended to all SSH commands"),
        ("Auto-Save Creds",   "remember last-used credential per host  [Space] toggle"),
    ];

    let rows: Vec<Row> = field_defs.iter().enumerate().map(|(i, (label, hint))| {
        let focused = app.settings_focused_field == i;
        let value = app.settings_inputs.get(i).map(|s| s.as_str()).unwrap_or("");
        let label_style = if focused {
            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value_display = if focused { format!("{value}█") } else { value.to_string() };
        let value_style = if focused {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::Gray)
        };
        Row::new(vec![
            Cell::from(Span::styled(*label, label_style)),
            Cell::from(Span::styled(value_display, value_style)),
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

    let default_cred = app.config.default_credential_id.as_deref()
        .and_then(|id| app.credentials.iter().find(|c| c.id == id))
        .map(|c| format!("default cred: {} · ", c.name))
        .unwrap_or_default();

    let hotkeys = Line::from(vec![
        Span::styled(default_cred, Style::default().fg(Color::Yellow)),
        Span::styled("[Tab]", Style::default().fg(Color::Blue)),
        Span::styled(" next  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Space]", Style::default().fg(Color::Blue)),
        Span::styled(" toggle  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Enter/Esc]", Style::default().fg(Color::Blue)),
        Span::styled(" save & back", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[1]);
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
    app.config.connect_timeout = inputs[FIELD_CONNECT_TIMEOUT].trim().parse().unwrap_or(10);
    app.config.strict_host_checking = inputs[FIELD_STRICT_HOST].trim().to_string();
    app.config.ssh_extra_args = inputs[FIELD_SSH_EXTRA_ARGS].trim().to_string();
    app.config.auto_save_credential = inputs[FIELD_AUTO_SAVE] == "yes";
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
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
        }
        KeyCode::BackTab => {
            app.settings_focused_field = app.settings_focused_field
                .checked_sub(1)
                .unwrap_or(FIELD_COUNT - 1);
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
                _ => {
                    app.settings_inputs[app.settings_focused_field].push(' ');
                }
            }
        }
        KeyCode::Char(c) => {
            app.settings_inputs[app.settings_focused_field].push(c);
        }
        KeyCode::Backspace => {
            app.settings_inputs[app.settings_focused_field].pop();
        }
        _ => {}
    }
    Ok(())
}
