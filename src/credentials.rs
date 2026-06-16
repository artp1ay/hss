use anyhow::Result;
use uuid::Uuid;
use crate::config;
use crate::types::{Credential, CredentialKind};

const KEYCHAIN_SERVICE: &str = "hss";

pub fn add_credential(
    name: &str,
    username: &str,
    kind: CredentialKind,
    key_path: Option<String>,
    password: Option<&str>,
) -> Result<Credential> {
    let id = Uuid::new_v4().to_string();
    if kind == CredentialKind::Password {
        let pw = password.ok_or_else(|| anyhow::anyhow!("password required for Password credential"))?;
        keyring::Entry::new(KEYCHAIN_SERVICE, &id)?.set_password(pw)?;
    }
    let cred = Credential { id, name: name.to_string(), username: username.to_string(), kind, key_path };
    let mut creds = config::load_credentials()?;
    creds.push(cred.clone());
    config::save_credentials(&creds)?;
    Ok(cred)
}

pub fn update_credential(
    id: &str,
    name: &str,
    username: &str,
    kind: CredentialKind,
    key_path: Option<String>,
    password: Option<&str>,
) -> Result<()> {
    if kind == CredentialKind::Password {
        if let Some(pw) = password {
            if !pw.is_empty() {
                keyring::Entry::new(KEYCHAIN_SERVICE, id)?.set_password(pw)?;
            }
        }
    }
    let mut creds = config::load_credentials()?;
    let cred = creds.iter_mut().find(|c| c.id == id)
        .ok_or_else(|| anyhow::anyhow!("credential not found: {id}"))?;
    cred.name = name.to_string();
    cred.username = username.to_string();
    cred.kind = kind;
    cred.key_path = key_path;
    config::save_credentials(&creds)
}

pub fn delete_credential(id: &str) -> Result<()> {
    let _ = keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.delete_credential());
    let mut creds = config::load_credentials()?;
    creds.retain(|c| c.id != id);
    config::save_credentials(&creds)
}

pub fn get_password(id: &str) -> Result<String> {
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, id)?.get_password()?)
}
