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
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Stdout, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
pub mod proxy_screen;
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
        return_to_proxy: bool,
    },
    HostForm,    // overlay for add/edit host
    ImportHosts, // overlay for importing from INI file path
    McpServer,   // modal: MCP server status + log while it runs
    CopyId,      // overlay for ssh-copy-id (key multi-select + password)
    Proxy {
        host_idx: usize,
    }, // overlay for explicit proxy status, endpoints, logs, start/stop
}

#[derive(Debug, Clone)]
pub enum CopyIdEvent {
    Progress(String),
    Success(String),
    Failure(String),
}

const MAX_PROXY_LOG_LINES: usize = 300;
const MAX_PROXY_LOG_BYTES: usize = 64 * 1024;

fn now_hms() -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

#[derive(Default)]
pub struct ProxyStderrLog {
    lines: VecDeque<String>,
    total_bytes: usize,
}

impl ProxyStderrLog {
    pub fn push_line(&mut self, raw: String) {
        let line = raw.trim_end_matches('\r').to_string();
        if line.is_empty() {
            return;
        }
        self.total_bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > MAX_PROXY_LOG_LINES || self.total_bytes > MAX_PROXY_LOG_BYTES {
            if let Some(old) = self.lines.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(old.len());
            } else {
                break;
            }
        }
    }

    pub fn push_event(&mut self, tag: &str, msg: impl AsRef<str>) {
        let ts = now_hms();
        self.push_line(format!("[{ts}] [{tag}] {}", msg.as_ref()));
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.total_bytes = 0;
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    pub fn last_meaningful(&self) -> Option<String> {
        self.lines
            .iter()
            .rev()
            .find(|l| {
                let t = l.trim();
                !t.is_empty()
                    && !t.contains("debug1:")
                    && !t.contains("debug2:")
                    && !t.contains("debug3:")
                    && !t.contains("[proxy]")
                    && !t.contains("[http]")
                    && !t.contains("[socks]")
            })
            .or_else(|| self.lines.iter().rev().find(|l| !l.trim().is_empty()))
            .cloned()
    }
}

fn spawn_stderr_drain(stderr: Option<std::process::ChildStderr>) -> Arc<Mutex<ProxyStderrLog>> {
    let log = Arc::new(Mutex::new(ProxyStderrLog::default()));
    if let Some(stderr) = stderr {
        let log_bg = Arc::clone(&log);
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(text) => {
                        if is_noisy_ssh_debug(&text) {
                            continue;
                        }
                        let ts = now_hms();
                        if let Ok(mut guard) = log_bg.lock() {
                            guard.push_line(format!("[{ts}] [ssh] {text}"));
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
    log
}

fn is_noisy_ssh_debug(line: &str) -> bool {
    let t = line.trim();
    let Some(rest) = t.strip_prefix("debug1:") else {
        return false;
    };
    let rest = rest.trim();
    for noisy_prefix in [
        "kex:",
        "kex_ext_info",
        "SSH2_MSG_",
        "expecting SSH2_MSG_",
        "load_hostkeys:",
        "Reading configuration data",
        "/etc/ssh/ssh_config",
        "Applying options for",
        "Authenticator provider",
        "compat_banner:",
        "Local version string",
        "Remote protocol version",
        "ssh_packet_",
        "rekey ",
        "pledge:",
        "client_input_global_request:",
        "Requesting no-more-sessions",
        "Entering interactive session",
        "get_agent_identities:",
        "identity file ",
        "no identity pubkey",
        "loaded pubkey from",
    ] {
        if rest.starts_with(noisy_prefix) || rest.contains("Applying options for") {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone)]
pub struct ProxyConfigSummary {
    pub host_id: String,
    pub host_name: String,
    pub host_ip: String,
    pub host_port: u16,
    pub jump_label: Option<String>,
    pub cred_label: String,
    pub socks_port: Option<u16>,
    pub internal_socks_port: Option<u16>,
    pub http_proxy_port: Option<u16>,
    pub port_forwards: Vec<String>,
}

impl ProxyConfigSummary {
    pub fn effective_socks_port(&self) -> Option<u16> {
        self.socks_port.or(self.internal_socks_port)
    }

    pub fn bound_local_ports(&self) -> Vec<u16> {
        let mut ports: Vec<u16> = self
            .port_forwards
            .iter()
            .filter_map(|f| f.split_once(':'))
            .filter_map(|(p, _)| p.parse::<u16>().ok())
            .collect();
        if let Some(p) = self.effective_socks_port() {
            ports.push(p);
        }
        if let Some(p) = self.http_proxy_port {
            ports.push(p);
        }
        ports
    }

    pub fn short_endpoints(&self) -> String {
        let mut parts = Vec::new();
        if let Some(p) = self.socks_port {
            parts.push(format!("SOCKS5 127.0.0.1:{p}"));
        }
        if let Some(p) = self.http_proxy_port {
            parts.push(format!("HTTP 127.0.0.1:{p}"));
        }
        for fwd in &self.port_forwards {
            parts.push(format!("L {}", fwd.replace(':', "→")));
        }
        parts.join("  ·  ")
    }
}

pub struct ProxyStartup {
    pub summary: ProxyConfigSummary,
    child: Option<std::process::Child>,
    _askpass: Option<crate::ssh::AskpassGuard>,
    check_port: u16,
    check_is_socks: bool,
    deadline: Instant,
    pub started_at: Instant,
    pub log: Arc<Mutex<ProxyStderrLog>>,
}

impl Drop for ProxyStartup {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct ProxySession {
    pub host_id: String,
    pub summary: ProxyConfigSummary,
    child: std::process::Child,
    _askpass: Option<crate::ssh::AskpassGuard>,
    http: Option<http_proxy::HttpProxy>,
    pub started_at: Instant,
    pub log: Arc<Mutex<ProxyStderrLog>>,
}

impl ProxySession {
    pub fn http_metrics(&self) -> Option<http_proxy::ProxyMetricsSnapshot> {
        self.http.as_ref().map(|h| h.metrics().snapshot())
    }

    pub fn http_bound_port(&self) -> Option<u16> {
        self.http.as_ref().map(http_proxy::HttpProxy::port)
    }
}

impl Drop for ProxySession {
    fn drop(&mut self) {
        self.http.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone)]
pub struct ProxyErrorReport {
    pub host_id: String,
    pub message: String,
    pub stderr_lines: Vec<String>,
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
    pub host_form_from_audit: bool,
    pub host_form_return_to_proxy: Option<usize>,
    pub import_path_input: String,
    pub import_cursor: usize,
    pub import_export_mode: usize, // 0 = import INI, 1 = export INI, 2 = export ssh_config
    // MCP server (Some while running; screen is modal)
    pub mcp: Option<crate::mcp::McpServer>,
    pub mcp_scroll_offset: usize,
    // Proxy state
    pub proxy: Option<ProxySession>,
    pub proxy_starting: Option<ProxyStartup>,
    pub proxy_last_error: Option<ProxyErrorReport>,
    pub proxy_log_scroll: usize,
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
            host_form_from_audit: false,
            host_form_return_to_proxy: None,
            import_path_input: String::new(),
            import_cursor: 0,
            import_export_mode: 0,
            mcp: None,
            mcp_scroll_offset: 0,
            proxy: None,
            proxy_starting: None,
            proxy_last_error: None,
            proxy_log_scroll: 0,
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

    pub fn active_proxy_ports(&self) -> Vec<u16> {
        let mut ports = Vec::new();
        if let Some(ref p) = self.proxy {
            ports.extend(p.summary.bound_local_ports());
        }
        if let Some(ref s) = self.proxy_starting {
            ports.extend(s.summary.bound_local_ports());
        }
        ports
    }

    pub fn is_proxy_active_or_starting_for(&self, host_id: &str) -> bool {
        self.proxy.as_ref().is_some_and(|p| p.host_id == host_id)
            || self
                .proxy_starting
                .as_ref()
                .is_some_and(|s| s.summary.host_id == host_id)
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
        let mut hosts_changed = false;
        for host in &mut self.hosts {
            if host.jump_credential_id.as_deref() == Some(cred_id) {
                host.jump_credential_id = None;
                hosts_changed = true;
            }
        }
        if hosts_changed {
            self.save_hosts()?;
        }
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

    pub fn clear_pinned_credential(&mut self, host_id: &str) -> Result<()> {
        let mut changed = false;
        let mut records = self.server_records.clone();
        for record in &mut records {
            if record.host_id == host_id && record.default_credential_id.is_some() {
                record.default_credential_id = None;
                changed = true;
            }
        }
        if changed {
            crate::config::save_server_records(&records)?;
            self.server_records = records;
        }
        Ok(())
    }

    /// Delete a host by index, clearing dangling jump references, orphaned server records,
    /// and any active/starting proxy bound to the deleted host.
    pub fn delete_host(&mut self, idx: usize) -> Result<Option<String>> {
        if idx >= self.hosts.len() {
            return Ok(None);
        }
        let deleted_id = self.hosts[idx].id.clone();
        let deleted_name = self.hosts[idx].name.clone();

        if self.proxy.as_ref().is_some_and(|p| p.host_id == deleted_id) {
            self.proxy = None;
        }
        if self
            .proxy_starting
            .as_ref()
            .is_some_and(|s| s.summary.host_id == deleted_id)
        {
            self.proxy_starting = None;
        }
        for h in &mut self.hosts {
            if h.jump_host_id.as_deref() == Some(deleted_id.as_str()) {
                h.jump_host_id = None;
                h.jump_credential_id = None;
            }
        }
        self.hosts.remove(idx);
        self.save_hosts()?;

        let before = self.server_records.len();
        self.server_records.retain(|r| r.host_id != deleted_id);
        if self.server_records.len() != before {
            crate::config::save_server_records(&self.server_records)?;
        }

        self.selected_row = self.selected_row.min(self.hosts.len().saturating_sub(1));
        Ok(Some(deleted_name))
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

    pub fn poll_proxy(&mut self) {
        if let Some(mut starting) = self.proxy_starting.take() {
            let status_res = starting
                .child
                .as_mut()
                .map(std::process::Child::try_wait)
                .transpose();
            match status_res {
                Ok(Some(Some(status))) => {
                    std::thread::sleep(Duration::from_millis(30));
                    let (lines, detail) = read_stderr_summary(&starting.log);
                    let err_msg = match detail {
                        Some(d) => format!("{d} ({status})"),
                        None => format!("SSH proxy exited with {status}"),
                    };
                    self.proxy_last_error = Some(ProxyErrorReport {
                        host_id: starting.summary.host_id.clone(),
                        message: err_msg.clone(),
                        stderr_lines: lines,
                    });
                    self.status_message = Some(crate::types::StatusMessage::error(format!(
                        "Proxy failed ({}): {err_msg}",
                        starting.summary.host_name
                    )));
                    return;
                }
                Err(e) => {
                    let (lines, _) = read_stderr_summary(&starting.log);
                    let err_msg = format!("Failed to check SSH proxy process: {e}");
                    self.proxy_last_error = Some(ProxyErrorReport {
                        host_id: starting.summary.host_id.clone(),
                        message: err_msg.clone(),
                        stderr_lines: lines,
                    });
                    self.status_message = Some(crate::types::StatusMessage::error(err_msg));
                    return;
                }
                _ => {}
            }

            let ready = if starting.check_is_socks {
                match probe_socks5_ready(starting.check_port) {
                    Ok(r) => r,
                    Err(err_msg) => {
                        if let Ok(mut g) = starting.log.lock() {
                            g.push_event("proxy", format!("ERROR: {err_msg}"));
                        }
                        let (lines, _) = read_stderr_summary(&starting.log);
                        self.proxy_last_error = Some(ProxyErrorReport {
                            host_id: starting.summary.host_id.clone(),
                            message: err_msg.clone(),
                            stderr_lines: lines,
                        });
                        self.status_message = Some(crate::types::StatusMessage::error(format!(
                            "Proxy failed ({}): {err_msg}",
                            starting.summary.host_name
                        )));
                        return;
                    }
                }
            } else {
                probe_local_forward_ready(starting.check_port)
            };

            if ready {
                // Re-check child is still alive (guards against partial bind + ExitOnForwardFailure exit).
                if let Some(ref mut child) = starting.child {
                    if let Ok(Some(status)) = child.try_wait() {
                        std::thread::sleep(Duration::from_millis(30));
                        let (lines, detail) = read_stderr_summary(&starting.log);
                        let err_msg = match detail {
                            Some(d) => format!("{d} ({status})"),
                            None => format!("SSH proxy exited with {status}"),
                        };
                        self.proxy_last_error = Some(ProxyErrorReport {
                            host_id: starting.summary.host_id.clone(),
                            message: err_msg.clone(),
                            stderr_lines: lines,
                        });
                        self.status_message = Some(crate::types::StatusMessage::error(format!(
                            "Proxy failed ({}): {err_msg}",
                            starting.summary.host_name
                        )));
                        return;
                    }
                }

                if let Ok(mut g) = starting.log.lock() {
                    g.push_event(
                        "proxy",
                        format!(
                            "Tunnel ready on 127.0.0.1:{} ({} ms)",
                            starting.check_port,
                            starting.started_at.elapsed().as_millis()
                        ),
                    );
                }

                let http = match (
                    starting.summary.http_proxy_port,
                    starting.summary.effective_socks_port(),
                ) {
                    (Some(http_port), Some(socks_port)) => {
                        match http_proxy::HttpProxy::start_with_log(
                            http_port,
                            socks_port,
                            Some(Arc::clone(&starting.log)),
                        ) {
                            Ok(http) => Some(http),
                            Err(err) => {
                                let (lines, _) = read_stderr_summary(&starting.log);
                                let err_msg = format!("{err:#}");
                                self.proxy_last_error = Some(ProxyErrorReport {
                                    host_id: starting.summary.host_id.clone(),
                                    message: err_msg.clone(),
                                    stderr_lines: lines,
                                });
                                self.status_message =
                                    Some(crate::types::StatusMessage::error(format!(
                                        "HTTP proxy failed ({}): {err_msg}",
                                        starting.summary.host_name
                                    )));
                                // `starting` drops here, killing SSH child cleanly.
                                return;
                            }
                        }
                    }
                    _ => None,
                };

                if let Some(child) = starting.child.take() {
                    let host_id = starting.summary.host_id.clone();
                    let summary_text = starting.summary.short_endpoints();
                    let host_name = starting.summary.host_name.clone();
                    let session = ProxySession {
                        host_id,
                        summary: starting.summary.clone(),
                        child,
                        _askpass: starting._askpass.take(),
                        http,
                        started_at: Instant::now(),
                        log: Arc::clone(&starting.log),
                    };
                    self.proxy = Some(session);
                    self.proxy_last_error = None;
                    self.status_message = Some(crate::types::StatusMessage::success(format!(
                        "Proxy active ({host_name}): {summary_text}"
                    )));
                }
                return;
            }

            if Instant::now() >= starting.deadline {
                let (lines, detail) = read_stderr_summary(&starting.log);
                let err_msg = match detail {
                    Some(d) => format!("Timed out waiting for port {} — {d}", starting.check_port),
                    None => format!(
                        "Timed out waiting for SSH proxy on 127.0.0.1:{}",
                        starting.check_port
                    ),
                };
                self.proxy_last_error = Some(ProxyErrorReport {
                    host_id: starting.summary.host_id.clone(),
                    message: err_msg.clone(),
                    stderr_lines: lines,
                });
                self.status_message = Some(crate::types::StatusMessage::error(format!(
                    "Proxy failed ({}): {err_msg}",
                    starting.summary.host_name
                )));
                // `starting` drops here, killing SSH child and wiping askpass secrets.
                return;
            }

            self.proxy_starting = Some(starting);
        }

        if let Some(ref mut proxy) = self.proxy {
            match proxy.child.try_wait() {
                Ok(Some(status)) => {
                    std::thread::sleep(Duration::from_millis(20));
                    let (lines, detail) = read_stderr_summary(&proxy.log);
                    let host_id = proxy.host_id.clone();
                    let host_name = proxy.summary.host_name.clone();
                    let err_msg = match detail {
                        Some(d) => format!("{d} ({status})"),
                        None => format!("SSH tunnel exited with {status}"),
                    };
                    self.proxy = None;
                    self.proxy_last_error = Some(ProxyErrorReport {
                        host_id,
                        message: err_msg.clone(),
                        stderr_lines: lines,
                    });
                    self.status_message = Some(crate::types::StatusMessage::error(format!(
                        "Proxy disconnected ({host_name}): {err_msg}"
                    )));
                }
                Err(e) => {
                    let host_id = proxy.host_id.clone();
                    let host_name = proxy.summary.host_name.clone();
                    let (lines, _) = read_stderr_summary(&proxy.log);
                    let err_msg = format!("Proxy monitor error: {e}");
                    self.proxy = None;
                    self.proxy_last_error = Some(ProxyErrorReport {
                        host_id,
                        message: err_msg.clone(),
                        stderr_lines: lines,
                    });
                    self.status_message = Some(crate::types::StatusMessage::error(format!(
                        "Proxy disconnected ({host_name}): {err_msg}"
                    )));
                }
                Ok(None) => {}
            }
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

fn read_stderr_summary(log: &Arc<Mutex<ProxyStderrLog>>) -> (Vec<String>, Option<String>) {
    match log.lock() {
        Ok(guard) => (guard.snapshot(), guard.last_meaningful()),
        Err(_) => (Vec::new(), None),
    }
}

fn probe_socks5_ready(port: u16) -> Result<bool, String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else {
        return Ok(false);
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
    if stream.write_all(&[5, 1, 0]).is_err() {
        return Ok(false);
    }
    let mut resp = [0u8; 2];
    Ok(stream.read_exact(&mut resp).is_ok() && resp == [5, 0])
}

fn probe_local_forward_ready(port: u16) -> bool {
    match TcpListener::bind(("127.0.0.1", port)) {
        Ok(_listener) => false,
        Err(e) => e.kind() == io::ErrorKind::AddrInUse,
    }
}

fn allocate_ephemeral_port(avoid: &[u16]) -> Result<u16> {
    for _ in 0..8 {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|e| anyhow::anyhow!("Cannot allocate internal SOCKS port: {e}"))?;
        let port = listener.local_addr()?.port();
        drop(listener);
        if port != 0 && !avoid.contains(&port) {
            return Ok(port);
        }
    }
    anyhow::bail!("Failed to allocate a free internal local port for HTTP proxy")
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

pub fn stop_proxy(app: &mut App) {
    if let Some(starting) = app.proxy_starting.take() {
        let name = starting.summary.host_name.clone();
        drop(starting);
        app.status_message = Some(crate::types::StatusMessage::info(format!(
            "Proxy startup cancelled ({name})"
        )));
        return;
    }
    if let Some(proxy) = app.proxy.take() {
        let name = proxy.summary.host_name.clone();
        drop(proxy);
        app.status_message = Some(crate::types::StatusMessage::info(format!(
            "Proxy stopped ({name})"
        )));
    }
}

pub fn start_proxy(app: &mut App, host: &Host) -> Result<()> {
    let record = app.record_for(&host.id);
    let cred = crate::ssh::resolve_credential(
        &app.credentials,
        &app.config,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )?
    .ok_or_else(|| anyhow::anyhow!("Select a credential for this host with [R] first"))?
    .clone();
    let jump =
        crate::ssh::resolve_jump_auth(&app.hosts, host, &app.credentials, &app.server_records)?;

    anyhow::ensure!(
        !host.port_forwards.is_empty()
            || host.socks_port.is_some()
            || host.http_proxy_port.is_some(),
        "Configure forwards, SOCKS port, or HTTP proxy port first ([E] -> F2)"
    );
    crate::ssh::port_forward_args(&host.port_forwards, host.socks_port)?;

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

    let mut local_ports: Vec<u16> = host
        .port_forwards
        .iter()
        .filter_map(|f| f.split_once(':'))
        .filter_map(|(p, _)| p.parse::<u16>().ok())
        .collect();
    if let Some(p) = host.socks_port {
        local_ports.push(p);
    }
    if let Some(p) = host.http_proxy_port {
        local_ports.push(p);
    }

    // Check ports not held by our own active/starting proxy BEFORE stopping the current proxy.
    let own_ports = app.active_proxy_ports();
    for &port in &local_ports {
        if !own_ports.contains(&port) {
            TcpListener::bind(("127.0.0.1", port))
                .map_err(|e| anyhow::anyhow!("Local proxy port {port} is unavailable: {e}"))?;
        }
    }

    // Allocate an internal ephemeral SOCKS5 port if only HTTP proxy is configured.
    let internal_socks_port = if host.http_proxy_port.is_some() && host.socks_port.is_none() {
        Some(allocate_ephemeral_port(&local_ports)?)
    } else {
        None
    };
    let effective_socks = host.socks_port.or(internal_socks_port);

    let (check_port, check_is_socks) = if let Some(sp) = effective_socks {
        (sp, true)
    } else {
        let fwd_port = host
            .port_forwards
            .first()
            .and_then(|f| f.split_once(':'))
            .and_then(|(p, _)| p.parse::<u16>().ok())
            .ok_or_else(|| anyhow::anyhow!("Invalid port forward configuration"))?;
        (fwd_port, false)
    };

    // Stop any previous proxy now that validation succeeded.
    app.proxy = None;
    app.proxy_starting = None;
    app.proxy_last_error = None;
    app.proxy_log_scroll = usize::MAX;

    // Verify all ports (including any previously owned by our old proxy) are now free.
    for &port in &local_ports {
        let mut bound = false;
        for _ in 0..6 {
            if TcpListener::bind(("127.0.0.1", port)).is_ok() {
                bound = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        anyhow::ensure!(bound, "Local proxy port {port} is unavailable");
    }

    let (mut child, askpass) = crate::ssh::spawn_proxy_with_options(
        host,
        &cred,
        &app.config,
        jump.as_ref(),
        internal_socks_port,
    )?;
    let log = spawn_stderr_drain(child.stderr.take());

    let jump_label = host
        .jump_host_id
        .as_deref()
        .and_then(|jid| app.hosts.iter().find(|h| h.id == jid))
        .map(|jh| {
            if let Some(ref ja) = jump {
                if let Some(ref jc) = ja.cred {
                    return format!("{} ({}) via {}", jh.name, jh.ip, jc.name);
                }
            }
            format!("{} ({})", jh.name, jh.ip)
        });
    let cred_label = format!("{} ({})", cred.name, cred.username);

    if let Ok(mut g) = log.lock() {
        g.push_event(
            "proxy",
            format!(
                "Starting tunnel to {}:{} (user: {}, cred: {}, jump: {})",
                host.ip,
                host.port,
                crate::ssh::effective_username(host.user.as_deref(), &cred),
                cred_label,
                jump_label.as_deref().unwrap_or("none"),
            ),
        );
        if let Some(sp) = effective_socks {
            g.push_event("proxy", format!("SOCKS5 target listener: 127.0.0.1:{sp}"));
        }
        if let Some(hp) = host.http_proxy_port {
            g.push_event(
                "proxy",
                format!("HTTP/HTTPS target listener: 127.0.0.1:{hp}"),
            );
        }
        for fwd in &host.port_forwards {
            g.push_event("proxy", format!("Local forward: 127.0.0.1:{fwd}"));
        }
    }

    let summary = ProxyConfigSummary {
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        host_ip: host.ip.clone(),
        host_port: host.port,
        jump_label,
        cred_label,
        socks_port: host.socks_port,
        internal_socks_port,
        http_proxy_port: host.http_proxy_port,
        port_forwards: host.port_forwards.clone(),
    };

    let timeout_secs = (app.config.connect_timeout as u64).saturating_add(10);
    app.proxy_starting = Some(ProxyStartup {
        summary,
        child: Some(child),
        _askpass: askpass,
        check_port,
        check_is_socks,
        deadline: Instant::now() + Duration::from_secs(timeout_secs),
        started_at: Instant::now(),
        log,
    });
    app.status_message = Some(crate::types::StatusMessage::info(format!(
        "Starting proxy on {}…",
        host.name
    )));
    Ok(())
}

pub fn toggle_proxy(app: &mut App, host: &Host) -> Result<()> {
    if app.is_proxy_active_or_starting_for(&host.id) {
        stop_proxy(app);
        return Ok(());
    }
    start_proxy(app, host)
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
        Some(h) => {
            crate::ssh::resolve_jump_auth(&app.hosts, h, &app.credentials, &app.server_records)?
        }
        None => None,
    };

    restore_terminal(terminal)?;
    let active_ports = app.active_proxy_ports();
    let proxy_same_host = host
        .as_ref()
        .is_some_and(|h| app.is_proxy_active_or_starting_for(&h.id));
    let forwards: Vec<String> = host
        .as_ref()
        .filter(|_| !proxy_same_host)
        .map(|h| {
            h.port_forwards
                .iter()
                .filter(|f| {
                    f.split_once(':')
                        .and_then(|(p, _)| p.parse::<u16>().ok())
                        .is_none_or(|p| !active_ports.contains(&p))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let socks_port = host
        .as_ref()
        .filter(|_| !proxy_same_host)
        .and_then(|h| h.socks_port)
        .filter(|p| !active_ports.contains(p));
    let (status, stderr) = match crate::ssh::spawn_ssh_with_error_and_jump_auth(
        &ip,
        port,
        host.as_ref().and_then(|h| h.user.as_deref()),
        cred,
        &app.config,
        jump.as_ref(),
        &forwards,
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
                    return_to_proxy: false,
                };
            }
        }
        app.status_message = Some(crate::types::StatusMessage::error(match kind {
            crate::ssh::SshErrorKind::AuthFailed if jump.is_some() => {
                "SSH auth failed on target or jump host. Use [R] for target cred or [E] for jump cred."
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
    app.proxy_starting = None;
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
        app.poll_proxy();

        // Clear expired status messages automatically
        if let Some(ref msg) = app.status_message {
            if msg.is_expired() {
                app.status_message = None;
            }
        }

        terminal.draw(|f| draw(f, app))?;

        if crossterm::event::poll(Duration::from_millis(100))? {
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
        Screen::CredentialPicker {
            host_idx,
            return_to_proxy,
            ..
        } => {
            if *return_to_proxy {
                proxy_screen::draw_for_host(f, app, *host_idx);
            } else {
                main_screen::draw(f, app);
            }
            popup::draw(f, app);
        }
        Screen::HostForm => {
            if app.host_form_from_audit {
                audit_screen::draw(f, app);
            } else if let Some(host_idx) = app.host_form_return_to_proxy {
                proxy_screen::draw_for_host(f, app, host_idx);
            } else {
                main_screen::draw(f, app);
            }
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
        Screen::Proxy { .. } => proxy_screen::draw(f, app),
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
            return_to_proxy,
        } => popup::handle_key(terminal, app, key, host_idx, after_failure, return_to_proxy),
        Screen::HostForm => host_form::handle_key(terminal, app, key),
        Screen::ImportHosts => host_form::handle_import_key(terminal, app, key),
        Screen::McpServer => mcp_screen::handle_key(terminal, app, key),
        Screen::CopyId => copy_id::handle_key(terminal, app, key),
        Screen::Proxy { host_idx } => proxy_screen::handle_key(terminal, app, key, host_idx),
    }
}
