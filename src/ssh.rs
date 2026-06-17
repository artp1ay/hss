use anyhow::Result;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use crate::config::{self, AppConfig};
use crate::credentials;
use crate::types::{Credential, CredentialKind};

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

pub fn spawn_ssh(host: &str, port: u16, cred: &Credential, cfg: &AppConfig) -> Result<std::process::ExitStatus> {
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

    let result = cmd.spawn().and_then(|mut child| child.wait());
    if let Some(ref path) = askpass_path {
        let _ = std::fs::remove_file(path);
    }
    Ok(result?)
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

    let (ssh_host, port, last_cred_id) = if let Some(h) = hosts.iter().find(|h| h.name == host || h.ip == host) {
        let last_id = records.iter()
            .find(|r| r.host_id == h.name)
            .and_then(|r| r.last_credential_id.clone());
        (h.ip.clone(), h.port, last_id)
    } else {
        (host.to_string(), 22, None)
    };

    let cred = resolve_credential(&creds, &cfg, last_cred_id.as_deref())?
        .ok_or_else(|| anyhow::anyhow!("No credentials configured. Run `hss` to set up credentials."))?;

    spawn_ssh(&ssh_host, port, cred, &cfg)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Credential, CredentialKind};

    fn key_cred(id: &str) -> Credential {
        Credential { id: id.into(), name: "t".into(), username: "u".into(), kind: CredentialKind::Key, key_path: None }
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
