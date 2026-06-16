use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::types::{Credential, ServerRecord};

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()))
        .join("hss")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub inventory_path: Option<String>,
    pub default_credential_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default, rename = "credential")]
    credentials: Vec<Credential>,
}

#[derive(Serialize, Deserialize, Default)]
struct ServersFile {
    #[serde(default, rename = "server")]
    servers: Vec<ServerRecord>,
}

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
    std::fs::write(dir.join("config.toml"), toml::to_string_pretty(cfg)?)?;
    Ok(())
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
    std::fs::rename(&tmp, dir.join("credentials.toml"))?;
    Ok(())
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
    std::fs::rename(&tmp, dir.join("servers.toml"))?;
    Ok(())
}
