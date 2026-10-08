use crate::config::{self, AppConfig};
use crate::credentials;
use crate::types::{Credential, CredentialKind, Host};
use anyhow::Result;
use libc;
use std::os::unix::fs::PermissionsExt;
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
        .rfind(|l| !l.trim().is_empty())
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
) -> Result<Option<&'a Credential>> {
    if let Some(id) = last_id {
        if let Some(c) = creds.iter().find(|c| c.id == id) {
            return Ok(Some(c));
        }
    }
    if let Some(ref id) = cfg.default_credential_id {
        if let Some(c) = creds.iter().find(|c| &c.id == id) {
            return Ok(Some(c));
        }
    }
    if creds.len() == 1 {
        return Ok(Some(&creds[0]));
    }
    Ok(None)
}

pub fn spawn_ssh(
    host: &str,
    port: u16,
    cred: &Credential,
    cfg: &AppConfig,
    jump: Option<&str>,
) -> Result<std::process::ExitStatus> {
    validate_host(host)?;
    validate_port(port)?;
    validate_username(&cred.username)?;
    if let Some(j) = jump {
        validate_jump_spec(j)?;
    }

    // If keychain entry is missing (e.g. session keyring cleared on logout),
    // fall back to SSH's own interactive password prompt rather than crashing.
    let password = if cred.kind == CredentialKind::Password {
        credentials::get_password(&cred.id).ok()
    } else {
        None
    };

    let mut ssh_args: Vec<String> = vec![
        "-o".into(),
        format!("StrictHostKeyChecking={}", cfg.strict_host_checking),
        "-o".into(),
        format!("ConnectTimeout={}", cfg.connect_timeout),
        "-p".into(),
        port.to_string(),
        "-l".into(),
        cred.username.clone(),
    ];
    if let Some(ref key) = cred.key_path {
        ssh_args.extend_from_slice(&["-i".into(), key.clone()]);
        ssh_args.extend_from_slice(&["-o".into(), "IdentitiesOnly=yes".into()]);
    }
    if let Some(j) = jump {
        ssh_args.extend_from_slice(&["-J".into(), j.to_string()]);
    }
    if !cfg.ssh_extra_args.is_empty() {
        for arg in parse_extra_args(&cfg.ssh_extra_args) {
            ssh_args.push(arg);
        }
    }
    ssh_args.push("--".into());
    ssh_args.push(host.to_string());

    let mut cmd = Command::new("ssh");
    cmd.args(&ssh_args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let askpass_path = if let Some(ref pw) = password {
        let path = write_askpass_helper()?;
        // DISPLAY is required by some SSH implementations (older macOS, some Linux) even
        // when SSH_ASKPASS_REQUIRE=force is set. Use the existing DISPLAY or a dummy value.
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        cmd.env("SSH_ASKPASS", &path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", display)
            .env("HSS_PASSWORD", pw);
        Some(path)
    } else {
        None
    };

    // Ignore SIGINT in hss while SSH runs: Ctrl+C kills SSH but returns control here.
    // Without this, Ctrl+C sends SIGINT to the whole process group and hss exits too.
    let old_sigint = unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    let result = cmd.spawn().and_then(|mut child| child.wait());
    unsafe {
        libc::signal(libc::SIGINT, old_sigint);
    }

    if let Some(ref path) = askpass_path {
        let _ = std::fs::remove_file(path);
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
    let last_id = records
        .iter()
        .find(|r| r.host_id == h.id)
        .and_then(|r| r.last_credential_id.clone());
    let cred = resolve_credential(&creds, &cfg, last_id.as_deref())?
        .ok_or_else(|| anyhow::anyhow!("No credential resolved for host '{}'", h.name))?;

    validate_host(&h.ip)?;
    validate_port(h.port)?;
    validate_username(&cred.username)?;
    let jump = resolve_jump_spec(&hosts, h)?;
    if let Some(ref j) = jump {
        validate_jump_spec(j)?;
    }

    let password = if cred.kind == CredentialKind::Password {
        credentials::get_password(&cred.id).ok()
    } else {
        None
    };

    let mut ssh_args: Vec<String> = vec![
        "-o".into(),
        format!("StrictHostKeyChecking={}", cfg.strict_host_checking),
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
        cred.username.clone(),
    ];
    if let Some(ref key) = cred.key_path {
        ssh_args.extend_from_slice(&["-i".into(), key.clone()]);
        ssh_args.extend_from_slice(&["-o".into(), "IdentitiesOnly=yes".into()]);
    }
    if let Some(ref j) = jump {
        ssh_args.extend_from_slice(&["-J".into(), j.clone()]);
    }
    if password.is_none() {
        // Key auth: never hang on an interactive prompt
        ssh_args.extend_from_slice(&["-o".into(), "BatchMode=yes".into()]);
    }
    ssh_args.push("--".into());
    ssh_args.push(h.ip.clone());
    ssh_args.push(command.to_string());

    let mut cmd = Command::new("ssh");
    cmd.args(&ssh_args).stdin(std::process::Stdio::null());

    let askpass_path = if let Some(ref pw) = password {
        let path = write_askpass_helper()?;
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        cmd.env("SSH_ASKPASS", &path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", display)
            .env("HSS_PASSWORD", pw);
        Some(path)
    } else {
        None
    };

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

    if let Some(ref path) = askpass_path {
        let _ = std::fs::remove_file(path);
    }
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
            let kp = expand_tilde(kp);
            let pubp = format!("{kp}.pub");
            if std::path::Path::new(&pubp).exists() && !keys.contains(&pubp) {
                keys.push(pubp);
            }
        }
    }
    keys.sort();
    keys
}

fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
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
    validate_host(ip)?;
    validate_port(port)?;
    validate_username(user)?;

    let mut args: Vec<String> = vec![
        "-i".into(),
        pub_key_path.into(),
        "-p".into(),
        port.to_string(),
        "-o".into(),
        format!("StrictHostKeyChecking={}", cfg.strict_host_checking),
        "-o".into(),
        format!("ConnectTimeout={}", cfg.connect_timeout),
    ];
    if password.is_none() {
        args.extend_from_slice(&["-o".into(), "BatchMode=yes".into()]);
    }
    args.push(format!("{user}@{ip}"));

    let mut cmd = Command::new("ssh-copy-id");
    cmd.args(&args).stdin(std::process::Stdio::null());

    let askpass_path = if let Some(pw) = password {
        let path = write_askpass_helper()?;
        let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".into());
        cmd.env("SSH_ASKPASS", &path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("DISPLAY", display)
            .env("HSS_PASSWORD", pw);
        Some(path)
    } else {
        None
    };

    let out = cmd.output();
    if let Some(ref path) = askpass_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(out?)
}

pub fn write_askpass_helper() -> Result<std::path::PathBuf> {
    let rand: u64 = {
        let mut buf = [0u8; 8];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut buf))
            .unwrap_or_else(|_| {
                buf = std::process::id()
                    .to_ne_bytes()
                    .into_iter()
                    .chain([0u8; 4])
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap_or([0u8; 8]);
            });
        u64::from_ne_bytes(buf)
    };
    let path = std::env::temp_dir().join(format!("hss-askpass-{:016x}", rand));
    std::fs::write(&path, "#!/bin/sh\necho \"$HSS_PASSWORD\"\n")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

pub fn connect_direct(host: &str) -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;
    let hosts = config::load_hosts()?;

    let records = config::migrate_server_records(records, &hosts);
    let (ssh_host, port, last_cred_id, jump) =
        if let Some(h) = hosts.iter().find(|h| h.name == host || h.ip == host) {
            let last_id = records
                .iter()
                .find(|r| r.host_id == h.id)
                .and_then(|r| r.last_credential_id.clone());
            let jump = resolve_jump_spec(&hosts, h)?;
            (h.ip.clone(), h.port, last_id, jump)
        } else {
            (host.to_string(), 22, None, None)
        };

    let cred = resolve_credential(&creds, &cfg, last_cred_id.as_deref())?.ok_or_else(|| {
        anyhow::anyhow!("No credentials configured. Run `hss` to set up credentials.")
    })?;

    spawn_ssh(&ssh_host, port, cred, &cfg, jump.as_deref())?;
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
        let result = resolve_credential(&creds, &AppConfig::default(), Some("b")).unwrap();
        assert_eq!(result.unwrap().id, "b");
    }

    #[test]
    fn resolve_default_fallback() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig {
            default_credential_id: Some("a".into()),
            ..Default::default()
        };
        let result = resolve_credential(&creds, &cfg, None).unwrap();
        assert_eq!(result.unwrap().id, "a");
    }

    #[test]
    fn resolve_single_cred_auto() {
        let creds = vec![key_cred("only")];
        let result = resolve_credential(&creds, &AppConfig::default(), None).unwrap();
        assert_eq!(result.unwrap().id, "only");
    }

    #[test]
    fn resolve_none_when_ambiguous() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let result = resolve_credential(&creds, &AppConfig::default(), None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn resolve_unknown_last_id_falls_to_default() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig {
            default_credential_id: Some("b".into()),
            ..Default::default()
        };
        let result = resolve_credential(&creds, &cfg, Some("nonexistent")).unwrap();
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
    }
}
