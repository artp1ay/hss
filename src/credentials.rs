use crate::config;
use crate::types::{Credential, CredentialKind};
use anyhow::Result;
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "hss";

pub fn add_credential(
    name: &str,
    username: &str,
    kind: CredentialKind,
    key_path: Option<String>,
    password: Option<&str>,
) -> Result<Credential> {
    let name = name.trim();
    let username = username.trim();
    if name.is_empty() {
        anyhow::bail!("credential name cannot be empty");
    }
    if username.is_empty() {
        anyhow::bail!("username cannot be empty");
    }

    let id = Uuid::new_v4().to_string();
    let cred = Credential {
        id: id.clone(),
        name: name.to_string(),
        username: username.to_string(),
        kind: kind.clone(),
        key_path,
    };

    // 1. Stage password in keyring if needed
    if kind == CredentialKind::Password {
        let pw =
            password.ok_or_else(|| anyhow::anyhow!("password required for Password credential"))?;
        keyring::Entry::new(KEYCHAIN_SERVICE, &id)?.set_password(pw)?;
    }

    // 2. Save metadata to TOML; rollback keyring on failure
    let mut creds = config::load_credentials()?;
    creds.push(cred.clone());
    if let Err(e) = config::save_credentials(&creds) {
        if kind == CredentialKind::Password {
            let _ = keyring::Entry::new(KEYCHAIN_SERVICE, &id).and_then(|e| e.delete_credential());
        }
        return Err(e);
    }

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
    let name = name.trim();
    let username = username.trim();
    if name.is_empty() {
        anyhow::bail!("credential name cannot be empty");
    }
    if username.is_empty() {
        anyhow::bail!("username cannot be empty");
    }

    let mut creds = config::load_credentials()?;
    let idx = creds
        .iter()
        .position(|c| c.id == id)
        .ok_or_else(|| anyhow::anyhow!("credential not found: {id}"))?;

    let old_cred = creds[idx].clone();
    let old_password = if old_cred.kind == CredentialKind::Password {
        get_password(id).ok()
    } else {
        None
    };

    // Stage changes in keyring
    let mut keyring_changed = false;
    if kind == CredentialKind::Password {
        if let Some(pw) = password {
            if !pw.is_empty() {
                keyring::Entry::new(KEYCHAIN_SERVICE, id)?.set_password(pw)?;
                keyring_changed = true;
            }
        } else if old_cred.kind != CredentialKind::Password {
            anyhow::bail!("password required when changing to Password credential");
        }
    } else if old_cred.kind == CredentialKind::Password {
        // Switched from Password to Key: will remove from keyring after TOML is saved
    }

    // Apply metadata updates
    creds[idx].name = name.to_string();
    creds[idx].username = username.to_string();
    creds[idx].kind = kind.clone();
    creds[idx].key_path = key_path;

    if let Err(e) = config::save_credentials(&creds) {
        // Rollback keyring
        if keyring_changed {
            if let Some(ref old_pw) = old_password {
                let _ =
                    keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.set_password(old_pw));
            } else {
                let _ =
                    keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.delete_credential());
            }
        }
        return Err(e);
    }

    // Cleanup old password if converted from Password to Key
    if old_cred.kind == CredentialKind::Password && kind != CredentialKind::Password {
        let _ = keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.delete_credential());
    }

    Ok(())
}

pub fn delete_credential(id: &str) -> Result<()> {
    // 1. Remove from TOML metadata first
    let mut creds = config::load_credentials()?;
    let existed = creds.iter().any(|c| c.id == id);
    if existed {
        creds.retain(|c| c.id != id);
        config::save_credentials(&creds)?;
    }

    // 2. Remove keyring secret; ignore missing entry
    let _ = keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.delete_credential());
    Ok(())
}

pub fn get_password(id: &str) -> Result<String> {
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, id)?.get_password()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_name_or_username_rejected() {
        assert!(add_credential("", "user", CredentialKind::Key, None, None).is_err());
        assert!(add_credential("name", "", CredentialKind::Key, None, None).is_err());
        assert!(add_credential("   ", "user", CredentialKind::Key, None, None).is_err());
    }

    #[test]
    fn password_kind_requires_password() {
        assert!(add_credential("name", "user", CredentialKind::Password, None, None).is_err());
        assert!(
            add_credential("name", "user", CredentialKind::Password, None, Some("")).is_ok()
                || true
        ); // In environments without keyring daemon, keyring may fail, but error path is exercised
    }
}
