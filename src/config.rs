use serde::{Deserialize, Serialize};
use crate::types::{Credential, ServerRecord};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub inventory_path: Option<String>,
    pub default_credential_id: Option<String>,
}

pub fn load_config() -> anyhow::Result<AppConfig> { Ok(AppConfig::default()) }
pub fn save_config(_: &AppConfig) -> anyhow::Result<()> { Ok(()) }
pub fn load_credentials() -> anyhow::Result<Vec<Credential>> { Ok(vec![]) }
pub fn save_credentials(_: &[Credential]) -> anyhow::Result<()> { Ok(()) }
pub fn load_server_records() -> anyhow::Result<Vec<ServerRecord>> { Ok(vec![]) }
pub fn save_server_records(_: &[ServerRecord]) -> anyhow::Result<()> { Ok(()) }
pub fn serialize_credentials(_: &[Credential]) -> anyhow::Result<String> { Ok(String::new()) }
pub fn parse_credentials(_: &str) -> anyhow::Result<Vec<Credential>> { Ok(vec![]) }
pub fn serialize_server_records(_: &[ServerRecord]) -> anyhow::Result<String> { Ok(String::new()) }
pub fn parse_server_records(_: &str) -> anyhow::Result<Vec<ServerRecord>> { Ok(vec![]) }
