use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::types::{Credential, Host, ServerRecord};

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()))
        .join("hss")
}

fn default_port() -> u16 { 22 }
fn default_timeout() -> u8 { 10 }
fn default_strict_host_checking() -> String { "accept-new".to_string() }
fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_credential_id: Option<String>,
    pub default_user: Option<String>,
    #[serde(default = "default_port")]
    pub default_port: u16,
    #[serde(default = "default_timeout")]
    pub connect_timeout: u8,
    #[serde(default)]
    pub ssh_extra_args: String,
    #[serde(default = "default_strict_host_checking")]
    pub strict_host_checking: String,
    #[serde(default = "default_true")]
    pub auto_save_credential: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_credential_id: None,
            default_user: None,
            default_port: 22,
            connect_timeout: 10,
            ssh_extra_args: String::new(),
            strict_host_checking: "accept-new".to_string(),
            auto_save_credential: true,
        }
    }
}

// ── AppConfig ────────────────────────────────────────────────────────────────

pub fn load_config() -> Result<AppConfig> {
    let path = config_dir().join("config.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(toml::from_str(&s)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(e) => Err(e.into()),
    }
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("config.toml.tmp");
    std::fs::write(&tmp, toml::to_string_pretty(cfg)?)?;
    std::fs::rename(tmp, dir.join("config.toml"))?;
    Ok(())
}

// ── Hosts ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct HostsFile {
    #[serde(default, rename = "host")]
    hosts: Vec<Host>,
}

pub fn serialize_hosts(hosts: &[Host]) -> Result<String> {
    Ok(toml::to_string_pretty(&HostsFile { hosts: hosts.to_vec() })?)
}

pub fn parse_hosts(s: &str) -> Result<Vec<Host>> {
    Ok(toml::from_str::<HostsFile>(s)?.hosts)
}

pub fn load_hosts() -> Result<Vec<Host>> {
    let path = config_dir().join("hosts.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => parse_hosts(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

pub fn save_hosts(hosts: &[Host]) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("hosts.toml.tmp");
    std::fs::write(&tmp, serialize_hosts(hosts)?)?;
    std::fs::rename(tmp, dir.join("hosts.toml"))?;
    Ok(())
}

// ── Credentials ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default, rename = "credential")]
    credentials: Vec<Credential>,
}

pub fn serialize_credentials(creds: &[Credential]) -> Result<String> {
    Ok(toml::to_string_pretty(&CredentialsFile { credentials: creds.to_vec() })?)
}

pub fn parse_credentials(s: &str) -> Result<Vec<Credential>> {
    Ok(toml::from_str::<CredentialsFile>(s)?.credentials)
}

pub fn load_credentials() -> Result<Vec<Credential>> {
    let path = config_dir().join("credentials.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => parse_credentials(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

pub fn save_credentials(creds: &[Credential]) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("credentials.toml.tmp");
    std::fs::write(&tmp, serialize_credentials(creds)?)?;
    std::fs::rename(tmp, dir.join("credentials.toml"))?;
    Ok(())
}

// ── Server records ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct ServersFile {
    #[serde(default, rename = "server")]
    servers: Vec<ServerRecord>,
}

pub fn serialize_server_records(records: &[ServerRecord]) -> Result<String> {
    Ok(toml::to_string_pretty(&ServersFile { servers: records.to_vec() })?)
}

pub fn parse_server_records(s: &str) -> Result<Vec<ServerRecord>> {
    Ok(toml::from_str::<ServersFile>(s)?.servers)
}

pub fn load_server_records() -> Result<Vec<ServerRecord>> {
    let path = config_dir().join("servers.toml");
    match std::fs::read_to_string(&path) {
        Ok(s) => parse_server_records(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.into()),
    }
}

pub fn save_server_records(records: &[ServerRecord]) -> Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join("servers.toml.tmp");
    std::fs::write(&tmp, serialize_server_records(records)?)?;
    std::fs::rename(tmp, dir.join("servers.toml"))?;
    Ok(())
}

/// Records used to be keyed by host *name*, which broke (silently lost the
/// saved default credential) whenever a host was renamed. Rewrite any
/// name-keyed entry to the host's stable id; already-migrated (id-keyed)
/// entries never match a host name, so this is idempotent.
pub fn migrate_server_records(records: Vec<ServerRecord>, hosts: &[Host]) -> Vec<ServerRecord> {
    records.into_iter().map(|mut r| {
        if let Some(h) = hosts.iter().find(|h| h.name == r.host_id) {
            r.host_id = h.id.clone();
        }
        r
    }).collect()
}
