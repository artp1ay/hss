use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    pub ip: String,
    pub group: String,
    pub port: u16,
    pub ansible_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CredentialKind {
    Password,
    Key,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub username: String,
    pub kind: CredentialKind,
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRecord {
    pub name: String,
    pub last_credential_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CredentialForm {
    pub editing_id: Option<String>, // None = new, Some(id) = editing
    pub is_key: bool,
    pub name: String,
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub focused: usize, // 0=type toggle, 1=name, 2=username, 3=secret
}
