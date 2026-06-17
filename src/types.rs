use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub group: String,
    pub port: u16,
    pub user: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub description: Option<String>,
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
    pub host_id: String,
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

#[derive(Debug, Clone, PartialEq)]
pub enum DeleteKind {
    Host,
    Credential,
}

#[derive(Debug, Clone)]
pub struct DeletePopup {
    pub kind: DeleteKind,
    pub name: String,
    pub idx: usize,
    pub dont_ask: bool,
}

#[derive(Debug, Clone, Default)]
pub struct HostForm {
    pub editing_id: Option<String>, // None = new host
    pub name: String,
    pub ip: String,
    pub group: String,
    pub port: String,        // stored as String for editing, parsed on save
    pub user: String,
    pub tags: String,        // comma-separated
    pub description: String,
    pub focused: usize,      // 0=name 1=ip 2=group 3=port 4=user 5=tags 6=description
}
