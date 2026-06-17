use std::io::{self, Stdout};
use anyhow::Result;
use crossterm::{execute, terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}};
use ratatui::{backend::CrosstermBackend, Terminal};
use crate::config::AppConfig;
use crate::types::{Credential, CredentialForm, DeleteKind, DeletePopup, Host, HostForm, ServerRecord};

pub mod credentials_screen;
pub mod delete_popup;
pub mod host_form;
pub mod main_screen;
pub mod popup;
pub mod settings_screen;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Main,
    Credentials,
    Settings,
    CredentialPicker { host_idx: usize, after_failure: bool },
    HostForm,      // overlay for add/edit host
    ImportHosts,   // overlay for importing from INI file path
}

pub struct App {
    pub screen: Screen,
    pub hosts: Vec<Host>,
    pub credentials: Vec<Credential>,
    pub config: AppConfig,
    pub server_records: Vec<ServerRecord>,
    // Main screen state
    pub search_query: String,
    pub selected_row: usize,
    pub search_focused: bool,
    // Credentials screen state
    pub cred_selected: usize,
    pub cred_form: Option<CredentialForm>,
    // Settings screen state
    pub settings_inputs: Vec<String>,   // 6 strings for the editable settings fields
    pub settings_focused_field: usize,
    // Popup state
    pub popup_selected: usize,
    // Host form / import state
    pub host_form: Option<HostForm>,
    pub import_path_input: String,
    // Delete confirmation popup
    pub delete_popup: Option<DeletePopup>,
    pub skip_delete_confirm: bool,
    // Global
    pub should_quit: bool,
    pub status_message: Option<String>,
}

impl App {
    pub fn new(hosts: Vec<Host>, credentials: Vec<Credential>, config: AppConfig, server_records: Vec<ServerRecord>) -> Self {
        Self {
            screen: Screen::Main,
            hosts,
            credentials,
            config,
            server_records,
            search_query: String::new(),
            selected_row: 0,
            search_focused: true,
            cred_selected: 0,
            cred_form: None,
            settings_inputs: Vec::new(),
            settings_focused_field: 0,
            popup_selected: 0,
            host_form: None,
            import_path_input: String::new(),
            delete_popup: None,
            skip_delete_confirm: false,
            should_quit: false,
            status_message: None,
        }
    }

    pub fn filtered_hosts(&self) -> Vec<&Host> {
        if self.search_query.is_empty() {
            return self.hosts.iter().collect();
        }
        let q = self.search_query.to_lowercase();
        self.hosts.iter().filter(|h| {
            h.name.to_lowercase().contains(&q)
                || h.group.to_lowercase().contains(&q)
                || h.ip.to_lowercase().contains(&q)
                || h.tags.iter().any(|t| t.to_lowercase().contains(&q))
                || h.description.as_deref().unwrap_or("").to_lowercase().contains(&q)
        }).collect()
    }

    pub fn last_credential_id(&self, host_name: &str) -> Option<&str> {
        self.server_records.iter()
            .find(|r| r.host_id == host_name)  // temporarily using name as host_id
            .and_then(|r| r.last_credential_id.as_deref())
    }

    pub fn save_last_credential(&mut self, host_name: &str, cred_id: &str) -> Result<()> {
        if let Some(r) = self.server_records.iter_mut().find(|r| r.host_id == host_name) {
            r.last_credential_id = Some(cred_id.to_string());
        } else {
            self.server_records.push(ServerRecord {
                host_id: host_name.to_string(),
                last_credential_id: Some(cred_id.to_string()),
            });
        }
        crate::config::save_server_records(&self.server_records)
    }

    pub fn save_hosts(&self) -> Result<()> {
        crate::config::save_hosts(&self.hosts)
    }

    pub fn reload_credentials(&mut self) -> Result<()> {
        self.credentials = crate::config::load_credentials()?;
        Ok(())
    }
}

pub fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

pub fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

/// Tear down TUI, run SSH, re-init TUI. Saves credential on success; shows picker on auth failure.
pub fn do_connect(terminal: &mut Term, app: &mut App, host_name: &str, cred: &Credential) -> Result<()> {
    let host = app.hosts.iter().find(|h| h.name == host_name || h.ip == host_name).cloned();
    let (ip, port) = host.as_ref().map(|h| (h.ip.clone(), h.port)).unwrap_or_else(|| (host_name.to_string(), 22));

    restore_terminal(terminal)?;
    let status = crate::ssh::spawn_ssh(&ip, port, cred, &app.config)?;
    *terminal = setup_terminal()?;

    if status.success() {
        app.save_last_credential(host_name, &cred.id)?;
        app.status_message = None;
    } else if status.code() == Some(255) {
        // Likely auth failure — show picker
        if let Some(idx) = app.hosts.iter().position(|h| h.name == host_name) {
            app.screen = Screen::CredentialPicker { host_idx: idx, after_failure: true };
        }
        app.status_message = Some("Authentication failed. Choose different credentials.".into());
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let cfg = crate::config::load_config()?;
    let hosts = crate::config::load_hosts()?;
    let credentials = crate::config::load_credentials()?;
    let records = crate::config::load_server_records()?;
    let mut terminal = setup_terminal()?;
    let mut app = App::new(hosts, credentials, cfg, records);
    let result = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(terminal: &mut Term, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                handle_key(terminal, app, key)?;
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    match &app.screen {
        Screen::Main => main_screen::draw(f, app),
        Screen::Credentials => credentials_screen::draw(f, app),
        Screen::Settings => settings_screen::draw(f, app),
        Screen::CredentialPicker { .. } => {
            main_screen::draw(f, app);
            popup::draw(f, app);
        }
        Screen::HostForm => {
            main_screen::draw(f, app);
            host_form::draw(f, app);
        }
        Screen::ImportHosts => {
            main_screen::draw(f, app);
            host_form::draw_import(f, app);
        }
    }
    // Delete confirmation popup renders on top of any screen
    if app.delete_popup.is_some() {
        delete_popup::draw(f, app);
    }
}

fn handle_key(terminal: &mut Term, app: &mut App, key: crossterm::event::KeyEvent) -> Result<()> {
    if app.delete_popup.is_some() {
        return delete_popup::handle_key(terminal, app, key);
    }
    let screen = app.screen.clone();
    match screen {
        Screen::Main => main_screen::handle_key(terminal, app, key),
        Screen::Credentials => credentials_screen::handle_key(terminal, app, key),
        Screen::Settings => settings_screen::handle_key(app, key),
        Screen::CredentialPicker { host_idx, after_failure } => {
            popup::handle_key(terminal, app, key, host_idx, after_failure)
        }
        Screen::HostForm => host_form::handle_key(terminal, app, key),
        Screen::ImportHosts => host_form::handle_import_key(terminal, app, key),
    }
}
