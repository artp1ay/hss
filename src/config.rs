use crate::types::{Credential, Host, ServerRecord};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()))
        .join("hss")
}

/// Ensures the config directory exists with secure 0700 permissions on Unix.
pub fn ensure_config_dir(dir: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    builder.mode(0o700);
    builder.create(dir)?;
    Ok(())
}

/// Atomically writes content to `path` with 0600 permissions using a unique temporary file
/// in the same directory, then renames it.
pub fn write_secure_atomic_file(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid file path without parent: {}", path.display()))?;
    ensure_config_dir(parent)?;

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let unique_tmp_name = format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4());
    let tmp_path = parent.join(unique_tmp_name);

    let res = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&tmp_path)?;
        use std::io::Write;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        Ok(())
    })();

    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e.into());
    }

    Ok(())
}

/// Legacy helper: writes directly to the given path securely.
pub fn write_secure_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_config_dir(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    use std::io::Write;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    Ok(())
}

fn default_port() -> u16 {
    22
}
fn default_group() -> String {
    "ungrouped".to_string()
}
fn default_timeout() -> u16 {
    10
}
fn default_exec_timeout() -> u32 {
    300
}
fn default_mcp_port() -> u16 {
    8822
}
fn default_audit_timeout() -> u16 {
    3
}
fn default_strict_host_checking() -> String {
    "accept-new".to_string()
}
fn default_true() -> bool {
    true
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub default_credential_id: Option<String>,
    pub default_user: Option<String>,
    #[serde(default = "default_port")]
    pub default_port: u16,
    #[serde(default = "default_group")]
    pub default_group: String,
    #[serde(default = "default_timeout")]
    pub connect_timeout: u16,
    /// Max seconds an MCP execute-command may run (0 = no limit).
    #[serde(default = "default_exec_timeout")]
    pub exec_timeout: u32,
    #[serde(default = "default_mcp_port")]
    pub mcp_port: u16,
    /// Optional bearer token for MCP server authentication.
    /// Can also be set via HSS_MCP_TOKEN env var.
    #[serde(default)]
    pub mcp_token: Option<String>,
    /// Allow execute-command even when no mcp_token is configured (unsafe).
    #[serde(default)]
    pub allow_unauthenticated_execute: bool,
    /// Optional file path for logging MCP requests/responses.
    #[serde(default)]
    pub mcp_log_file: Option<String>,
    /// Timeout in seconds for connectivity and auth audit tests.
    #[serde(default = "default_audit_timeout")]
    pub audit_timeout: u16,
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
            schema_version: 1,
            default_credential_id: None,
            default_user: None,
            default_port: 22,
            default_group: "ungrouped".to_string(),
            connect_timeout: 10,
            exec_timeout: 300,
            mcp_port: 8822,
            mcp_token: None,
            allow_unauthenticated_execute: false,
            mcp_log_file: None,
            audit_timeout: 3,
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
    let path = config_dir().join("config.toml");
    write_secure_atomic_file(&path, &toml::to_string_pretty(cfg)?)
}

// ── Hosts ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct HostsFile {
    #[serde(default, rename = "host")]
    hosts: Vec<Host>,
}

pub fn serialize_hosts(hosts: &[Host]) -> Result<String> {
    Ok(toml::to_string_pretty(&HostsFile {
        hosts: hosts.to_vec(),
    })?)
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
    let path = config_dir().join("hosts.toml");
    write_secure_atomic_file(&path, &serialize_hosts(hosts)?)
}

// ── Credentials ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default, rename = "credential")]
    credentials: Vec<Credential>,
}

pub fn serialize_credentials(creds: &[Credential]) -> Result<String> {
    Ok(toml::to_string_pretty(&CredentialsFile {
        credentials: creds.to_vec(),
    })?)
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
    let path = config_dir().join("credentials.toml");
    write_secure_atomic_file(&path, &serialize_credentials(creds)?)
}

// ── Server records ───────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct ServersFile {
    #[serde(default, rename = "server")]
    servers: Vec<ServerRecord>,
}

pub fn serialize_server_records(records: &[ServerRecord]) -> Result<String> {
    Ok(toml::to_string_pretty(&ServersFile {
        servers: records.to_vec(),
    })?)
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
    let path = config_dir().join("servers.toml");
    write_secure_atomic_file(&path, &serialize_server_records(records)?)
}

/// Records used to be keyed by host *name*, which broke (silently lost the
/// saved default credential) whenever a host was renamed. Rewrite any
/// name-keyed entry to the host's stable id; already-migrated (id-keyed)
/// entries never match a host name, so this is idempotent.
/// If any record was modified, saves the migrated records immediately to disk.
pub fn migrate_server_records(records: Vec<ServerRecord>, hosts: &[Host]) -> Vec<ServerRecord> {
    let mut modified = false;
    let migrated: Vec<ServerRecord> = records
        .into_iter()
        .map(|mut r| {
            if let Some(h) = hosts.iter().find(|h| h.name == r.host_id) {
                if r.host_id != h.id {
                    r.host_id = h.id.clone();
                    modified = true;
                }
            }
            r
        })
        .collect();

    if modified {
        let _ = save_server_records(&migrated);
    }
    migrated
}

pub fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return format!("{}{}", home.display(), &path[1..]);
        }
    }
    path.to_string()
}
