use crate::config::AppConfig;
use crate::types::{Credential, CredentialForm, DeletePopup, Host, HostForm, ServerRecord};
use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

pub mod audit_screen;
pub mod copy_id;
pub mod credentials_screen;
pub mod delete_popup;
pub mod host_form;
mod http_proxy;
pub mod input;
pub mod main_screen;
pub mod mcp_screen;
pub mod popup;
pub mod settings_screen;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Main,
    Credentials,
    Settings,
    Audit,
    CredentialPicker {
        host_idx: usize,
        after_failure: bool,
    },
    HostForm,    // overlay for add/edit host
    ImportHosts, // overlay for importing from INI file path
    McpServer,   // modal: MCP server status + log while it runs
    CopyId,      // overlay for ssh-copy-id (key multi-select + password)
}

#[derive(Debug, Clone)]
pub enum CopyIdEvent {
    Progress(String),
    Success(String),
    Failure(String),
}

pub struct ProxySession {
    pub host_id: String,
    child: std::process::Child,
    askpass_path: Option<std::path::PathBuf>,
    http: Option<http_proxy::HttpProxy>,
}

impl Drop for ProxySession {
    fn drop(&mut self) {
        self.http.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(ref path) = self.askpass_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub struct App {
    pub screen: Screen,
    pub hosts: Vec<Host>,
    pub credentials: Vec<Credential>,
    pub config: AppConfig,
    pub server_records: Vec<ServerRecord>,
    // Audit state
    pub audit_state: Option<crate::audit::AuditState>,
    // Main screen state
    pub search_query: String,
    pub search_cursor: usize,
    pub selected_row: usize,
    pub search_focused: bool,
    // Credentials screen state
    pub cred_selected: usize,
    pub cred_form: Option<CredentialForm>,
    // Settings screen state
    pub settings_inputs: Vec<String>, // 6 strings for the editable settings fields
    pub settings_focused_field: usize,
    pub settings_cursor: usize, // char index inside the focused settings field
    pub settings_reveal_token: bool,
    // Popup state
    pub popup_selected: usize,
    // Host form / import state
    pub host_form: Option<HostForm>,
    pub import_path_input: String,
    pub import_cursor: usize,
    pub import_export_mode: usize, // 0 = import INI, 1 = export INI, 2 = export ssh_config
    // MCP server (Some while running; screen is modal)
    pub mcp: Option<crate::mcp::McpServer>,
    pub mcp_scroll_offset: usize,
    pub proxy: Option<ProxySession>,
    // ssh-copy-id overlay state
    pub copy_id_form: Option<crate::types::CopyIdForm>,
    pub copy_id_rx: Option<std::sync::mpsc::Receiver<CopyIdEvent>>,
    // Delete confirmation popup
    pub delete_popup: Option<DeletePopup>,
    pub skip_delete_confirm: bool,
    // Global
    pub should_quit: bool,
    pub status_message: Option<crate::types::StatusMessage>,
}

impl App {
    pub fn new(
        hosts: Vec<Host>,
        credentials: Vec<Credential>,
        config: AppConfig,
        server_records: Vec<ServerRecord>,
    ) -> Self {
        Self {
            screen: Screen::Main,
            hosts,
            credentials,
            config,
            server_records,
            audit_state: None,
            search_query: String::new(),
            search_cursor: 0,
            selected_row: 0,
            search_focused: false,
            cred_selected: 0,
            cred_form: None,
            settings_inputs: Vec::new(),
            settings_focused_field: 0,
            settings_cursor: 0,
            settings_reveal_token: false,
            popup_selected: 0,
            host_form: None,
            import_path_input: String::new(),
            import_cursor: 0,
            import_export_mode: 0,
            mcp: None,
            mcp_scroll_offset: 0,
            proxy: None,
            copy_id_form: None,
            copy_id_rx: None,
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
        self.hosts
            .iter()
            .filter(|h| {
                h.name.to_lowercase().contains(&q)
                    || h.group.to_lowercase().contains(&q)
                    || h.ip.to_lowercase().contains(&q)
                    || h.tags.iter().any(|t| t.to_lowercase().contains(&q))
                    || h.description
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
            })
            .collect()
    }

    pub fn record_for(&self, host_id: &str) -> Option<&ServerRecord> {
        self.server_records.iter().find(|r| r.host_id == host_id)
    }

    pub fn select_credential(&mut self, host_id: &str, cred_id: &str, pin: bool) -> Result<()> {
        let mut records = self.server_records.clone();
        if let Some(r) = records.iter_mut().find(|r| r.host_id == host_id) {
            if pin {
                r.default_credential_id = Some(cred_id.to_string());
            } else {
                r.last_credential_id = Some(cred_id.to_string());
            }
        } else {
            records.push(ServerRecord {
                host_id: host_id.to_string(),
                last_credential_id: (!pin).then(|| cred_id.to_string()),
                default_credential_id: pin.then(|| cred_id.to_string()),
            });
        }
        crate::config::save_server_records(&records)?;
        self.server_records = records;
        Ok(())
    }

    pub fn picker_selection(&self, host_id: &str) -> usize {
        let record = self.record_for(host_id);
        let id = [
            record.and_then(|r| r.default_credential_id.as_deref()),
            record.and_then(|r| r.last_credential_id.as_deref()),
            self.config.default_credential_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find(|id| self.credentials.iter().any(|c| c.id == *id));
        self.credentials
            .iter()
            .position(|c| Some(c.id.as_str()) == id)
            .unwrap_or(0)
    }

    pub fn clear_credential_references(&mut self, cred_id: &str) -> Result<()> {
        if !self.server_records.iter().any(|r| {
            r.last_credential_id.as_deref() == Some(cred_id)
                || r.default_credential_id.as_deref() == Some(cred_id)
        }) {
            return Ok(());
        }
        let mut records = self.server_records.clone();
        for record in &mut records {
            if record.last_credential_id.as_deref() == Some(cred_id) {
                record.last_credential_id = None;
            }
            if record.default_credential_id.as_deref() == Some(cred_id) {
                record.default_credential_id = None;
            }
        }
        crate::config::save_server_records(&records)?;
        self.server_records = records;
        Ok(())
    }

    pub fn save_hosts(&self) -> Result<()> {
        crate::config::save_hosts(&self.hosts)
    }

    pub fn reload_credentials(&mut self) -> Result<()> {
        self.credentials = crate::config::load_credentials()?;
        Ok(())
    }

    pub fn start_audit(&mut self) {
        let mut state = crate::audit::AuditState::new(&self.hosts);
        state.start_all(
            self.hosts.clone(),
            self.credentials.clone(),
            self.server_records.clone(),
            self.config.clone(),
        );
        self.audit_state = Some(state);
    }

    pub fn poll_audit(&mut self) {
        if let Some(ref mut state) = self.audit_state {
            state.poll();
        }
    }

    pub fn poll_copy_id(&mut self) {
        if let Some(ref rx) = self.copy_id_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    CopyIdEvent::Progress(msg) => {
                        if let Some(ref mut form) = self.copy_id_form {
                            form.progress_status = Some(msg);
                        }
                    }
                    CopyIdEvent::Success(msg) => {
                        self.status_message = Some(crate::types::StatusMessage::success(msg));
                        self.copy_id_form = None;
                        self.copy_id_rx = None;
                        self.screen = Screen::Main;
                        return;
                    }
                    CopyIdEvent::Failure(msg) => {
                        if let Some(ref mut form) = self.copy_id_form {
                            form.in_progress = false;
                            form.progress_status = None;
                            form.error_message = Some(msg.clone());
                        }
                        self.status_message = Some(crate::types::StatusMessage::error(msg));
                        self.copy_id_rx = None;
                        return;
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub struct TerminalGuard {
    raw_mode_enabled: bool,
    alternate_screen_entered: bool,
    cursor_hidden: bool,
    mouse_capture_enabled: bool,
}

impl TerminalGuard {
    pub fn new() -> Self {
        Self {
            raw_mode_enabled: false,
            alternate_screen_entered: false,
            cursor_hidden: false,
            mouse_capture_enabled: false,
        }
    }

    pub fn setup(&mut self) -> Result<()> {
        if !self.raw_mode_enabled {
            enable_raw_mode()?;
            self.raw_mode_enabled = true;
        }
        if !self.alternate_screen_entered {
            execute!(io::stdout(), EnterAlternateScreen)?;
            self.alternate_screen_entered = true;
        }
        if !self.mouse_capture_enabled {
            execute!(io::stdout(), EnableMouseCapture)?;
            self.mouse_capture_enabled = true;
        }
        Ok(())
    }

    pub fn restore(&mut self) -> Result<()> {
        let mut first_err = None;
        if self.mouse_capture_enabled {
            if let Err(e) = execute!(io::stdout(), DisableMouseCapture) {
                first_err = Some(e);
            } else {
                self.mouse_capture_enabled = false;
            }
        }
        if self.alternate_screen_entered {
            if let Err(e) = execute!(io::stdout(), LeaveAlternateScreen, Show) {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            } else {
                self.alternate_screen_entered = false;
                self.cursor_hidden = false;
            }
        }
        if self.raw_mode_enabled {
            if let Err(e) = disable_raw_mode() {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            } else {
                self.raw_mode_enabled = false;
            }
        }
        if let Some(e) = first_err {
            return Err(e.into());
        }
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

pub fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

pub fn restore_terminal(terminal: &mut Term) -> Result<()> {
    let _ = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    );
    let _ = disable_raw_mode();
    Ok(())
}

/// Computes an adaptive centered rectangle that switches to fullscreen
/// if the terminal window is too small for dialog percentages.
pub fn adaptive_centered_rect(
    percent_x: u16,
    percent_y: u16,
    min_width: u16,
    min_height: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    if area.width <= min_width || area.height <= min_height {
        return area;
    }

    let desired_width = (area.width * percent_x / 100)
        .max(min_width)
        .min(area.width);
    let desired_height = (area.height * percent_y / 100)
        .max(min_height)
        .min(area.height);

    let pad_x = (area.width.saturating_sub(desired_width)) / 2;
    let pad_y = (area.height.saturating_sub(desired_height)) / 2;

    ratatui::layout::Rect {
        x: area.x + pad_x,
        y: area.y + pad_y,
        width: desired_width,
        height: desired_height,
    }
}

pub fn wrap_hotkey_lines<'a>(
    pairs: &[(&'a str, &'a str)],
    max_width: u16,
) -> Vec<ratatui::text::Line<'a>> {
    use ratatui::style::{Color, Style};
    use ratatui::text::{Line, Span};

    let width = (max_width as usize).max(20);
    let mut lines = Vec::new();
    let mut current_spans = Vec::new();
    let mut current_len = 0;

    for (key, label) in pairs {
        let item_len =
            crate::tui::input::str_width(key) + 2 + 1 + crate::tui::input::str_width(label);
        let sep_len = if current_len > 0 { 2 } else { 0 };

        if current_len > 0 && current_len + sep_len + item_len > width {
            lines.push(Line::from(current_spans));
            current_spans = Vec::new();
            current_len = 0;
        }

        if current_len > 0 {
            current_spans.push(Span::raw("  "));
            current_len += 2;
        }

        current_spans.push(Span::styled(
            format!("[{key}]"),
            Style::default().fg(Color::Blue),
        ));
        current_spans.push(Span::styled(
            format!(" {label}"),
            Style::default().fg(Color::DarkGray),
        ));
        current_len += item_len;
    }

    if !current_spans.is_empty() {
        lines.push(Line::from(current_spans));
    }

    if lines.is_empty() {
        lines.push(Line::from(""));
    }

    lines
}

pub fn toggle_proxy(app: &mut App, host: &Host) -> Result<()> {
    if let Some(ref proxy) = app.proxy {
        if proxy.host_id == host.id {
            app.proxy = None;
            app.status_message = Some(crate::types::StatusMessage::info("Proxy stopped"));
            return Ok(());
        }
    }
    let record = app.record_for(&host.id);
    let cred = crate::ssh::resolve_credential(
        &app.credentials,
        &app.config,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )?
    .ok_or_else(|| anyhow::anyhow!("Select a credential for this host with R first"))?;
    let jump = crate::ssh::resolve_jump_spec(&app.hosts, host)?;
    if let Some(port) = host.http_proxy_port {
        crate::ssh::validate_port(port)?;
        anyhow::ensure!(
            host.socks_port != Some(port),
            "HTTP and SOCKS ports must differ"
        );
        anyhow::ensure!(
            !host
                .port_forwards
                .iter()
                .any(|f| f.split_once(':').and_then(|(p, _)| p.parse::<u16>().ok()) == Some(port)),
            "HTTP port conflicts with a local forward"
        );
    }
    anyhow::ensure!(
        host.http_proxy_port.is_none() || host.socks_port.is_some(),
        "HTTP proxy requires a SOCKS port"
    );
    let check_port = host
        .socks_port
        .or_else(|| {
            host.port_forwards
                .first()
                .and_then(|f| f.split_once(':'))
                .and_then(|(p, _)| p.parse::<u16>().ok())
        })
        .ok_or_else(|| anyhow::anyhow!("Configure forwards or a SOCKS port first"))?;
    app.proxy = None;
    for port in host
        .port_forwards
        .iter()
        .filter_map(|f| f.split_once(':'))
        .filter_map(|(p, _)| p.parse::<u16>().ok())
        .chain(host.socks_port)
    {
        std::net::TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| anyhow::anyhow!("Local proxy port {port} is unavailable: {e}"))?;
    }
    let (mut child, path) = crate::ssh::spawn_proxy(host, cred, &app.config, jump.as_deref())?;
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs(app.config.connect_timeout as u64 + 10);
    loop {
        if let Some(status) = child.try_wait()? {
            use std::io::Read;
            let mut error = String::new();
            if let Some(ref mut stderr) = child.stderr {
                let _ = stderr.read_to_string(&mut error);
            }
            if let Some(ref path) = path {
                let _ = std::fs::remove_file(path);
            }
            anyhow::bail!("SSH proxy exited with {status}: {}", error.trim());
        }
        if std::net::TcpStream::connect(("127.0.0.1", check_port)).is_ok() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            if let Some(ref path) = path {
                let _ = std::fs::remove_file(path);
            }
            anyhow::bail!("Timed out waiting for SSH proxy on port {check_port}");
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let http = match host.http_proxy_port {
        Some(port) => match http_proxy::HttpProxy::start(port, host.socks_port.unwrap()) {
            Ok(http) => Some(http),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(ref path) = path {
                    let _ = std::fs::remove_file(path);
                }
                return Err(err);
            }
        },
        None => None,
    };
    app.proxy = Some(ProxySession {
        host_id: host.id.clone(),
        child,
        askpass_path: path,
        http,
    });
    app.status_message = Some(crate::types::StatusMessage::success(format!(
        "Proxy active: {}",
        host.name
    )));
    Ok(())
}

/// Tear down TUI, run SSH, re-init TUI. Saves credential on success; shows picker on auth failure.
pub fn do_connect(
    terminal: &mut Term,
    app: &mut App,
    host_id: &str,
    cred: &Credential,
) -> Result<()> {
    let host = app.hosts.iter().find(|h| h.id == host_id).cloned();
    let (ip, port) = host
        .as_ref()
        .map(|h| (h.ip.clone(), h.port))
        .unwrap_or_else(|| (host_id.to_string(), 22));
    let jump = match host.as_ref() {
        Some(h) => crate::ssh::resolve_jump_spec(&app.hosts, h)?,
        None => None,
    };

    restore_terminal(terminal)?;
    let proxy_active = host
        .as_ref()
        .is_some_and(|h| app.proxy.as_ref().is_some_and(|p| p.host_id == h.id));
    let forwards = host
        .as_ref()
        .filter(|_| !proxy_active)
        .map(|h| h.port_forwards.as_slice())
        .unwrap_or(&[]);
    let socks_port = host
        .as_ref()
        .filter(|_| !proxy_active)
        .and_then(|h| h.socks_port);
    let (status, stderr) = match crate::ssh::spawn_ssh_with_error(
        &ip,
        port,
        cred,
        &app.config,
        jump.as_deref(),
        forwards,
        socks_port,
    ) {
        Ok(s) => s,
        Err(e) => {
            *terminal = setup_terminal()?;
            return Err(e);
        }
    };
    *terminal = setup_terminal()?;

    if status.success() {
        if app.config.auto_save_credential {
            if let Some(ref h) = host {
                app.select_credential(&h.id, &cred.id, false)?;
            }
        }
        app.status_message = None;
    } else {
        let kind = crate::ssh::classify_ssh_error(&stderr);
        if kind.is_auth_failure() && jump.is_none() {
            if let Some(idx) = host
                .as_ref()
                .and_then(|h| app.hosts.iter().position(|x| x.id == h.id))
            {
                app.screen = Screen::CredentialPicker {
                    host_idx: idx,
                    after_failure: true,
                };
            }
        }
        app.status_message = Some(crate::types::StatusMessage::error(match kind {
            crate::ssh::SshErrorKind::AuthFailed if jump.is_some() => {
                "SSH authentication failed on the target or jump host. Check both connections."
                    .into()
            }
            crate::ssh::SshErrorKind::Unknown(ref message) if message.is_empty() => {
                format!("SSH exited with {status}")
            }
            _ => kind.display_message(),
        }));
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let cfg = crate::config::load_config()?;
    let hosts = crate::config::load_hosts()?;
    let credentials = crate::config::load_credentials()?;
    let records =
        crate::config::migrate_server_records(crate::config::load_server_records()?, &hosts);

    let mut guard = TerminalGuard::new();
    guard.setup()?;

    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut app = App::new(hosts, credentials, cfg, records);
    let result = run_loop(&mut terminal, &mut app);
    app.proxy = None;
    if let Some(mcp) = app.mcp.take() {
        mcp.stop();
    }
    let _ = guard.restore();
    result
}

fn run_loop(terminal: &mut Term, app: &mut App) -> Result<()> {
    loop {
        app.poll_audit();
        app.poll_copy_id();
        if let Some(ref mut proxy) = app.proxy {
            if let Some(status) = proxy.child.try_wait()? {
                app.proxy = None;
                app.status_message = Some(crate::types::StatusMessage::error(format!(
                    "Proxy disconnected: {status}"
                )));
            }
        }

        // Clear expired status messages automatically
        if let Some(ref msg) = app.status_message {
            if msg.is_expired() {
                app.status_message = None;
            }
        }

        terminal.draw(|f| draw(f, app))?;

        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            match crossterm::event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let Err(e) = handle_key(terminal, app, key) {
                        app.status_message =
                            Some(crate::types::StatusMessage::error(format!("Error: {e:#}")));
                    }
                }
                Event::Mouse(mouse) if app.screen == Screen::HostForm => {
                    let size = terminal.size()?;
                    host_form::handle_mouse(
                        app,
                        mouse,
                        ratatui::layout::Rect::new(0, 0, size.width, size.height),
                    );
                }
                _ => {}
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
        Screen::Audit => audit_screen::draw(f, app),
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
        Screen::McpServer => mcp_screen::draw(f, app),
        Screen::CopyId => {
            main_screen::draw(f, app);
            copy_id::draw(f, app);
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
        Screen::Audit => audit_screen::handle_key(terminal, app, key),
        Screen::CredentialPicker {
            host_idx,
            after_failure,
        } => popup::handle_key(terminal, app, key, host_idx, after_failure),
        Screen::HostForm => host_form::handle_key(terminal, app, key),
        Screen::ImportHosts => host_form::handle_import_key(terminal, app, key),
        Screen::McpServer => mcp_screen::handle_key(terminal, app, key),
        Screen::CopyId => copy_id::handle_key(terminal, app, key),
    }
}
