use crate::config::{self, AppConfig};
use crate::credentials;
use crate::types::{Credential, CredentialKind, Host, ServerRecord};
use anyhow::Result;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::process::Command;

/// Build the ssh -J destination ("user@ip:port") for a host's jump host, if any.
///
/// Returns:
/// - `Ok(Some(spec))` if a valid jump host is configured and reachable.
/// - `Ok(None)` if no jump host is configured (`jump_host_id` is None or empty).
/// - `Err(...)` if a jump host is specified but cannot be found (dangling), is self-referential,
///   or has an invalid configuration (e.g. invalid host/IP/user/port).
pub fn resolve_jump_spec(hosts: &[Host], host: &Host) -> Result<Option<String>> {
    let Some(id) = host.jump_host_id.as_deref() else {
        return Ok(None);
    };
    if id.trim().is_empty() {
        return Ok(None);
    }
    if id == host.id {
        anyhow::bail!("Host '{}' references itself as jump host", host.name);
    }
    let jump = hosts.iter().find(|h| h.id == id).ok_or_else(|| {
        anyhow::anyhow!("Configured jump host (id: '{id}') was not found in inventory")
    })?;

    let mut spec = String::new();
    if let Some(ref user) = jump.user {
        validate_username(user)?;
        spec.push_str(user);
        spec.push('@');
    }
    validate_host(&jump.ip)?;
    validate_port(jump.port)?;
    spec.push_str(&jump.ip);
    if jump.port != 22 {
        spec.push_str(&format!(":{}", jump.port));
    }
    Ok(Some(spec))
}

/// Build the ssh -J destination ("user@ip:port") for a host's jump host, if any.
/// Backward-compatible helper: returns None if no jump host or if resolution fails.
pub fn jump_spec(hosts: &[Host], host: &Host) -> Option<String> {
    resolve_jump_spec(hosts, host).ok().flatten()
}

/// Resolved jump host connection target with optional dedicated authentication credential.
#[derive(Debug, Clone)]
pub struct JumpAuth {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub cred: Option<Credential>,
}

impl JumpAuth {
    /// Format as an OpenSSH `ProxyJump` target (`[user@]host[:port]`).
    pub fn spec(&self) -> String {
        let mut spec = String::new();
        if let Some(ref user) = self.user {
            spec.push_str(user);
            spec.push('@');
        }
        spec.push_str(&self.host);
        if self.port != 22 {
            spec.push_str(&format!(":{}", self.port));
        }
        spec
    }
}

/// Resolve the jump host and its dedicated credential, if configured.
///
/// Jump credential precedence:
/// 1. Explicit `host.jump_credential_id` chosen on the target host's edit card.
/// 2. The jump host's own pinned `default_credential_id` in `records`.
/// 3. The jump host's own `last_credential_id` in `records`.
/// 4. `None` (falls back to standard OpenSSH `-J` / agent / `~/.ssh/config`).
pub fn resolve_jump_auth(
    hosts: &[Host],
    host: &Host,
    creds: &[Credential],
    records: &[ServerRecord],
) -> Result<Option<JumpAuth>> {
    let Some(id) = host.jump_host_id.as_deref() else {
        return Ok(None);
    };
    if id.trim().is_empty() {
        return Ok(None);
    }
    if id == host.id {
        anyhow::bail!("Host '{}' references itself as jump host", host.name);
    }
    let jump = hosts.iter().find(|h| h.id == id).ok_or_else(|| {
        anyhow::anyhow!("Configured jump host (id: '{id}') was not found in inventory")
    })?;

    validate_host(&jump.ip)?;
    validate_port(jump.port)?;

    let jump_record = records.iter().find(|r| r.host_id == jump.id);
    let jump_cred = [
        host.jump_credential_id.as_deref(),
        jump_record.and_then(|r| r.default_credential_id.as_deref()),
        jump_record.and_then(|r| r.last_credential_id.as_deref()),
    ]
    .into_iter()
    .flatten()
    .find_map(|cid| creds.iter().find(|c| c.id == cid))
    .cloned();

    let user = if let Some(ref c) = jump_cred {
        let u = effective_username(jump.user.as_deref(), c);
        validate_username(u)?;
        Some(u.to_string())
    } else if let Some(ref u) = jump.user {
        let trimmed = u.trim();
        if trimmed.is_empty() {
            None
        } else {
            validate_username(trimmed)?;
            Some(trimmed.to_string())
        }
    } else {
        None
    };

    Ok(Some(JumpAuth {
        host: jump.ip.clone(),
        port: jump.port,
        user,
        cred: jump_cred,
    }))
}

/// High-level classification of SSH connection failure reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SshErrorKind {
    AuthFailed,
    HostKeyMismatch,
    ConnectionTimeout,
    ConnectionRefused,
    DnsError,
    NoRoute,
    Unknown(String),
}

impl SshErrorKind {
    pub fn is_auth_failure(&self) -> bool {
        matches!(self, Self::AuthFailed)
    }

    pub fn display_message(&self) -> String {
        match self {
            Self::AuthFailed => "Authentication failed. Choose different credentials.".into(),
            Self::HostKeyMismatch => {
                "Host key verification failed. Review server fingerprint.".into()
            }
            Self::ConnectionTimeout => {
                "Connection timed out. Check host address, VPN or firewall.".into()
            }
            Self::ConnectionRefused => "Connection refused. Check SSH service and port.".into(),
            Self::DnsError => "DNS lookup failed. Hostname cannot be resolved.".into(),
            Self::NoRoute => "No route to host. Check network connectivity.".into(),
            Self::Unknown(err) => {
                if err.is_empty() {
                    "SSH connection failed (unknown error).".into()
                } else {
                    format!("SSH connection failed: {err}")
                }
            }
        }
    }
}

/// Classifies an SSH error message from stderr into an `SshErrorKind`.
pub fn classify_ssh_error(stderr: &str) -> SshErrorKind {
    let err_line = stderr
        .lines()
        .rfind(|l| !l.trim().is_empty() && !l.trim().starts_with("Connection to "))
        .unwrap_or("")
        .trim();

    let lower = err_line.to_lowercase();
    if lower.contains("permission denied") {
        SshErrorKind::AuthFailed
    } else if lower.contains("host key verification failed") {
        SshErrorKind::HostKeyMismatch
    } else if lower.contains("connection timed out") || lower.contains("operation timed out") {
        SshErrorKind::ConnectionTimeout
    } else if lower.contains("connection refused") {
        SshErrorKind::ConnectionRefused
    } else if lower.contains("name or service not known")
        || lower.contains("could not resolve hostname")
        || lower.contains("nodename nor servname provided")
    {
        SshErrorKind::DnsError
    } else if lower.contains("no route to host") {
        SshErrorKind::NoRoute
    } else {
        SshErrorKind::Unknown(err_line.to_string())
    }
}
/// Parse a string of arguments taking double and single quotes into account.
pub fn parse_extra_args(input: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for c in input.chars() {
        if escaped {
            current.push(c);
            escaped = false;
        } else if c == '\\' && !in_single {
            escaped = true;
        } else if c == '\'' && !in_double {
            in_single = !in_single;
        } else if c == '"' && !in_single {
            in_double = !in_double;
        } else if c.is_whitespace() && !in_single && !in_double {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        args.push(current);
    }
    args
}

pub fn validate_host(host: &str) -> Result<()> {
    let host = host.trim();
    if host.is_empty() {
        anyhow::bail!("Host cannot be empty");
    }
    if host.starts_with('-') {
        anyhow::bail!("Host cannot start with '-': '{host}'");
    }
    if host
        .chars()
        .any(|c| c.is_ascii_control() || c.is_whitespace())
    {
        anyhow::bail!("Host contains whitespace or control characters: '{host}'");
    }

    // Check if it's a valid IPv4 address
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return Ok(());
    }

    // Check if it's a valid IPv6 address (e.g. 2001:db8::1 or [2001:db8::1])
    let unbracketed = if host.starts_with('[') && host.ends_with(']') {
        &host[1..host.len() - 1]
    } else {
        host
    };
    if unbracketed.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok(());
    }

    // Check if it's a valid hostname (RFC 1123, plus underscores for internal host aliases)
    if host.len() > 255 {
        anyhow::bail!("Host name is too long (> 255 characters): '{host}'");
    }

    for label in host.split('.') {
        if label.is_empty() {
            anyhow::bail!("Host contains empty label: '{host}'");
        }
        if label.len() > 63 {
            anyhow::bail!("Host label is too long (> 63 characters): '{label}'");
        }
        if label.starts_with('-') || label.ends_with('-') {
            anyhow::bail!("Host label cannot start or end with '-': '{label}'");
        }
        if !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            anyhow::bail!("Host label contains invalid characters: '{label}'");
        }
    }

    Ok(())
}

/// Validates an SSH username.
///
/// Disallows leading hyphens, spaces, control characters, and separators (`@`, `:`, `/`).
pub fn validate_username(user: &str) -> Result<()> {
    let user = user.trim();
    if user.is_empty() {
        anyhow::bail!("Username cannot be empty");
    }
    if user.starts_with('-') {
        anyhow::bail!("Username cannot start with '-': '{user}'");
    }
    if user
        .chars()
        .any(|c| c.is_ascii_control() || c.is_whitespace() || c == '@' || c == ':' || c == '/')
    {
        anyhow::bail!(
            "Username contains invalid characters ('@', ':', '/', whitespace or control): '{user}'"
        );
    }
    if !user
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
    {
        anyhow::bail!("Username contains invalid characters: '{user}'");
    }
    Ok(())
}

/// Validates an SSH port number (1..=65535).
pub fn validate_port(port: u16) -> Result<()> {
    if port == 0 {
        anyhow::bail!("Port must be between 1 and 65535 (got 0)");
    }
    Ok(())
}

/// Validates an SSH jump specification (e.g. `user@host:port` or `host` or `host1,host2`).
pub fn validate_jump_spec(jump: &str) -> Result<()> {
    let jump = jump.trim();
    if jump.is_empty() {
        anyhow::bail!("Jump spec cannot be empty");
    }
    if jump.starts_with('-') {
        anyhow::bail!("Jump spec cannot start with '-': '{jump}'");
    }
    if jump
        .chars()
        .any(|c| c.is_ascii_control() || c.is_whitespace())
    {
        anyhow::bail!("Jump spec contains whitespace or control characters: '{jump}'");
    }

    for hop in jump.split(',') {
        let hop = hop.trim();
        if hop.is_empty() {
            anyhow::bail!("Jump hop cannot be empty in '{jump}'");
        }
        if hop.starts_with('-') {
            anyhow::bail!("Jump hop cannot start with '-': '{hop}'");
        }

        let (user_opt, host_port) = match hop.split_once('@') {
            Some((u, hp)) => (Some(u), hp),
            None => (None, hop),
        };

        if let Some(user) = user_opt {
            validate_username(user)?;
        }

        let (host_part, port_opt) = if host_port.starts_with('[') {
            match host_port.find(']') {
                Some(bracket_end) => {
                    let ipv6 = &host_port[0..=bracket_end];
                    let rest = &host_port[bracket_end + 1..];
                    if let Some(rest) = rest.strip_prefix(':') {
                        (ipv6, Some(rest))
                    } else if rest.is_empty() {
                        (ipv6, None)
                    } else {
                        anyhow::bail!("Invalid jump host format: '{hop}'");
                    }
                }
                None => anyhow::bail!("Unmatched bracket in jump IPv6: '{hop}'"),
            }
        } else {
            match host_port.rfind(':') {
                Some(idx) => (&host_port[..idx], Some(&host_port[idx + 1..])),
                None => (host_port, None),
            }
        };

        validate_host(host_part)?;

        if let Some(port_str) = port_opt {
            let port: u16 = port_str
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid port '{port_str}' in jump hop '{hop}'"))?;
            validate_port(port)?;
        }
    }

    Ok(())
}

pub fn resolve_credential<'a>(
    creds: &'a [Credential],
    cfg: &AppConfig,
    last_id: Option<&str>,
    server_default_id: Option<&str>,
) -> Result<Option<&'a Credential>> {
    for id in [
        server_default_id,
        last_id,
        cfg.default_credential_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(c) = creds.iter().find(|c| c.id == id) {
            return Ok(Some(c));
        }
    }
    if creds.len() == 1 {
        return Ok(Some(&creds[0]));
    }
    Ok(None)
}

pub fn port_forward_args(forwards: &[String], socks_port: Option<u16>) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut local_ports = std::collections::HashSet::new();
    for forward in forwards {
        let (local, remote) = forward.split_once(':').ok_or_else(|| {
            anyhow::anyhow!("Invalid port forward '{forward}': expected local:remote")
        })?;
        let local: u16 = local
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid local port in '{forward}'"))?;
        let remote: u16 = remote
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid remote port in '{forward}'"))?;
        validate_port(local)?;
        validate_port(remote)?;
        anyhow::ensure!(
            local_ports.insert(local),
            "Local port {local} is configured more than once"
        );
        args.extend(["-L".into(), format!("127.0.0.1:{local}:127.0.0.1:{remote}")]);
    }
    if let Some(port) = socks_port {
        validate_port(port)?;
        anyhow::ensure!(
            local_ports.insert(port),
            "Local port {port} is configured more than once"
        );
        args.extend(["-D".into(), format!("127.0.0.1:{port}")]);
    }
    if !args.is_empty() {
        args.extend(["-o".into(), "ExitOnForwardFailure=yes".into()]);
    }
    Ok(args)
}

/// Resolve the effective SSH username for a host + credential pair.
/// For Key credentials, an explicit `host.user` overrides `cred.username` so one key can be
/// reused across hosts with different accounts. For Password credentials, the password belongs
/// to `cred.username`, so `cred.username` is always authoritative.
pub fn effective_username<'a>(host_user: Option<&'a str>, cred: &'a Credential) -> &'a str {
    if cred.kind == CredentialKind::Key {
        if let Some(u) = host_user.map(str::trim).filter(|u| !u.is_empty()) {
            return u;
        }
    }
    cred.username.trim()
}

/// RAII guard for private temporary askpass/proxy helpers and secret files.
/// Overwrites and deletes all secret files, helper scripts, and the private directory on drop.
#[derive(Debug)]
pub struct AskpassGuard {
    dir: std::path::PathBuf,
    script_path: std::path::PathBuf,
    secret_paths: Vec<std::path::PathBuf>,
    extra_files: Vec<std::path::PathBuf>,
}

impl AskpassGuard {
    pub fn script_path(&self) -> &std::path::Path {
        &self.script_path
    }
}

impl Drop for AskpassGuard {
    fn drop(&mut self) {
        for secret_path in &self.secret_paths {
            if let Ok(meta) = std::fs::symlink_metadata(secret_path) {
                if meta.is_file() {
                    let len = meta.len() as usize;
                    if len > 0 {
                        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(secret_path)
                        {
                            let zeros = vec![0u8; len.min(64 * 1024)];
                            let _ = f.write_all(&zeros);
                            let _ = f.sync_all();
                        }
                    }
                }
            }
            let _ = std::fs::remove_file(secret_path);
        }
        if !self.script_path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.script_path);
        }
        for file in &self.extra_files {
            let _ = std::fs::remove_file(file);
        }
        let _ = std::fs::remove_dir(&self.dir);
    }
}

pub(crate) enum JumpInput<'a> {
    Raw(&'a str),
    Auth(&'a JumpAuth),
}

pub(crate) fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[allow(clippy::too_many_arguments)]
fn ssh_command(
    host: &str,
    port: u16,
    host_user: Option<&str>,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<JumpInput<'_>>,
    forwards: &[String],
    socks_port: Option<u16>,
    proxy: bool,
) -> Result<(Command, Option<AskpassGuard>)> {
    validate_host(host)?;
    validate_port(port)?;
    let username = effective_username(host_user, cred);
    validate_username(username)?;
    let strict_host_checking = config::validate_strict_host_checking(&cfg.strict_host_checking)?;
    let secret = credentials::get_secret(cred)?;
    let mut args = vec![
        "-o".into(),
        format!("StrictHostKeyChecking={strict_host_checking}"),
        "-o".into(),
        format!("ConnectTimeout={}", cfg.connect_timeout),
        "-p".into(),
        port.to_string(),
        "-l".into(),
        username.to_string(),
    ];
    append_auth_args(&mut args, cred, secret.is_some(), !proxy);

    let mut cmd = Command::new("ssh");
    let askpass = configure_jump_and_askpass(
        &mut cmd,
        &mut args,
        &cred.kind,
        secret.as_deref(),
        jump,
        strict_host_checking,
        cfg.connect_timeout as u64,
        !proxy,
        false,
    )?;

    args.extend(port_forward_args(forwards, socks_port)?);
    if proxy {
        args.extend([
            "-N".into(),
            "-o".into(),
            "ServerAliveInterval=15".into(),
            "-o".into(),
            "ServerAliveCountMax=2".into(),
        ]);
    }
    if !cfg.ssh_extra_args.is_empty() {
        args.extend(parse_extra_args(&cfg.ssh_extra_args));
    }
    args.extend(["--".into(), host.to_string()]);
    cmd.args(args);
    Ok((cmd, askpass))
}

pub fn spawn_proxy(
    host: &Host,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
) -> Result<(std::process::Child, Option<AskpassGuard>)> {
    spawn_proxy_internal(host, cred, cfg, jump.map(JumpInput::Raw))
}

pub fn spawn_proxy_with_jump_auth(
    host: &Host,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&JumpAuth>,
) -> Result<(std::process::Child, Option<AskpassGuard>)> {
    spawn_proxy_internal(host, cred, cfg, jump.map(JumpInput::Auth))
}

fn spawn_proxy_internal(
    host: &Host,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<JumpInput<'_>>,
) -> Result<(std::process::Child, Option<AskpassGuard>)> {
    anyhow::ensure!(
        !host.port_forwards.is_empty() || host.socks_port.is_some(),
        "Configure forwards or a SOCKS port in the host card first"
    );
    let (mut cmd, askpass) = ssh_command(
        &host.ip,
        host.port,
        host.user.as_deref(),
        cred,
        cfg,
        jump,
        &host.port_forwards,
        host.socks_port,
        true,
    )?;
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let child = cmd.spawn()?;
    Ok((child, askpass))
}

pub fn spawn_ssh(
    host: &str,
    port: u16,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<std::process::ExitStatus> {
    spawn_ssh_with_user(host, port, None, cred, cfg, jump, forwards, socks_port)
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_ssh_with_user(
    host: &str,
    port: u16,
    host_user: Option<&str>,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<std::process::ExitStatus> {
    Ok(
        spawn_ssh_with_error_and_user(
            host, port, host_user, cred, cfg, jump, forwards, socks_port,
        )?
        .0,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_ssh_with_jump_auth(
    host: &str,
    port: u16,
    host_user: Option<&str>,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&JumpAuth>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<std::process::ExitStatus> {
    Ok(spawn_ssh_with_error_and_jump_auth(
        host, port, host_user, cred, cfg, jump, forwards, socks_port,
    )?
    .0)
}

pub fn spawn_ssh_with_error(
    host: &str,
    port: u16,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<(std::process::ExitStatus, String)> {
    spawn_ssh_with_error_and_user(host, port, None, cred, cfg, jump, forwards, socks_port)
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_ssh_with_error_and_user(
    host: &str,
    port: u16,
    host_user: Option<&str>,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<(std::process::ExitStatus, String)> {
    spawn_ssh_with_error_internal(
        host,
        port,
        host_user,
        cred,
        cfg,
        jump.map(JumpInput::Raw),
        forwards,
        socks_port,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_ssh_with_error_and_jump_auth(
    host: &str,
    port: u16,
    host_user: Option<&str>,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&JumpAuth>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<(std::process::ExitStatus, String)> {
    spawn_ssh_with_error_internal(
        host,
        port,
        host_user,
        cred,
        cfg,
        jump.map(JumpInput::Auth),
        forwards,
        socks_port,
    )
}

#[allow(clippy::too_many_arguments)]
fn spawn_ssh_with_error_internal(
    host: &str,
    port: u16,
    host_user: Option<&str>,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<JumpInput<'_>>,
    forwards: &[String],
    socks_port: Option<u16>,
) -> Result<(std::process::ExitStatus, String)> {
    use std::io::{Read, Write};
    let (mut cmd, _askpass) = ssh_command(
        host, port, host_user, cred, cfg, jump, forwards, socks_port, false,
    )?;
    cmd.stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::piped());
    let old_sigint = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let result = (|| -> std::io::Result<_> {
        let mut child = cmd.spawn()?;
        let mut stderr = child.stderr.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut captured = Vec::new();
            let mut buf = [0; 4096];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let _ = std::io::stderr().write_all(&buf[..n]);
                captured.extend_from_slice(&buf[..n]);
            }
            captured
        });
        let status = child.wait()?;
        let captured = reader.join().unwrap_or_default();
        Ok((status, String::from_utf8_lossy(&captured).into_owned()))
    })();
    unsafe {
        libc::signal(libc::SIGINT, old_sigint);
    }
    Ok(result?)
}

/// Run a single command on a host non-interactively, capturing output.
/// Host is looked up by name or IP; credentials resolve the same way as interactive connect.
/// Respects `exec_timeout` from config (0 = no limit, default 300s).
pub fn exec_command(host_query: &str, command: &str) -> Result<std::process::Output> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;
    let hosts = config::load_hosts()?;

    let h = hosts
        .iter()
        .find(|h| h.name == host_query || h.ip == host_query)
        .ok_or_else(|| anyhow::anyhow!("Host not found: {host_query}"))?;
    let records = config::migrate_server_records(records, &hosts);
    let record = records.iter().find(|r| r.host_id == h.id);
    let cred = resolve_credential(
        &creds,
        &cfg,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )?
    .ok_or_else(|| anyhow::anyhow!("No credential resolved for host '{}'", h.name))?;

    validate_host(&h.ip)?;
    validate_port(h.port)?;
    let username = effective_username(h.user.as_deref(), cred);
    validate_username(username)?;
    let strict_host_checking = config::validate_strict_host_checking(&cfg.strict_host_checking)?;
    let jump = resolve_jump_auth(&hosts, h, &creds, &records)?;

    let secret = credentials::get_secret(cred)?;

    let mut ssh_args: Vec<String> = vec![
        "-o".into(),
        format!("StrictHostKeyChecking={strict_host_checking}"),
        "-o".into(),
        format!("ConnectTimeout={}", cfg.connect_timeout),
        // Detect dead connections: probe every 15s, give up after 3 missed replies.
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-p".into(),
        h.port.to_string(),
        "-l".into(),
        username.to_string(),
    ];
    append_auth_args(&mut ssh_args, cred, secret.is_some(), false);

    let mut cmd = Command::new("ssh");
    let _askpass = configure_jump_and_askpass(
        &mut cmd,
        &mut ssh_args,
        &cred.kind,
        secret.as_deref(),
        jump.as_ref().map(JumpInput::Auth),
        strict_host_checking,
        cfg.connect_timeout as u64,
        false,
        false,
    )?;

    ssh_args.push("--".into());
    ssh_args.push(h.ip.clone());
    ssh_args.push(command.to_string());

    cmd.args(&ssh_args).stdin(std::process::Stdio::null());

    let timeout = cfg.exec_timeout;
    let out = if timeout == 0 {
        cmd.output()
    } else {
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        match cmd.spawn() {
            Ok(mut child) => {
                // Drain stdout and stderr in background threads to prevent pipe-full deadlock.
                // Cap collected output to 1 MiB each to prevent memory exhaustion.
                const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
                let stdout_pipe = child.stdout.take();
                let stderr_pipe = child.stderr.take();
                let stdout_handle = std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    if let Some(r) = stdout_pipe {
                        let _ = std::io::Read::read_to_end(
                            &mut std::io::Read::take(r, MAX_OUTPUT_BYTES),
                            &mut buf,
                        );
                    }
                    buf
                });
                let stderr_handle = std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    if let Some(r) = stderr_pipe {
                        let _ = std::io::Read::read_to_end(
                            &mut std::io::Read::take(r, MAX_OUTPUT_BYTES),
                            &mut buf,
                        );
                    }
                    buf
                });
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_secs(timeout as u64);
                let status = loop {
                    match child.try_wait() {
                        Ok(Some(s)) => break Ok(s),
                        Ok(None) => {
                            if std::time::Instant::now() >= deadline {
                                let _ = child.kill();
                                let _ = child.wait();
                                break Err(std::io::Error::new(
                                    std::io::ErrorKind::TimedOut,
                                    format!("command timed out after {timeout}s"),
                                ));
                            }
                            std::thread::sleep(std::time::Duration::from_millis(250));
                        }
                        Err(e) => break Err(e),
                    }
                };
                let stdout_bytes = stdout_handle.join().unwrap_or_default();
                let stderr_bytes = stderr_handle.join().unwrap_or_default();
                status.map(|s| std::process::Output {
                    status: s,
                    stdout: stdout_bytes,
                    stderr: stderr_bytes,
                })
            }
            Err(e) => Err(e),
        }
    };

    Ok(out?)
}

/// Discover public keys to offer for ssh-copy-id: ~/.ssh/*.pub plus
/// .pub siblings of key paths referenced by credentials.
pub fn find_public_keys(creds: &[Credential]) -> Vec<String> {
    find_public_keys_in(creds, dirs::home_dir().as_deref())
}

pub fn find_public_keys_in(creds: &[Credential], home: Option<&std::path::Path>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    if let Some(home) = home {
        if let Ok(rd) = std::fs::read_dir(home.join(".ssh")) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "pub") {
                    keys.push(p.to_string_lossy().into_owned());
                }
            }
        }
    }
    for c in creds {
        if let Some(ref kp) = c.key_path {
            let kp = config::expand_tilde(kp);
            let pubp = format!("{kp}.pub");
            if std::path::Path::new(&pubp).exists() && !keys.contains(&pubp) {
                keys.push(pubp);
            }
        }
    }
    keys.sort();
    keys
}

/// Run ssh-copy-id for one public key, non-interactively.
/// With a password it goes through the askpass helper; without one,
/// BatchMode relies on agent/default keys so nothing can hang on a prompt.
pub fn copy_id(
    ip: &str,
    port: u16,
    user: &str,
    pub_key_path: &str,
    password: Option<&str>,
    cfg: &AppConfig,
) -> Result<std::process::Output> {
    copy_id_with_jump(ip, port, user, pub_key_path, password, None, cfg)
}

/// Run ssh-copy-id with optional jump host and bounded execution timeout.
pub fn copy_id_with_jump(
    ip: &str,
    port: u16,
    user: &str,
    pub_key_path: &str,
    password: Option<&str>,
    jump: Option<&str>,
    cfg: &AppConfig,
) -> Result<std::process::Output> {
    copy_id_internal(
        ip,
        port,
        user,
        pub_key_path,
        password,
        jump.map(JumpInput::Raw),
        cfg,
    )
}

/// Run ssh-copy-id with optional jump host using separate jump host credentials.
pub fn copy_id_with_jump_auth(
    ip: &str,
    port: u16,
    user: &str,
    pub_key_path: &str,
    password: Option<&str>,
    jump: Option<&JumpAuth>,
    cfg: &AppConfig,
) -> Result<std::process::Output> {
    copy_id_internal(
        ip,
        port,
        user,
        pub_key_path,
        password,
        jump.map(JumpInput::Auth),
        cfg,
    )
}

fn copy_id_internal(
    ip: &str,
    port: u16,
    user: &str,
    pub_key_path: &str,
    password: Option<&str>,
    jump: Option<JumpInput<'_>>,
    cfg: &AppConfig,
) -> Result<std::process::Output> {
    validate_host(ip)?;
    validate_port(port)?;
    validate_username(user)?;
    let strict_host_checking = config::validate_strict_host_checking(&cfg.strict_host_checking)?;
    let key_path = config::expand_tilde(pub_key_path.trim());
    anyhow::ensure!(!key_path.is_empty(), "Public key path cannot be empty");
    anyhow::ensure!(
        !key_path.chars().any(|c| c.is_ascii_control()),
        "Public key path contains invalid control characters"
    );

    let mut args: Vec<String> = vec![
        "-i".into(),
        key_path,
        "-p".into(),
        port.to_string(),
        "-o".into(),
        format!("StrictHostKeyChecking={strict_host_checking}"),
        "-o".into(),
        format!("ConnectTimeout={}", cfg.connect_timeout),
    ];

    let mut cmd = Command::new("ssh-copy-id");
    let _askpass = configure_jump_and_askpass(
        &mut cmd,
        &mut args,
        &CredentialKind::Password,
        password,
        jump,
        strict_host_checking,
        cfg.connect_timeout as u64,
        false,
        true,
    )?;

    if password.is_some() {
        for option in [
            "PreferredAuthentications=password,keyboard-interactive",
            "PubkeyAuthentication=no",
            "PasswordAuthentication=yes",
            "KbdInteractiveAuthentication=yes",
            "BatchMode=no",
            "NumberOfPasswordPrompts=1",
        ] {
            args.extend(["-o".into(), option.into()]);
        }
    } else {
        args.extend_from_slice(&["-o".into(), "BatchMode=yes".into()]);
    }
    args.push("--".into());
    args.push(format!("{user}@{ip}"));

    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    const MAX_COPY_ID_OUTPUT: u64 = 256 * 1024;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(r) = stdout_pipe {
            let _ = std::io::Read::read_to_end(
                &mut std::io::Read::take(r, MAX_COPY_ID_OUTPUT),
                &mut buf,
            );
        }
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(r) = stderr_pipe {
            let _ = std::io::Read::read_to_end(
                &mut std::io::Read::take(r, MAX_COPY_ID_OUTPUT),
                &mut buf,
            );
        }
        buf
    });

    let timeout_secs = (cfg.connect_timeout.max(10) as u64).saturating_add(30);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("ssh-copy-id timed out after {timeout_secs}s"),
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => break Err(e),
        }
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(status.map(|s| std::process::Output {
        status: s,
        stdout,
        stderr,
    })?)
}

pub(crate) fn append_auth_args(
    args: &mut Vec<String>,
    cred: &Credential,
    has_secret: bool,
    interactive: bool,
) {
    let options = match cred.kind {
        CredentialKind::Password => [
            "PreferredAuthentications=password,keyboard-interactive",
            "PubkeyAuthentication=no",
            "PasswordAuthentication=yes",
            "KbdInteractiveAuthentication=yes",
        ],
        CredentialKind::Key => [
            "PreferredAuthentications=publickey",
            "PubkeyAuthentication=yes",
            "PasswordAuthentication=no",
            "KbdInteractiveAuthentication=no",
        ],
    };
    for option in options {
        args.extend(["-o".into(), option.into()]);
    }
    if cred.kind == CredentialKind::Key {
        if let Some(key) = cred
            .key_path
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            args.extend([
                "-i".into(),
                config::expand_tilde(key),
                "-o".into(),
                "IdentitiesOnly=yes".into(),
            ]);
        }
    }
    args.extend([
        "-o".into(),
        if has_secret || interactive {
            "BatchMode=no"
        } else {
            "BatchMode=yes"
        }
        .into(),
    ]);
}

const ASKPASS_SCRIPT: &[u8] = b"#!/bin/sh\ncase \"$HSS_SECRET_KIND:$1\" in\n  key:*[Pp]assphrase*) ;;\n  password:*[Pp]assphrase*) exit 1 ;;\n  password:*[Pp]assword*) ;;\n  *) exit 1 ;;\nesac\ncat -- \"$HSS_SECRET_FILE\"\nprintf '\\n'\n";

fn create_private_askpass_dir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("hss-askpass-{}", uuid::Uuid::new_v4()));
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(&dir)?;
    Ok(dir)
}

fn write_private_file(path: &std::path::Path, content: &[u8], mode: u32) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

/// Configure target askpass and optional jump host forwarding (with isolated per-hop credentials).
///
/// When the jump host has its own resolved credential, this generates a private `0o700`
/// `ProxyCommand` wrapper script so the jump hop uses its own username, key/password auth flags,
/// and isolated `SSH_ASKPASS` secret file without colliding with the target host's authentication.
#[allow(clippy::too_many_arguments)]
pub(crate) fn configure_jump_and_askpass(
    cmd: &mut Command,
    args: &mut Vec<String>,
    target_kind: &CredentialKind,
    target_secret: Option<&str>,
    jump: Option<JumpInput<'_>>,
    strict_host_checking: &str,
    connect_timeout: u64,
    interactive: bool,
    proxy_jump_as_option: bool,
) -> Result<Option<AskpassGuard>> {
    let jump_secret = match jump {
        Some(JumpInput::Auth(ja)) => match ja.cred.as_ref() {
            Some(jcred) => credentials::get_secret(jcred)?,
            None => None,
        },
        _ => None,
    };
    configure_jump_and_askpass_with_secret(
        cmd,
        args,
        target_kind,
        target_secret,
        jump,
        jump_secret.as_deref(),
        strict_host_checking,
        connect_timeout,
        interactive,
        proxy_jump_as_option,
    )
}

#[allow(clippy::too_many_arguments)]
fn configure_jump_and_askpass_with_secret(
    cmd: &mut Command,
    args: &mut Vec<String>,
    target_kind: &CredentialKind,
    target_secret: Option<&str>,
    jump: Option<JumpInput<'_>>,
    jump_secret: Option<&str>,
    strict_host_checking: &str,
    connect_timeout: u64,
    interactive: bool,
    proxy_jump_as_option: bool,
) -> Result<Option<AskpassGuard>> {
    match jump {
        None => configure_askpass(cmd, target_kind, target_secret),
        Some(JumpInput::Raw(j)) => {
            validate_jump_spec(j)?;
            if proxy_jump_as_option {
                args.extend(["-o".into(), format!("ProxyJump={j}")]);
            } else {
                args.extend(["-J".into(), j.to_string()]);
            }
            configure_askpass(cmd, target_kind, target_secret)
        }
        Some(JumpInput::Auth(ja)) if ja.cred.is_none() => {
            let spec = ja.spec();
            validate_jump_spec(&spec)?;
            if proxy_jump_as_option {
                args.extend(["-o".into(), format!("ProxyJump={spec}")]);
            } else {
                args.extend(["-J".into(), spec]);
            }
            configure_askpass(cmd, target_kind, target_secret)
        }
        Some(JumpInput::Auth(ja)) => {
            let jcred = ja.cred.as_ref().unwrap();
            validate_host(&ja.host)?;
            validate_port(ja.port)?;
            if let Some(ref u) = ja.user {
                validate_username(u)?;
            }
            let dir = create_private_askpass_dir()?;
            let mut guard = AskpassGuard {
                dir: dir.clone(),
                script_path: std::path::PathBuf::new(),
                secret_paths: Vec::new(),
                extra_files: Vec::new(),
            };
            let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());

            if let Some(secret) = target_secret {
                let secret_path = dir.join("secret");
                let script_path = dir.join("helper");
                guard.secret_paths.push(secret_path.clone());
                guard.script_path = script_path.clone();
                write_private_file(&secret_path, secret.as_bytes(), 0o600)?;
                write_private_file(&script_path, ASKPASS_SCRIPT, 0o700)?;

                cmd.env("SSH_ASKPASS", &script_path)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env("DISPLAY", &display)
                    .env("HSS_SECRET_FILE", &secret_path)
                    .env(
                        "HSS_SECRET_KIND",
                        match target_kind {
                            CredentialKind::Password => "password",
                            CredentialKind::Key => "key",
                        },
                    );
            }

            let mut proxy_script_body = String::from("#!/bin/sh\n");
            if let Some(jsecret) = jump_secret {
                let jump_secret_path = dir.join("jump_secret");
                let jump_askpass_path = dir.join("jump_askpass");
                guard.secret_paths.push(jump_secret_path.clone());
                guard.extra_files.push(jump_askpass_path.clone());
                write_private_file(&jump_secret_path, jsecret.as_bytes(), 0o600)?;
                write_private_file(&jump_askpass_path, ASKPASS_SCRIPT, 0o700)?;

                let jump_kind = match jcred.kind {
                    CredentialKind::Password => "password",
                    CredentialKind::Key => "key",
                };
                proxy_script_body.push_str(&format!(
                    "export SSH_ASKPASS={}\n",
                    shell_single_quote(&jump_askpass_path.to_string_lossy())
                ));
                proxy_script_body.push_str("export SSH_ASKPASS_REQUIRE=force\n");
                proxy_script_body.push_str(&format!(
                    "export DISPLAY={}\n",
                    shell_single_quote(&display)
                ));
                proxy_script_body.push_str(&format!(
                    "export HSS_SECRET_FILE={}\n",
                    shell_single_quote(&jump_secret_path.to_string_lossy())
                ));
                proxy_script_body.push_str(&format!("export HSS_SECRET_KIND={jump_kind}\n"));
            } else {
                // Prevent outer target askpass env vars from leaking into the jump hop ssh process.
                proxy_script_body.push_str(
                    "unset SSH_ASKPASS SSH_ASKPASS_REQUIRE HSS_SECRET_FILE HSS_SECRET_KIND\n",
                );
            }

            let mut jump_args: Vec<String> = vec![
                "-o".into(),
                format!("StrictHostKeyChecking={strict_host_checking}"),
                "-o".into(),
                format!("ConnectTimeout={}", connect_timeout.max(1)),
                "-p".into(),
                ja.port.to_string(),
            ];
            if jump_secret.is_some() || !interactive {
                jump_args.extend(["-o".into(), "NumberOfPasswordPrompts=1".into()]);
            }
            if let Some(ref u) = ja.user {
                jump_args.extend(["-l".into(), u.clone()]);
            }
            append_auth_args(&mut jump_args, jcred, jump_secret.is_some(), interactive);

            proxy_script_body.push_str("host=\"$1\"\n");
            proxy_script_body.push_str("case \"$host\" in\n");
            proxy_script_body.push_str("  \\[?*\\]) target=\"${host}:$2\" ;;\n");
            proxy_script_body.push_str("  *) target=\"[${host}]:$2\" ;;\n");
            proxy_script_body.push_str("esac\n");
            proxy_script_body.push_str("exec ssh");
            for arg in &jump_args {
                proxy_script_body.push(' ');
                proxy_script_body.push_str(&shell_single_quote(arg));
            }
            proxy_script_body.push_str(" -W \"$target\" -- ");
            proxy_script_body.push_str(&shell_single_quote(&ja.host));
            proxy_script_body.push('\n');

            let proxy_script_path = dir.join("jump_proxy");
            guard.extra_files.push(proxy_script_path.clone());
            write_private_file(&proxy_script_path, proxy_script_body.as_bytes(), 0o700)?;

            args.extend([
                "-o".into(),
                format!(
                    "ProxyCommand={} '%h' '%p'",
                    shell_single_quote(&proxy_script_path.to_string_lossy())
                ),
            ]);

            Ok(Some(guard))
        }
    }
}

pub(crate) fn configure_askpass(
    cmd: &mut Command,
    kind: &CredentialKind,
    secret: Option<&str>,
) -> Result<Option<AskpassGuard>> {
    let Some(secret) = secret else {
        return Ok(None);
    };
    let dir = create_private_askpass_dir()?;
    let script_path = dir.join("helper");
    let secret_path = dir.join("secret");
    let guard = AskpassGuard {
        dir,
        script_path: script_path.clone(),
        secret_paths: vec![secret_path.clone()],
        extra_files: Vec::new(),
    };

    write_private_file(&secret_path, secret.as_bytes(), 0o600)?;
    write_private_file(&script_path, ASKPASS_SCRIPT, 0o700)?;

    cmd.env("SSH_ASKPASS", &script_path)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env(
            "DISPLAY",
            std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into()),
        )
        .env("HSS_SECRET_FILE", &secret_path)
        .env(
            "HSS_SECRET_KIND",
            match kind {
                CredentialKind::Password => "password",
                CredentialKind::Key => "key",
            },
        );
    Ok(Some(guard))
}

pub fn write_askpass_helper() -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("hss-askpass-{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&path)?;
    if let Err(err) = file.write_all(ASKPASS_SCRIPT) {
        let _ = std::fs::remove_file(&path);
        return Err(err.into());
    }
    Ok(path)
}

pub fn connect_direct(host: &str) -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;
    let hosts = config::load_hosts()?;

    let records = config::migrate_server_records(records, &hosts);
    let saved_host = hosts.iter().find(|h| h.name == host || h.ip == host);
    let default_port = if cfg.default_port != 0 {
        cfg.default_port
    } else {
        22
    };
    let (ssh_host, port, host_user, record, jump, forwards, socks_port) =
        if let Some(h) = saved_host {
            (
                h.ip.as_str(),
                h.port,
                h.user.as_deref(),
                records.iter().find(|r| r.host_id == h.id),
                resolve_jump_auth(&hosts, h, &creds, &records)?,
                h.port_forwards.as_slice(),
                h.socks_port,
            )
        } else {
            (host, default_port, None, None, None, &[][..], None)
        };

    let cred = resolve_credential(
        &creds,
        &cfg,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )?
    .ok_or_else(|| {
        anyhow::anyhow!("No credentials configured. Run `hss` to set up credentials.")
    })?;

    spawn_ssh_with_jump_auth(
        ssh_host,
        port,
        host_user,
        cred,
        &cfg,
        jump.as_ref(),
        forwards,
        socks_port,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Credential, CredentialKind};

    fn key_cred(id: &str) -> Credential {
        Credential {
            id: id.into(),
            name: "t".into(),
            username: "u".into(),
            kind: CredentialKind::Key,
            key_path: None,
        }
    }

    fn host(id: &str, jump: Option<&str>, user: Option<&str>, port: u16) -> Host {
        Host {
            id: id.into(),
            name: id.into(),
            ip: format!("ip-{id}"),
            group: String::new(),
            port,
            user: user.map(Into::into),
            tags: vec![],
            description: None,
            jump_host_id: jump.map(Into::into),
            jump_credential_id: None,
            port_forwards: vec![],
            socks_port: None,
            http_proxy_port: None,
        }
    }

    #[test]
    fn find_public_keys_scans_ssh_dir_and_cred_siblings() {
        let tmp = std::env::temp_dir().join(format!("hss-test-keys-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(tmp.join(".ssh")).unwrap();
        std::fs::write(tmp.join(".ssh/id_ed25519.pub"), "k").unwrap();
        std::fs::write(tmp.join(".ssh/id_ed25519"), "k").unwrap();
        std::fs::write(tmp.join("work_key"), "k").unwrap();
        std::fs::write(tmp.join("work_key.pub"), "k").unwrap();

        let cred = Credential {
            id: "c".into(),
            name: "c".into(),
            username: "u".into(),
            kind: CredentialKind::Key,
            key_path: Some(tmp.join("work_key").to_string_lossy().into_owned()),
        };
        let keys = find_public_keys_in(&[cred], Some(&tmp));
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(keys.len(), 2);
        assert!(keys.iter().any(|k| k.ends_with(".ssh/id_ed25519.pub")));
        assert!(keys.iter().any(|k| k.ends_with("work_key.pub")));
    }

    #[test]
    fn jump_spec_full() {
        let hosts = vec![
            host("bastion", None, Some("admin"), 2222),
            host("target", Some("bastion"), None, 22),
        ];
        assert_eq!(
            jump_spec(&hosts, &hosts[1]).unwrap(),
            "admin@ip-bastion:2222"
        );
    }

    #[test]
    fn jump_spec_no_user_default_port() {
        let hosts = vec![
            host("bastion", None, None, 22),
            host("target", Some("bastion"), None, 22),
        ];
        assert_eq!(jump_spec(&hosts, &hosts[1]).unwrap(), "ip-bastion");
    }

    #[test]
    fn jump_spec_none_dangling_and_self() {
        let hosts = vec![
            host("a", Some("missing"), None, 22),
            host("b", Some("b"), None, 22),
        ];
        assert!(jump_spec(&hosts, &hosts[0]).is_none());
        assert!(jump_spec(&hosts, &hosts[1]).is_none());
        assert!(resolve_jump_spec(&hosts, &hosts[0]).is_err());
        assert!(resolve_jump_spec(&hosts, &hosts[1]).is_err());
    }

    #[test]
    fn resolve_last_id_wins() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let result = resolve_credential(&creds, &AppConfig::default(), Some("b"), None).unwrap();
        assert_eq!(result.unwrap().id, "b");
    }

    #[test]
    fn resolve_history_overrides_global_default() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig {
            default_credential_id: Some("a".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_credential(&creds, &cfg, Some("b"), None)
                .unwrap()
                .unwrap()
                .id,
            "b"
        );
    }

    #[test]
    fn resolve_server_default_overrides_history_and_global() {
        let creds = vec![key_cred("a"), key_cred("b"), key_cred("c")];
        let cfg = AppConfig {
            default_credential_id: Some("a".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_credential(&creds, &cfg, Some("b"), Some("c"))
                .unwrap()
                .unwrap()
                .id,
            "c"
        );
        assert_eq!(
            resolve_credential(&creds, &cfg, Some("b"), Some("missing"))
                .unwrap()
                .unwrap()
                .id,
            "b"
        );
    }

    #[test]
    fn resolve_missing_default_uses_history() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig {
            default_credential_id: Some("missing".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_credential(&creds, &cfg, Some("b"), None)
                .unwrap()
                .unwrap()
                .id,
            "b"
        );
    }

    #[test]
    fn auth_args_isolate_credential_kind_and_allow_saved_passphrase() {
        let mut cred = key_cred("key");
        cred.key_path = Some("/test/key".into());
        let mut args = Vec::new();
        append_auth_args(&mut args, &cred, true, false);
        for expected in [
            "PreferredAuthentications=publickey",
            "PasswordAuthentication=no",
            "KbdInteractiveAuthentication=no",
            "BatchMode=no",
            "IdentitiesOnly=yes",
            "/test/key",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "missing {expected}");
        }
        args.clear();
        append_auth_args(&mut args, &cred, false, false);
        assert!(args.iter().any(|arg| arg == "BatchMode=yes"));
        args.clear();
        append_auth_args(&mut args, &cred, false, true);
        assert!(args.iter().any(|arg| arg == "BatchMode=no"));
        cred.kind = CredentialKind::Password;
        args.clear();
        append_auth_args(&mut args, &cred, true, false);
        for expected in [
            "PreferredAuthentications=password,keyboard-interactive",
            "PubkeyAuthentication=no",
            "PasswordAuthentication=yes",
            "KbdInteractiveAuthentication=yes",
        ] {
            assert!(args.iter().any(|arg| arg == expected), "missing {expected}");
        }
        assert!(!args.iter().any(|arg| arg == "-i" || arg == "/test/key"));
    }

    #[test]
    fn askpass_is_private_literal_and_prompt_scoped() {
        use std::os::unix::fs::PermissionsExt;
        let secret = "-n back\\slash 'quoted'\nsecond line";
        for (kind, prompt, allowed) in [
            (
                CredentialKind::Key,
                "Enter passphrase for key '/test/key':",
                true,
            ),
            (CredentialKind::Key, "u@host's password:", false),
            (CredentialKind::Password, "u@host's password:", true),
            (CredentialKind::Password, "Password: ", true),
            (CredentialKind::Password, "Enter passphrase for key:", false),
            (CredentialKind::Password, "Verification code:", false),
            (
                CredentialKind::Key,
                "Are you sure you want to continue connecting?",
                false,
            ),
        ] {
            let mut cmd = Command::new("sh");
            let guard = configure_askpass(&mut cmd, &kind, Some(secret))
                .unwrap()
                .unwrap();
            let path = guard.script_path().to_path_buf();
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert!(!std::fs::read_to_string(&path).unwrap().contains(secret));
            // Secret must never be placed into child process environment variables.
            assert!(!cmd.get_envs().any(|(k, _)| k == "HSS_SECRET"));
            let output = cmd.arg(&path).arg(prompt).output().unwrap();
            drop(guard);
            assert!(!path.exists());
            assert_eq!(output.status.success(), allowed);
            assert_eq!(
                output.stdout,
                if allowed {
                    format!("{secret}\n").into_bytes()
                } else {
                    Vec::new()
                }
            );
        }
        let mut cmd = Command::new("ssh");
        assert!(configure_askpass(&mut cmd, &CredentialKind::Key, None)
            .unwrap()
            .is_none());
        assert_eq!(cmd.get_envs().count(), 0);
    }

    #[test]
    fn effective_username_respects_host_user_for_keys_only() {
        let key = key_cred("k");
        assert_eq!(effective_username(Some("deploy"), &key), "deploy");
        assert_eq!(effective_username(None, &key), "u");
        assert_eq!(effective_username(Some("   "), &key), "u");

        let mut pw = key_cred("p");
        pw.kind = CredentialKind::Password;
        assert_eq!(effective_username(Some("deploy"), &pw), "u");
    }

    #[test]
    fn resolve_default_fallback() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig {
            default_credential_id: Some("a".into()),
            ..Default::default()
        };
        let result = resolve_credential(&creds, &cfg, None, None).unwrap();
        assert_eq!(result.unwrap().id, "a");
    }

    #[test]
    fn resolve_single_cred_auto() {
        let creds = vec![key_cred("only")];
        let result = resolve_credential(&creds, &AppConfig::default(), None, None).unwrap();
        assert_eq!(result.unwrap().id, "only");
    }

    #[test]
    fn resolve_none_when_ambiguous() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let result = resolve_credential(&creds, &AppConfig::default(), None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn resolve_unknown_last_id_falls_to_default() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig {
            default_credential_id: Some("b".into()),
            ..Default::default()
        };
        let result = resolve_credential(&creds, &cfg, Some("nonexistent"), None).unwrap();
        assert_eq!(result.unwrap().id, "b");
    }

    #[test]
    fn validate_host_valid() {
        assert!(validate_host("localhost").is_ok());
        assert!(validate_host("example.com").is_ok());
        assert!(validate_host("sub.domain.example.com").is_ok());
        assert!(validate_host("192.168.1.1").is_ok());
        assert!(validate_host("127.0.0.1").is_ok());
        assert!(validate_host("::1").is_ok());
        assert!(validate_host("fe80::1").is_ok());
        assert!(validate_host("[::1]").is_ok());
        assert!(validate_host("server_1").is_ok());
    }

    #[test]
    fn validate_host_option_injection_rejected() {
        assert!(validate_host("-oProxyCommand=calc").is_err());
        assert!(validate_host("-F/tmp/evil").is_err());
        assert!(validate_host("-p2222").is_err());
        assert!(validate_host("--help").is_err());
        assert!(validate_host("-Jevil").is_err());
    }

    #[test]
    fn validate_host_invalid_chars() {
        assert!(validate_host("").is_err());
        assert!(validate_host("   ").is_err());
        assert!(validate_host("host name").is_err());
        assert!(validate_host("host\nname").is_err());
        assert!(validate_host("host;rm -rf /").is_err());
        assert!(validate_host("host$IFS").is_err());
    }

    #[test]
    fn validate_username_rules() {
        assert!(validate_username("root").is_ok());
        assert!(validate_username("deploy").is_ok());
        assert!(validate_username("user-1").is_ok());
        assert!(validate_username("user_name").is_ok());

        assert!(validate_username("").is_err());
        assert!(validate_username("-oProxyCommand=calc").is_err());
        assert!(validate_username("--help").is_err());
        assert!(validate_username("user@host").is_err());
        assert!(validate_username("user:pass").is_err());
        assert!(validate_username("user name").is_err());
    }

    #[test]
    fn port_forward_args_bind_locally_and_reject_bad_ports() {
        let args = port_forward_args(&["8080:80".into(), "9000:9001".into()], Some(1080)).unwrap();
        assert_eq!(
            args,
            [
                "-L",
                "127.0.0.1:8080:127.0.0.1:80",
                "-L",
                "127.0.0.1:9000:127.0.0.1:9001",
                "-D",
                "127.0.0.1:1080",
                "-o",
                "ExitOnForwardFailure=yes",
            ]
        );
        assert!(port_forward_args(&["8080:80".into()], Some(8080)).is_err());
        assert!(port_forward_args(&["0:80".into()], None).is_err());
        assert!(port_forward_args(&["8080:0".into()], None).is_err());
        assert!(port_forward_args(&["8080:80:90".into()], None).is_err());
        assert!(port_forward_args(&[], Some(0)).is_err());
        assert!(port_forward_args(&[], None).unwrap().is_empty());
    }

    #[test]
    fn validate_port_rules() {
        assert!(validate_port(22).is_ok());
        assert!(validate_port(2222).is_ok());
        assert!(validate_port(65535).is_ok());
        assert!(validate_port(0).is_err());
    }

    #[test]
    fn validate_jump_spec_rules() {
        assert!(validate_jump_spec("bastion").is_ok());
        assert!(validate_jump_spec("admin@bastion:2222").is_ok());
        assert!(validate_jump_spec("admin@10.0.0.1:22").is_ok());
        assert!(validate_jump_spec("bastion1,bastion2").is_ok());

        assert!(validate_jump_spec("-oProxyCommand=calc").is_err());
        assert!(validate_jump_spec("-Jevil").is_err());
        assert!(validate_jump_spec("admin@-oProxyCommand=calc").is_err());
        assert!(validate_jump_spec("").is_err());
    }

    #[test]
    fn classify_ssh_error_patterns() {
        assert_eq!(
            classify_ssh_error("debug1: ...\nPermission denied (publickey,password)."),
            SshErrorKind::AuthFailed
        );
        assert_eq!(
            classify_ssh_error("Host key verification failed."),
            SshErrorKind::HostKeyMismatch
        );
        assert_eq!(
            classify_ssh_error("ssh: connect to host example.com port 22: Connection timed out"),
            SshErrorKind::ConnectionTimeout
        );
        assert_eq!(
            classify_ssh_error("ssh: connect to host 127.0.0.1 port 22: Connection refused"),
            SshErrorKind::ConnectionRefused
        );
        assert_eq!(
            classify_ssh_error(
                "ssh: Could not resolve hostname unknown.local: Name or service not known"
            ),
            SshErrorKind::DnsError
        );
        assert_eq!(
            classify_ssh_error("ssh: connect to host 10.255.255.1: No route to host"),
            SshErrorKind::NoRoute
        );
        assert!(matches!(
            classify_ssh_error("some bizarre custom ssh proxy error"),
            SshErrorKind::Unknown(_)
        ));
        assert_eq!(
            classify_ssh_error("bind [127.0.0.1]:1080: Address already in use\nchannel_setup_fwd_listener_tcpip: cannot listen to port: 1080\nCould not request local forwarding.\nConnection to example.org closed."),
            SshErrorKind::Unknown("Could not request local forwarding.".into())
        );
    }

    #[test]
    fn resolve_jump_auth_precedence_and_username() {
        let mut bastion = host("bastion", None, Some("jumpuser"), 2222);
        bastion.ip = "198.51.100.1".into();
        let mut target = host("target", Some("bastion"), Some("appuser"), 22);
        target.jump_credential_id = Some("explicit".into());

        let hosts = vec![bastion.clone(), target.clone()];
        let creds = vec![
            Credential {
                id: "explicit".into(),
                name: "explicit".into(),
                username: "pw_jump".into(),
                kind: CredentialKind::Password,
                key_path: None,
            },
            Credential {
                id: "bastion-pinned".into(),
                name: "bastion-pinned".into(),
                username: "pinned_user".into(),
                kind: CredentialKind::Key,
                key_path: Some("/keys/bastion".into()),
            },
            Credential {
                id: "bastion-last".into(),
                name: "bastion-last".into(),
                username: "last_user".into(),
                kind: CredentialKind::Password,
                key_path: None,
            },
        ];
        let records = vec![ServerRecord {
            host_id: "bastion".into(),
            last_credential_id: Some("bastion-last".into()),
            default_credential_id: Some("bastion-pinned".into()),
        }];

        // 1. Explicit target.jump_credential_id wins; password cred forces cred.username
        let ja = resolve_jump_auth(&hosts, &target, &creds, &records)
            .unwrap()
            .unwrap();
        assert_eq!(ja.cred.as_ref().unwrap().id, "explicit");
        assert_eq!(ja.user.as_deref(), Some("pw_jump"));
        assert_eq!(ja.spec(), "pw_jump@198.51.100.1:2222");

        // 2. Without explicit jump_credential_id, jump host's pinned default wins;
        //    key cred respects jump host's user override
        target.jump_credential_id = None;
        let ja = resolve_jump_auth(&hosts, &target, &creds, &records)
            .unwrap()
            .unwrap();
        assert_eq!(ja.cred.as_ref().unwrap().id, "bastion-pinned");
        assert_eq!(ja.user.as_deref(), Some("jumpuser"));

        // 3. Without pinned default, jump host's last_credential_id wins
        let records_last = vec![ServerRecord {
            host_id: "bastion".into(),
            last_credential_id: Some("bastion-last".into()),
            default_credential_id: None,
        }];
        let ja = resolve_jump_auth(&hosts, &target, &creds, &records_last)
            .unwrap()
            .unwrap();
        assert_eq!(ja.cred.as_ref().unwrap().id, "bastion-last");
        assert_eq!(ja.user.as_deref(), Some("last_user"));

        // 4. Without any jump cred or jump record, cred is None (standard -J fallback)
        let ja = resolve_jump_auth(&hosts, &target, &creds, &[])
            .unwrap()
            .unwrap();
        assert!(ja.cred.is_none());
        assert_eq!(ja.user.as_deref(), Some("jumpuser"));
    }

    #[test]
    fn configure_jump_and_askpass_isolates_jump_key_from_target_password() {
        let jump_cred = Credential {
            id: "jump-key-no-secret".into(),
            name: "bastion key".into(),
            username: "ops".into(),
            kind: CredentialKind::Key,
            key_path: Some("/tmp/my key/bastion_id".into()),
        };
        let ja = JumpAuth {
            host: "198.51.100.1".into(),
            port: 2222,
            user: Some("ops".into()),
            cred: Some(jump_cred),
        };
        let mut cmd = Command::new("ssh");
        let mut args = Vec::new();
        let guard = configure_jump_and_askpass_with_secret(
            &mut cmd,
            &mut args,
            &CredentialKind::Password,
            Some("target-secret"),
            Some(JumpInput::Auth(&ja)),
            None,
            "accept-new",
            10,
            false,
            false,
        )
        .unwrap()
        .unwrap();

        assert!(args.iter().any(|a| a.starts_with("ProxyCommand=")));
        assert!(!args.iter().any(|a| a == "-J"));
        assert!(cmd.get_envs().any(|(k, _)| k == "SSH_ASKPASS"));

        let proxy_path = guard.extra_files[0].clone();
        let script = std::fs::read_to_string(&proxy_path).unwrap();
        assert!(script.contains("unset SSH_ASKPASS SSH_ASKPASS_REQUIRE"));
        assert!(script.contains("'/tmp/my key/bastion_id'"));
        assert!(script.contains("'IdentitiesOnly=yes'"));
        assert!(script.contains("'-p' '2222'"));
        assert!(script.contains("'-l' 'ops'"));
        assert!(script.contains("'198.51.100.1'"));

        let dir = guard.dir.clone();
        drop(guard);
        assert!(!proxy_path.exists());
        assert!(!dir.exists());
    }

    #[test]
    fn configure_jump_and_askpass_separates_jump_and_target_passwords() {
        let jump_cred = Credential {
            id: "jump-pw".into(),
            name: "bastion pw".into(),
            username: "jump_admin".into(),
            kind: CredentialKind::Password,
            key_path: None,
        };
        let ja = JumpAuth {
            host: "198.51.100.1".into(),
            port: 22,
            user: Some("jump_admin".into()),
            cred: Some(jump_cred),
        };
        let mut cmd = Command::new("ssh");
        let mut args = Vec::new();
        let guard = configure_jump_and_askpass_with_secret(
            &mut cmd,
            &mut args,
            &CredentialKind::Password,
            Some("target-pw-123"),
            Some(JumpInput::Auth(&ja)),
            Some("jump-pw-456"),
            "accept-new",
            10,
            false,
            false,
        )
        .unwrap()
        .unwrap();

        assert_eq!(guard.secret_paths.len(), 2);
        assert_eq!(
            std::fs::read_to_string(&guard.secret_paths[0]).unwrap(),
            "target-pw-123"
        );
        assert_eq!(
            std::fs::read_to_string(&guard.secret_paths[1]).unwrap(),
            "jump-pw-456"
        );
        let proxy_path = guard.extra_files[1].clone();
        let script = std::fs::read_to_string(&proxy_path).unwrap();
        assert!(script.contains("export SSH_ASKPASS="));
        assert!(script.contains("export HSS_SECRET_FILE="));
        assert!(script.contains("export HSS_SECRET_KIND=password"));
        assert!(!script.contains("jump-pw-456"));
        assert!(!script.contains("target-pw-123"));

        let dir = guard.dir.clone();
        drop(guard);
        assert!(!dir.exists());
    }

    #[test]
    fn shell_single_quote_escapes_embedded_quotes() {
        assert_eq!(shell_single_quote("/tmp/foo"), "'/tmp/foo'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }
}
