use crate::config::AppConfig;
use crate::tui::{theme, App, Screen};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame,
};

// Field indices must match the draft initialized by main_screen.
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

const FIELD_DEFS: [(&str, &str); FIELD_COUNT] = [
    ("Default User", "Fallback when a host has no user set; blank uses SSH's default."),
    ("Default Port", "SSH port: 1–65535. A number is required."),
    ("Default Group", "Group for new hosts; blank uses ungrouped."),
    ("Connect Timeout", "SSH connection timeout: 1–65535 seconds."),
    ("Strict Host Check", "[Space] cycle accept-new / yes / no. Disabling checks is unsafe."),
    ("SSH Extra Args", "Appended to all SSH commands."),
    ("Auto-Save Creds", "[Space] toggle remembering the last-used credential per host."),
    ("Exec Timeout", "MCP command timeout: 0–4294967295 seconds; 0 means no limit."),
    ("MCP Port", "MCP HTTP port: 1–65535. A number is required."),
    ("MCP Token", "HSS_MCP_TOKEN overrides this token. Without either, requests need no auth; execution is disabled unless allow_unauthenticated_execute is enabled. [F2] reveal while focused."),
    ("MCP Read-Only", "[Space] toggle a destructive-command denylist, not a sandbox or a guarantee of read-only execution."),
    ("MCP Allowed Hosts", "Comma-separated host names or glob patterns; blank or * exposes all hosts."),
    ("MCP DLP Filter", "[Space] toggle PEM block redaction (keys/certificates) in responses unless allow_key_output is set. Logs always redact PEM; other secrets are not covered."),
    ("MCP Log File", "Path for MCP request/response logs; blank disables logging."),
    ("Audit Timeout", "Ping and connection-check timeout: 1–65535 seconds."),
];

pub fn draw(f: &mut Frame, app: &App) {
    let area = crate::tui::help::content_area(app, f.area());
    let outer = Block::default()
        .title(if is_dirty(app) {
            " hss · settings · unsaved "
        } else {
            " hss · settings "
        })
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::MUTED));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let focused = app.settings_focused_field.min(FIELD_COUNT - 1);
    let hotkeys = if app.settings_confirm_discard {
        vec![("S/Enter", "save"), ("D", "discard"), ("C/Esc", "continue")]
    } else {
        let mut keys = vec![("Tab", "next"), ("Shift+Tab", "prev")];
        if is_toggle(focused) {
            keys.push(("Space", "toggle"));
        }
        if focused == FIELD_MCP_TOKEN {
            keys.push(("F2", "reveal/hide"));
        }
        keys.extend([("Enter", "save"), ("Esc", "back")]);
        keys
    };
    let hotkey_lines = crate::tui::wrap_hotkey_lines(&hotkeys, inner.width);
    let hint = if app.settings_confirm_discard {
        "Unsaved settings. Save changes, discard them, or continue editing?"
    } else {
        FIELD_DEFS[focused].1
    };
    let hint_lines = wrap_message(hint, inner.width);
    let error_lines = app
        .settings_error
        .as_deref()
        .map(|error| wrap_message(error, inner.width))
        .unwrap_or_default();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(hint_lines.len() as u16),
            Constraint::Length(error_lines.len().min(4) as u16),
            Constraint::Length(hotkey_lines.len() as u16),
        ])
        .split(inner);

    let label_width = 19u16.min(inner.width / 2);
    let value_width = inner.width.saturating_sub(label_width + 1);
    let rows: Vec<Row> = FIELD_DEFS
        .iter()
        .enumerate()
        .map(|(i, (label, _))| {
            let is_focused = focused == i;
            let value = app.settings_inputs.get(i).map(String::as_str).unwrap_or("");
            let masked;
            let display = if i == FIELD_MCP_TOKEN
                && !(is_focused && app.settings_reveal_token && !app.settings_confirm_discard)
            {
                masked = "•".repeat(value.chars().count());
                masked.as_str()
            } else {
                value
            };
            let style = Style::default().fg(if is_focused {
                theme::TEXT
            } else {
                theme::MUTED
            });
            let value = if is_focused && !is_toggle(i) {
                Cell::from(Line::from(crate::tui::input::spans_with_width(
                    display,
                    app.settings_cursor,
                    style,
                    value_width as usize,
                )))
            } else {
                Cell::from(Span::styled(display.to_string(), style))
            };
            Row::new(vec![
                Cell::from(Span::styled(
                    *label,
                    if is_focused {
                        Style::default()
                            .fg(theme::FOCUS)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::MUTED)
                    },
                )),
                value,
            ])
        })
        .collect();
    let table = Table::new(rows, [Constraint::Length(label_width), Constraint::Min(0)])
        .row_highlight_style(Style::default().bg(theme::SELECTION));
    let mut state = TableState::default().with_selected(Some(focused));
    f.render_stateful_widget(table, chunks[0], &mut state);
    f.render_widget(
        Paragraph::new(hint_lines).style(Style::default().fg(theme::MUTED)),
        chunks[1],
    );
    f.render_widget(
        Paragraph::new(error_lines).style(Style::default().fg(theme::ERROR)),
        chunks[2],
    );
    f.render_widget(Paragraph::new(hotkey_lines), chunks[3]);
}

fn wrap_message(message: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in message.split_whitespace() {
        if !line.is_empty()
            && crate::tui::input::str_width(&line) + crate::tui::input::str_width(word) > width
        {
            lines.push(Line::from(std::mem::take(&mut line)));
        }
        for ch in word.chars() {
            // Wrap long unbroken paths as well as ordinary help text.
            if crate::tui::input::str_width(&line) + crate::tui::input::str_width(&ch.to_string())
                > width
            {
                lines.push(Line::from(std::mem::take(&mut line)));
            }
            line.push(ch);
        }
        line.push(' ');
    }
    if !line.is_empty() {
        lines.push(Line::from(line.trim_end().to_string()));
    }
    lines
}

fn config_inputs(config: &AppConfig) -> Vec<String> {
    let yes_no = |value| if value { "yes" } else { "no" }.to_string();
    vec![
        config.default_user.clone().unwrap_or_default(),
        config.default_port.to_string(),
        config.default_group.clone(),
        config.connect_timeout.to_string(),
        config.strict_host_checking.clone(),
        config.ssh_extra_args.clone(),
        yes_no(config.auto_save_credential),
        config.exec_timeout.to_string(),
        config.mcp_port.to_string(),
        config.mcp_token.clone().unwrap_or_default(),
        yes_no(config.mcp_read_only),
        config.mcp_allowed_hosts.clone(),
        yes_no(config.mcp_dlp_filter),
        config.mcp_log_file.clone().unwrap_or_default(),
        config.audit_timeout.to_string(),
    ]
}

fn is_dirty(app: &App) -> bool {
    app.settings_inputs != config_inputs(&app.config)
}

fn is_toggle(field: usize) -> bool {
    matches!(
        field,
        FIELD_STRICT_HOST | FIELD_AUTO_SAVE | FIELD_MCP_READ_ONLY | FIELD_MCP_DLP_FILTER
    )
}

type ValidationError = (usize, String);

fn draft_config(
    config: &AppConfig,
    inputs: &[String],
) -> std::result::Result<AppConfig, ValidationError> {
    if inputs.len() != FIELD_COUNT {
        return Err((
            0,
            "Settings draft is incomplete; close and reopen settings.".into(),
        ));
    }
    let positive = |field: usize| -> std::result::Result<u16, ValidationError> {
        inputs[field]
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                (
                    field,
                    format!(
                        "{}: enter a whole number from 1 to 65535.",
                        FIELD_DEFS[field].0
                    ),
                )
            })
    };
    let boolean = |field: usize| -> std::result::Result<bool, ValidationError> {
        match inputs[field].as_str() {
            "yes" => Ok(true),
            "no" => Ok(false),
            _ => Err((
                field,
                format!("{}: use Space to choose yes or no.", FIELD_DEFS[field].0),
            )),
        }
    };
    let optional = |field: usize| {
        let value = inputs[field].trim();
        (!value.is_empty()).then(|| value.to_string())
    };
    let mut draft = config.clone();
    draft.default_user = optional(FIELD_DEFAULT_USER);
    draft.default_port = positive(FIELD_DEFAULT_PORT)?;
    draft.default_group = optional(FIELD_DEFAULT_GROUP).unwrap_or_else(|| "ungrouped".into());
    draft.connect_timeout = positive(FIELD_CONNECT_TIMEOUT)?;
    draft.strict_host_checking = match inputs[FIELD_STRICT_HOST].as_str() {
        "accept-new" | "yes" | "no" => inputs[FIELD_STRICT_HOST].clone(),
        _ => {
            return Err((
                FIELD_STRICT_HOST,
                "Strict Host Check: use Space to choose accept-new, yes, or no.".into(),
            ))
        }
    };
    draft.ssh_extra_args = inputs[FIELD_SSH_EXTRA_ARGS].trim().to_string();
    draft.auto_save_credential = boolean(FIELD_AUTO_SAVE)?;
    draft.exec_timeout = inputs[FIELD_EXEC_TIMEOUT]
        .trim()
        .parse::<u32>()
        .map_err(|_| {
            (
                FIELD_EXEC_TIMEOUT,
                "Exec Timeout: enter a whole number from 0 to 4294967295 (0 = no limit).".into(),
            )
        })?;
    draft.mcp_port = positive(FIELD_MCP_PORT)?;
    draft.mcp_token = optional(FIELD_MCP_TOKEN);
    draft.mcp_read_only = boolean(FIELD_MCP_READ_ONLY)?;
    draft.mcp_allowed_hosts = optional(FIELD_MCP_ALLOWED_HOSTS).unwrap_or_else(|| "*".into());
    draft.mcp_dlp_filter = boolean(FIELD_MCP_DLP_FILTER)?;
    draft.mcp_log_file = optional(FIELD_MCP_LOG_FILE);
    draft.audit_timeout = positive(FIELD_AUDIT_TIMEOUT)?;
    Ok(draft)
}

fn leave_settings(app: &mut App, message: crate::types::StatusMessage) {
    app.settings_inputs.clear();
    app.settings_focused_field = 0;
    app.settings_cursor = 0;
    app.settings_reveal_token = false;
    app.settings_error = None;
    app.settings_confirm_discard = false;
    app.status_message = Some(message);
    app.screen = Screen::Main;
}

fn save_settings_with(app: &mut App, save: impl FnOnce(&AppConfig) -> Result<()>) {
    app.settings_confirm_discard = false;
    app.settings_reveal_token = false;
    match draft_config(&app.config, &app.settings_inputs) {
        Err((field, message)) => {
            app.settings_focused_field = field;
            app.settings_cursor = focused_len(app);
            app.settings_error = Some(message);
        }
        Ok(draft) => match save(&draft) {
            Err(error) => {
                app.settings_error = Some(format!(
                    "Could not save settings: {error}. Changes are still unsaved."
                ))
            }
            Ok(()) => {
                app.config = draft;
                leave_settings(app, crate::types::StatusMessage::success("Settings saved."));
            }
        },
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if app.settings_focused_field != FIELD_MCP_TOKEN {
        app.settings_reveal_token = false;
    }
    if app.settings_confirm_discard {
        match key.code {
            KeyCode::Enter | KeyCode::Char('s' | 'S') => {
                save_settings_with(app, crate::config::save_config)
            }
            KeyCode::Char('d' | 'D') => leave_settings(
                app,
                crate::types::StatusMessage::info("Unsaved settings discarded."),
            ),
            KeyCode::Esc | KeyCode::Char('c' | 'C') => app.settings_confirm_discard = false,
            _ => {}
        }
        return Ok(());
    }

    // Toggle fields never accept free-form text, deletion, or line-edit commands.
    if !is_toggle(app.settings_focused_field)
        && !matches!(
            key.code,
            KeyCode::Esc | KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab | KeyCode::F(2)
        )
    {
        if let Some(field) = app.settings_inputs.get_mut(app.settings_focused_field) {
            if crate::tui::input::handle(field, &mut app.settings_cursor, key) {
                return Ok(());
            }
        }
    }
    match key.code {
        KeyCode::Enter => save_settings_with(app, crate::config::save_config),
        KeyCode::Esc if is_dirty(app) => {
            app.settings_confirm_discard = true;
            app.settings_reveal_token = false;
        }
        KeyCode::Esc => leave_settings(
            app,
            crate::types::StatusMessage::info("Settings closed; no changes."),
        ),
        KeyCode::Tab | KeyCode::BackTab => {
            app.settings_focused_field = if key.code == KeyCode::Tab {
                (app.settings_focused_field + 1) % FIELD_COUNT
            } else {
                (app.settings_focused_field + FIELD_COUNT - 1) % FIELD_COUNT
            };
            app.settings_cursor = focused_len(app);
            app.settings_reveal_token = false;
        }
        KeyCode::F(2) if app.settings_focused_field == FIELD_MCP_TOKEN => {
            app.settings_reveal_token = !app.settings_reveal_token;
        }
        KeyCode::Char(' ') if is_toggle(app.settings_focused_field) => {
            if let Some(value) = app.settings_inputs.get_mut(app.settings_focused_field) {
                *value = if app.settings_focused_field == FIELD_STRICT_HOST {
                    match value.as_str() {
                        "accept-new" => "yes",
                        "yes" => "no",
                        _ => "accept-new",
                    }
                } else if value == "yes" {
                    "no"
                } else {
                    "yes"
                }
                .into();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::{backend::TestBackend, Terminal};

    fn app() -> App {
        let mut app = App::new(vec![], vec![], AppConfig::default(), vec![]);
        app.screen = Screen::Settings;
        app.settings_inputs = config_inputs(&app.config);
        app
    }

    fn key(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    #[test]
    fn numeric_drafts_reject_blank_invalid_zero_and_overflow() {
        let config = AppConfig::default();
        for field in [
            FIELD_DEFAULT_PORT,
            FIELD_CONNECT_TIMEOUT,
            FIELD_MCP_PORT,
            FIELD_AUDIT_TIMEOUT,
        ] {
            for invalid in ["", "abc", "-1", "0", "65536", "1.5"] {
                let mut inputs = config_inputs(&config);
                inputs[field] = invalid.into();
                assert_eq!(draft_config(&config, &inputs).unwrap_err().0, field);
            }
        }
        let mut inputs = config_inputs(&config);
        for invalid in ["", "abc", "-1", "4294967296"] {
            inputs[FIELD_EXEC_TIMEOUT] = invalid.into();
            assert_eq!(
                draft_config(&config, &inputs).unwrap_err().0,
                FIELD_EXEC_TIMEOUT
            );
        }
        inputs[FIELD_EXEC_TIMEOUT] = "0".into();
        assert_eq!(draft_config(&config, &inputs).unwrap().exec_timeout, 0);
    }

    #[test]
    fn toggles_reject_text_and_validate_boolean_values() {
        let mut app = app();
        for field in [
            FIELD_STRICT_HOST,
            FIELD_AUTO_SAVE,
            FIELD_MCP_READ_ONLY,
            FIELD_MCP_DLP_FILTER,
        ] {
            app.settings_focused_field = field;
            let previous = app.settings_inputs[field].clone();
            key(&mut app, KeyCode::Char('x'));
            key(&mut app, KeyCode::Backspace);
            assert_eq!(app.settings_inputs[field], previous);
            key(&mut app, KeyCode::Char(' '));
            assert_ne!(app.settings_inputs[field], previous);
        }
        app.settings_inputs[FIELD_AUTO_SAVE] = "maybe".into();
        assert_eq!(
            draft_config(&app.config, &app.settings_inputs)
                .unwrap_err()
                .0,
            FIELD_AUTO_SAVE
        );
    }

    #[test]
    fn save_failure_keeps_live_config_and_draft_then_success_commits() {
        let mut app = app();
        app.settings_inputs[FIELD_DEFAULT_PORT] = "2222".into();
        save_settings_with(&mut app, |_| anyhow::bail!("test write failure"));
        assert_eq!(app.config.default_port, 22);
        assert_eq!(app.settings_inputs[FIELD_DEFAULT_PORT], "2222");
        assert!(matches!(app.screen, Screen::Settings));
        assert!(app
            .settings_error
            .as_deref()
            .unwrap()
            .contains("test write failure"));
        save_settings_with(&mut app, |_| Ok(()));
        assert_eq!(app.config.default_port, 2222);
        assert!(matches!(app.screen, Screen::Main));
        assert!(app.settings_error.is_none());
    }

    #[test]
    fn invalid_save_focuses_field_without_calling_writer() {
        let mut app = app();
        app.settings_inputs[FIELD_MCP_PORT] = "oops".into();
        save_settings_with(&mut app, |_| panic!("invalid draft must not be saved"));
        assert_eq!(app.settings_focused_field, FIELD_MCP_PORT);
        assert_eq!(app.config.mcp_port, 8822);
        assert!(app.settings_error.is_some());
    }

    #[test]
    fn escape_offers_discard_or_continue_and_clean_exit_is_accurate() {
        let mut app = app();
        app.settings_inputs[FIELD_DEFAULT_USER] = "changed".into();
        key(&mut app, KeyCode::Esc);
        assert!(app.settings_confirm_discard);
        assert!(matches!(app.screen, Screen::Settings));
        key(&mut app, KeyCode::Esc);
        assert!(!app.settings_confirm_discard);
        assert_eq!(app.settings_inputs[FIELD_DEFAULT_USER], "changed");
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.screen, Screen::Main));
        assert!(app.config.default_user.is_none());
        app.screen = Screen::Settings;
        app.settings_inputs = config_inputs(&app.config);
        key(&mut app, KeyCode::Esc);
        assert_eq!(
            app.status_message.unwrap().text,
            "Settings closed; no changes."
        );
    }

    #[test]
    fn reveal_is_focus_local_and_resets_on_navigation_and_exit() {
        let mut app = app();
        key(&mut app, KeyCode::F(2));
        assert!(!app.settings_reveal_token);
        app.settings_focused_field = FIELD_MCP_TOKEN;
        key(&mut app, KeyCode::F(2));
        assert!(app.settings_reveal_token);
        key(&mut app, KeyCode::Tab);
        assert!(!app.settings_reveal_token);
        key(&mut app, KeyCode::BackTab);
        assert!(!app.settings_reveal_token);
        key(&mut app, KeyCode::F(2));
        key(&mut app, KeyCode::Esc);
        assert!(!app.settings_reveal_token);
    }

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn compact_render_keeps_focused_field_help_error_and_actions_visible() {
        let mut app = app();
        app.settings_focused_field = FIELD_AUDIT_TIMEOUT;
        app.settings_error = Some("Audit Timeout: enter a whole number from 1 to 65535.".into());
        for (width, height) in [(80, 24), (60, 20)] {
            let text = rendered(&app, width, height);
            assert!(text.contains("Audit Timeout"));
            assert!(text.contains("connection-check timeout"));
            assert!(text.contains("whole number"));
            assert!(text.contains("[Enter] save"));
            assert!(text.contains("[Esc] back"));
        }
    }

    #[test]
    fn token_never_renders_revealed_outside_its_focus_or_during_confirmation() {
        let mut app = app();
        app.config.mcp_token = Some("secret-token".into());
        app.settings_inputs = config_inputs(&app.config);
        app.settings_reveal_token = true;
        assert!(!rendered(&app, 80, 24).contains("secret-token"));
        app.settings_focused_field = FIELD_MCP_TOKEN;
        assert!(rendered(&app, 80, 24).contains("secret-token"));
        app.settings_confirm_discard = true;
        let text = rendered(&app, 60, 20);
        assert!(!text.contains("secret-token"));
        assert!(text.contains("[D] discard"));
        assert!(text.contains("[C/Esc] continue"));
    }
}
