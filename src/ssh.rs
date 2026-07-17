use anyhow::Result;
use std::os::unix::fs::PermissionsExt;
use libc;
use std::process::Command;
use crate::config::{self, AppConfig};
use crate::credentials;
use crate::types::{Credential, CredentialKind, Host};

/// Build the ssh -J destination ("user@ip:port") for a host's jump host, if any.
/// Dangling ids and self-references resolve to None.
pub fn jump_spec(hosts: &[Host], host: &Host) -> Option<String> {
    let id = host.jump_host_id.as_deref()?;
    if id == host.id { return None; }
    let jump = hosts.iter().find(|h| h.id == id)?;
    let mut spec = String::new();
    if let Some(ref user) = jump.user {
        spec.push_str(user);
        spec.push('@');
    }
    spec.push_str(&jump.ip);
    if jump.port != 22 {
        spec.push_str(&format!(":{}", jump.port));
    }
    Some(spec)
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

pub fn spawn_ssh(host: &str, port: u16, cred: &Credential, cfg: &AppConfig, jump: Option<&str>) -> Result<std::process::ExitStatus> {
    // If keychain entry is missing (e.g. session keyring cleared on logout),
    // fall back to SSH's own interactive password prompt rather than crashing.
    let password = if cred.kind == CredentialKind::Password {
        credentials::get_password(&cred.id).ok()
    } else {
        None
    };

    let mut ssh_args: Vec<String> = vec![
        "-o".into(), format!("StrictHostKeyChecking={}", cfg.strict_host_checking),
        "-o".into(), format!("ConnectTimeout={}", cfg.connect_timeout),
        "-p".into(), port.to_string(),
        "-l".into(), cred.username.clone(),
    ];
    if let Some(ref key) = cred.key_path {
        ssh_args.extend_from_slice(&["-i".into(), key.clone()]);
    }
    if let Some(j) = jump {
        ssh_args.extend_from_slice(&["-J".into(), j.to_string()]);
    }
    if !cfg.ssh_extra_args.is_empty() {
        for arg in cfg.ssh_extra_args.split_whitespace() {
            ssh_args.push(arg.to_string());
        }
    }
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
    unsafe { libc::signal(libc::SIGINT, old_sigint); }

    if let Some(ref path) = askpass_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(result?)
}

/// Run a single command on a host non-interactively, capturing output.
/// Host is looked up by name or IP; credentials resolve the same way as interactive connect.
pub fn exec_command(host_query: &str, command: &str) -> Result<std::process::Output> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;
    let hosts = config::load_hosts()?;

    let h = hosts.iter().find(|h| h.name == host_query || h.ip == host_query)
        .ok_or_else(|| anyhow::anyhow!("Host not found: {host_query}"))?;
    let last_id = records.iter()
        .find(|r| r.host_id == h.name)
        .and_then(|r| r.last_credential_id.clone());
    let cred = resolve_credential(&creds, &cfg, last_id.as_deref())?
        .ok_or_else(|| anyhow::anyhow!("No credential resolved for host '{}'", h.name))?;

    let password = if cred.kind == CredentialKind::Password {
        credentials::get_password(&cred.id).ok()
    } else {
        None
    };

    let mut ssh_args: Vec<String> = vec![
        "-o".into(), format!("StrictHostKeyChecking={}", cfg.strict_host_checking),
        "-o".into(), format!("ConnectTimeout={}", cfg.connect_timeout),
        "-p".into(), h.port.to_string(),
        "-l".into(), cred.username.clone(),
    ];
    if let Some(ref key) = cred.key_path {
        ssh_args.extend_from_slice(&["-i".into(), key.clone()]);
    }
    if password.is_none() {
        // Key auth: never hang on an interactive prompt
        ssh_args.extend_from_slice(&["-o".into(), "BatchMode=yes".into()]);
    }
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

    let out = cmd.output();
    if let Some(ref path) = askpass_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(out?)
}

fn write_askpass_helper() -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("hss-askpass-{}", std::process::id()));
    std::fs::write(&path, "#!/bin/sh\necho \"$HSS_PASSWORD\"\n")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

pub fn connect_direct(host: &str) -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;
    let hosts = config::load_hosts()?;

    let (ssh_host, port, last_cred_id, jump) = if let Some(h) = hosts.iter().find(|h| h.name == host || h.ip == host) {
        let last_id = records.iter()
            .find(|r| r.host_id == h.name)
            .and_then(|r| r.last_credential_id.clone());
        (h.ip.clone(), h.port, last_id, jump_spec(&hosts, h))
    } else {
        (host.to_string(), 22, None, None)
    };

    let cred = resolve_credential(&creds, &cfg, last_cred_id.as_deref())?
        .ok_or_else(|| anyhow::anyhow!("No credentials configured. Run `hss` to set up credentials."))?;

    spawn_ssh(&ssh_host, port, cred, &cfg, jump.as_deref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Credential, CredentialKind};

    fn key_cred(id: &str) -> Credential {
        Credential { id: id.into(), name: "t".into(), username: "u".into(), kind: CredentialKind::Key, key_path: None }
    }

    fn host(id: &str, jump: Option<&str>, user: Option<&str>, port: u16) -> Host {
        Host {
            id: id.into(), name: id.into(), ip: format!("ip-{id}"), group: String::new(),
            port, user: user.map(Into::into), tags: vec![], description: None,
            jump_host_id: jump.map(Into::into),
        }
    }

    #[test]
    fn jump_spec_full() {
        let hosts = vec![host("bastion", None, Some("admin"), 2222), host("target", Some("bastion"), None, 22)];
        assert_eq!(jump_spec(&hosts, &hosts[1]).unwrap(), "admin@ip-bastion:2222");
    }

    #[test]
    fn jump_spec_no_user_default_port() {
        let hosts = vec![host("bastion", None, None, 22), host("target", Some("bastion"), None, 22)];
        assert_eq!(jump_spec(&hosts, &hosts[1]).unwrap(), "ip-bastion");
    }

    #[test]
    fn jump_spec_none_dangling_and_self() {
        let hosts = vec![host("a", Some("missing"), None, 22), host("b", Some("b"), None, 22)];
        assert!(jump_spec(&hosts, &hosts[0]).is_none());
        assert!(jump_spec(&hosts, &hosts[1]).is_none());
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
        let cfg = AppConfig { default_credential_id: Some("a".into()), ..Default::default() };
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
        let cfg = AppConfig { default_credential_id: Some("b".into()), ..Default::default() };
        let result = resolve_credential(&creds, &cfg, Some("nonexistent")).unwrap();
        assert_eq!(result.unwrap().id, "b");
    }
}
