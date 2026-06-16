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
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_dir().join("config.toml"), toml::to_string_pretty(cfg)?)?;
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
    if !path.exists() {
        return Ok(vec![]);
    }
    parse_credentials(&std::fs::read_to_string(path)?)
}

pub fn save_credentials(creds: &[Credential]) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_dir().join("credentials.toml"), serialize_credentials(creds)?)?;
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
    if !path.exists() {
        return Ok(vec![]);
    }
    parse_server_records(&std::fs::read_to_string(path)?)
}

pub fn save_server_records(records: &[ServerRecord]) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_dir().join("servers.toml"), serialize_server_records(records)?)?;
    Ok(())
}
