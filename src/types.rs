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
    #[serde(default)]
    pub jump_host_id: Option<String>,
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
    #[serde(alias = "name")]
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
    pub cursor: usize,  // char index inside the focused field
}

#[derive(Debug, Clone, Default)]
pub struct CopyIdForm {
    pub host_idx: usize,
    pub keys: Vec<(String, bool)>, // (path, selected)
    pub key_cursor: usize,
    pub user: String,
    pub password: String,
    pub focused: usize, // 0=key list 1=user 2=password
    pub cursor: usize,  // char index inside the focused field
    pub error_message: Option<String>,
    pub in_progress: bool,
    pub progress_status: Option<String>,
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
    pub port: String, // stored as String for editing, parsed on save
    pub user: String,
    pub tags: String, // comma-separated
    pub description: String,
    pub jump_host_id: Option<String>, // selected from existing hosts, cycled with ←/→
    pub focused: usize, // 0=name 1=ip 2=group 3=port 4=user 5=tags 6=description 7=jump host
    pub cursor: usize,  // char index inside the focused field
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusKind {
    Success,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMessage {
    pub text: String,
    pub kind: StatusKind,
    pub created_at: std::time::Instant,
}

impl StatusMessage {
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Success,
            created_at: std::time::Instant::now(),
        }
    }

    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Info,
            created_at: std::time::Instant::now(),
        }
    }

    pub fn warning(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Warning,
            created_at: std::time::Instant::now(),
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Error,
            created_at: std::time::Instant::now(),
        }
    }

    pub fn is_expired(&self) -> bool {
        // Success and info auto-expire after 4 seconds; errors/warnings persist longer (10s)
        let ttl = match self.kind {
            StatusKind::Success | StatusKind::Info => std::time::Duration::from_secs(4),
            StatusKind::Warning | StatusKind::Error => std::time::Duration::from_secs(10),
        };
        self.created_at.elapsed() > ttl
    }
}
