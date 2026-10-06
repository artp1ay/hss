use std::net::{TcpStream, ToSocketAddrs};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::AppConfig;
use crate::types::{Credential, CredentialKind, Host, ServerRecord};

#[derive(Debug, Clone, PartialEq)]
pub enum HostStatus {
    Pending,
    Checking,
    Ok,
    AuthFailed(String),
    Unreachable(String),
    PortClosed(String),
    NoCredential,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct HostAuditResult {
    pub host_id: String,
    pub host_name: String,
    pub ip: String,
    pub port: u16,
    pub group: String,
    pub status: HostStatus,
    pub latency_ms: Option<u64>,
    pub credential_name: Option<String>,
    pub detail: String,
}

pub enum AuditEvent {
    HostStarted(String),
    HostFinished(HostAuditResult),
    AllFinished,
}

pub struct AuditState {
    pub results: Vec<HostAuditResult>,
    pub selected: usize,
    pub is_running: bool,
    pub rx: Option<Receiver<AuditEvent>>,
    pub total: usize,
    pub completed: usize,
}

impl AuditState {
    pub fn new(hosts: &[Host]) -> Self {
        let results = hosts
            .iter()
            .map(|h| HostAuditResult {
                host_id: h.id.clone(),
                host_name: h.name.clone(),
                ip: h.ip.clone(),
                port: h.port,
                group: h.group.clone(),
                status: HostStatus::Pending,
                latency_ms: None,
                credential_name: None,
                detail: "Pending scan...".into(),
            })
            .collect();

        Self {
            results,
            selected: 0,
            is_running: false,
            rx: None,
            total: hosts.len(),
            completed: 0,
        }
    }

    pub fn start_all(
        &mut self,
        hosts: Vec<Host>,
        creds: Vec<Credential>,
        records: Vec<ServerRecord>,
        cfg: AppConfig,
    ) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.is_running = true;
        self.total = hosts.len();
        self.completed = 0;

        for r in &mut self.results {
            r.status = HostStatus::Pending;
            r.detail = "Queued...".into();
            r.latency_ms = None;
        }

        let hosts_arc = Arc::new(hosts);
        let creds_arc = Arc::new(creds);
        let records_arc = Arc::new(records);
        let cfg_arc = Arc::new(cfg);

        thread::spawn(move || {
            let mut handles = Vec::new();
            for i in 0..hosts_arc.len() {
                let tx_c = tx.clone();
                let hosts_c = Arc::clone(&hosts_arc);
                let creds_c = Arc::clone(&creds_arc);
                let records_c = Arc::clone(&records_arc);
                let cfg_c = Arc::clone(&cfg_arc);

                let handle = thread::spawn(move || {
                    let host = &hosts_c[i];
                    let _ = tx_c.send(AuditEvent::HostStarted(host.id.clone()));
                    let result = check_single_host(host, &hosts_c, &creds_c, &records_c, &cfg_c);
                    let _ = tx_c.send(AuditEvent::HostFinished(result));
                });
                handles.push(handle);
            }

            for h in handles {
                let _ = h.join();
            }
            let _ = tx.send(AuditEvent::AllFinished);
        });
    }

    pub fn start_single(
        &mut self,
        idx: usize,
        hosts: Vec<Host>,
        creds: Vec<Credential>,
        records: Vec<ServerRecord>,
        cfg: AppConfig,
    ) {
        if idx >= self.results.len() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.is_running = true;
        let host_id = self.results[idx].host_id.clone();
        self.results[idx].status = HostStatus::Checking;
        self.results[idx].detail = "Checking...".into();

        thread::spawn(move || {
            if let Some(host) = hosts.iter().find(|h| h.id == host_id) {
                let _ = tx.send(AuditEvent::HostStarted(host.id.clone()));
                let result = check_single_host(host, &hosts, &creds, &records, &cfg);
                let _ = tx.send(AuditEvent::HostFinished(result));
            }
            let _ = tx.send(AuditEvent::AllFinished);
        });
    }

    pub fn poll(&mut self) {
        if let Some(ref rx) = self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    AuditEvent::HostStarted(id) => {
                        if let Some(r) = self.results.iter_mut().find(|r| r.host_id == id) {
                            r.status = HostStatus::Checking;
                            r.detail = "Testing connection & auth...".into();
                        }
                    }
                    AuditEvent::HostFinished(res) => {
                        if let Some(r) = self.results.iter_mut().find(|r| r.host_id == res.host_id) {
                            *r = res;
                            self.completed += 1;
                        }
                    }
                    AuditEvent::AllFinished => {
                        self.is_running = false;
                    }
                }
            }
        }
        if !self.is_running {
            self.rx = None;
        }
    }
}

pub fn check_single_host(
    host: &Host,
    hosts: &[Host],
    creds: &[Credential],
    records: &[ServerRecord],
    cfg: &AppConfig,
) -> HostAuditResult {
    let timeout = Duration::from_secs(cfg.audit_timeout.max(1) as u64);
    let jump = crate::ssh::jump_spec(hosts, host);
    let has_jump = jump.is_some();

    let last_id = records
        .iter()
        .find(|r| r.host_id == host.id)
        .and_then(|r| r.last_credential_id.as_deref());
    let resolved_cred = crate::ssh::resolve_credential(creds, cfg, last_id).ok().flatten();
    let cred_name = resolved_cred.map(|c| c.name.clone());

    // 1. Connectivity test: TCP port probe (when direct)
    let (tcp_ok, latency_ms, tcp_err) = if !has_jump {
        let (res, rtt) = ping_tcp(&host.ip, host.port, timeout);
        match res {
            Ok(()) => (true, rtt, None),
            Err(err) => (false, None, Some(err)),
        }
    } else {
        // Behind jump host: rely on SSH tunnel through bastion
        (true, None, None)
    };

    if !tcp_ok {
        let err_msg = tcp_err.unwrap_or_else(|| "Connection failed".into());
        let status = if err_msg.contains("refused") {
            HostStatus::PortClosed(err_msg.clone())
        } else {
            HostStatus::Unreachable(err_msg.clone())
        };
        return HostAuditResult {
            host_id: host.id.clone(),
            host_name: host.name.clone(),
            ip: host.ip.clone(),
            port: host.port,
            group: host.group.clone(),
            status,
            latency_ms: None,
            credential_name: cred_name,
            detail: err_msg,
        };
    }

    // 2. Authentication test
    let cred = match resolved_cred {
        Some(c) => c,
        None => {
            return HostAuditResult {
                host_id: host.id.clone(),
                host_name: host.name.clone(),
                ip: host.ip.clone(),
                port: host.port,
                group: host.group.clone(),
                status: HostStatus::NoCredential,
                latency_ms,
                credential_name: None,
                detail: "Port open, but no credentials configured".into(),
            };
        }
    };

    let (status, detail, ssh_latency) = check_ssh_auth(host, cred, cfg, jump.as_deref(), timeout);
    let final_latency = latency_ms.or(ssh_latency);

    HostAuditResult {
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        ip: host.ip.clone(),
        port: host.port,
        group: host.group.clone(),
        status,
        latency_ms: final_latency,
        credential_name: Some(cred.name.clone()),
        detail,
    }
}

fn ping_tcp(host: &str, port: u16, timeout: Duration) -> (Result<(), String>, Option<u64>) {
    let start = Instant::now();
    let addr_str = format!("{}:{}", host, port);
    let addrs = match addr_str.to_socket_addrs() {
        Ok(mut iter) => match iter.next() {
            Some(a) => a,
            None => return (Err("DNS resolution failed".into()), None),
        },
        Err(e) => return (Err(format!("DNS error: {e}")), None),
    };

    match TcpStream::connect_timeout(&addrs, timeout) {
        Ok(_) => {
            let rtt = start.elapsed().as_millis() as u64;
            (Ok(()), Some(rtt))
        }
        Err(e) => {
            let msg = match e.kind() {
                std::io::ErrorKind::TimedOut => "Connection timed out".to_string(),
                std::io::ErrorKind::ConnectionRefused => "Connection refused (port closed)".to_string(),
                std::io::ErrorKind::HostUnreachable => "Host unreachable".to_string(),
                std::io::ErrorKind::NetworkUnreachable => "Network unreachable".to_string(),
                _ => format!("{e}"),
            };
            (Err(msg), None)
        }
    }
}

fn check_ssh_auth(
    host: &Host,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
    timeout: Duration,
) -> (HostStatus, String, Option<u64>) {
    let t0 = Instant::now();
    let password = if cred.kind == CredentialKind::Password {
        crate::credentials::get_password(&cred.id).ok()
    } else {
        None
    };

    let timeout_secs = timeout.as_secs().max(1).to_string();
    let mut args: Vec<String> = vec![
        "-o".into(), format!("StrictHostKeyChecking={}", cfg.strict_host_checking),
        "-o".into(), format!("ConnectTimeout={timeout_secs}"),
        "-o".into(), "NumberOfPasswordPrompts=1".into(),
        "-p".into(), host.port.to_string(),
        "-l".into(), cred.username.clone(),
    ];

    if let Some(ref key) = cred.key_path {
        args.extend_from_slice(&["-i".into(), key.clone()]);
    }
    if let Some(j) = jump {
        args.extend_from_slice(&["-J".into(), j.to_string()]);
    }
    if password.is_none() {
        args.extend_from_slice(&["-o".into(), "BatchMode=yes".into()]);
    }
    args.push(host.ip.clone());
    args.push("exit 0".into());

    let mut cmd = Command::new("ssh");
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let askpass_path = if let Some(ref pw) = password {
        if let Ok(p) = crate::ssh::write_askpass_helper() {
            let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
            cmd.env("SSH_ASKPASS", &p)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", display)
                .env("HSS_PASSWORD", pw);
            Some(p)
        } else {
            None
        }
    } else {
        None
    };

    let result = match cmd.spawn() {
        Ok(mut child) => {
            let deadline = Instant::now() + timeout + Duration::from_secs(1);
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let mut stderr_buf = Vec::new();
                        if let Some(mut err_pipe) = child.stderr.take() {
                            let _ = std::io::Read::read_to_end(&mut err_pipe, &mut stderr_buf);
                        }
                        break Ok((status, String::from_utf8_lossy(&stderr_buf).to_string()));
                    }
                    Ok(None) => {
                        if Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break Err("SSH timed out".to_string());
                        }
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(e) => break Err(format!("{e}")),
                }
            }
        }
        Err(e) => Err(format!("Failed to spawn ssh: {e}")),
    };

    if let Some(ref p) = askpass_path {
        let _ = std::fs::remove_file(p);
    }

    let elapsed_ms = t0.elapsed().as_millis() as u64;

    match result {
        Ok((status, stderr)) => {
            if status.success() {
                (HostStatus::Ok, "Connection and authentication successful".into(), Some(elapsed_ms))
            } else {
                let err_line = stderr
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .last()
                    .unwrap_or("Auth failed")
                    .trim()
                    .to_string();

                if err_line.contains("Permission denied") {
                    (HostStatus::AuthFailed("Permission denied (invalid key or password)".into()), err_line, Some(elapsed_ms))
                } else if err_line.contains("Host key verification failed") {
                    (HostStatus::AuthFailed("Host key verification failed".into()), err_line, Some(elapsed_ms))
                } else if err_line.contains("Connection timed out") || err_line.contains("Operation timed out") {
                    (HostStatus::Unreachable("Connection timed out".into()), err_line, None)
                } else if err_line.contains("Connection refused") {
                    (HostStatus::PortClosed("Connection refused".into()), err_line, None)
                } else if err_line.contains("No route to host") {
                    (HostStatus::Unreachable("No route to host".into()), err_line, None)
                } else {
                    (HostStatus::AuthFailed(err_line.clone()), err_line, Some(elapsed_ms))
                }
            }
        }
        Err(e) => (HostStatus::Unreachable(e.clone()), e, None),
    }
}
