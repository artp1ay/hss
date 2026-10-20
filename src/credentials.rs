use crate::config;
use crate::types::{Credential, CredentialKind};
use anyhow::{Context, Result};
use uuid::Uuid;

const KEYCHAIN_SERVICE: &str = "hss";

fn validate_fields(name: &str, username: &str) -> Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "credential name cannot be empty");
    anyhow::ensure!(!username.trim().is_empty(), "username cannot be empty");
    Ok(())
}

fn secret_store_context(operation: &str) -> String {
    let hint = if cfg!(target_os = "macos") {
        "macOS Keychain: unlock the login keychain and allow access for the hss executable"
    } else {
        "Secret Service: run a session D-Bus and an unlocked persistent keyring (e.g. GNOME Keyring) for the user running hss; org.freedesktop.secrets must be available on that bus"
    };
    format!(
        "Cannot {operation} credential in system secret store (hss {}, {}): {hint}. MCP uses the secret store of its own process, not the SSH destination",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    )
}

fn read_secret(entry: &keyring::Entry) -> Result<Option<String>> {
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(err) => Err(err).with_context(|| secret_store_context("read")),
    }
}

fn write_secret(entry: &keyring::Entry, secret: Option<&str>) -> Result<()> {
    match secret {
        Some(secret) => entry
            .set_password(secret)
            .with_context(|| secret_store_context("save")),
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err).with_context(|| secret_store_context("delete")),
        },
    }
}

fn next_secret<'a>(
    kind: &CredentialKind,
    old_kind: Option<&CredentialKind>,
    old_secret: Option<&'a str>,
    supplied: Option<&'a str>,
) -> Result<Option<&'a str>> {
    let secret = supplied.filter(|s| !s.is_empty()).or_else(|| {
        if old_kind == Some(kind) {
            old_secret
        } else {
            None
        }
    });
    anyhow::ensure!(
        *kind != CredentialKind::Password || secret.is_some_and(|s| !s.is_empty()),
        "Password required for Password credential; save it in Credentials"
    );
    Ok(secret)
}

fn save_with_secret(
    entry: &keyring::Entry,
    old: Option<&str>,
    new: Option<&str>,
    save: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if old != new {
        write_secret(entry, new)?;
    }
    if let Err(err) = save() {
        if old != new {
            write_secret(entry, old).context(format!(
                "Metadata save failed ({err}); secret rollback failed"
            ))?;
        }
        return Err(err);
    }
    Ok(())
}

pub fn add_credential(
    name: &str,
    username: &str,
    kind: CredentialKind,
    key_path: Option<String>,
    password: Option<&str>,
) -> Result<Credential> {
    validate_fields(name, username)?;
    let secret = next_secret(&kind, None, None, password)?;
    let mut creds = config::load_credentials()?;
    let cred = Credential {
        id: Uuid::new_v4().to_string(),
        name: name.trim().to_string(),
        username: username.trim().to_string(),
        kind,
        key_path,
    };
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &cred.id)
        .with_context(|| secret_store_context("initialize"))?;
    creds.push(cred.clone());
    save_with_secret(&entry, None, secret, || config::save_credentials(&creds))?;
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
    validate_fields(name, username)?;
    let mut creds = config::load_credentials()?;
    let idx = creds
        .iter()
        .position(|c| c.id == id)
        .ok_or_else(|| anyhow::anyhow!("credential not found: {id}"))?;
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, id)
        .with_context(|| secret_store_context("initialize"))?;
    let old = read_secret(&entry)?;
    let new = next_secret(&kind, Some(&creds[idx].kind), old.as_deref(), password)?;
    creds[idx] = Credential {
        id: id.to_string(),
        name: name.trim().to_string(),
        username: username.trim().to_string(),
        kind,
        key_path,
    };
    save_with_secret(&entry, old.as_deref(), new, || {
        config::save_credentials(&creds)
    })
}

pub fn delete_credential(id: &str) -> Result<()> {
    let mut creds = config::load_credentials()?;
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, id)
        .with_context(|| secret_store_context("initialize"))?;
    let old = read_secret(&entry)?;
    creds.retain(|c| c.id != id);
    save_with_secret(&entry, old.as_deref(), None, || {
        config::save_credentials(&creds)
    })
}

pub fn get_secret(cred: &Credential) -> Result<Option<String>> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, &cred.id)
        .with_context(|| secret_store_context("initialize"))?;
    let secret = read_secret(&entry)?;
    next_secret(&cred.kind, Some(&cred.kind), secret.as_deref(), None)
        .with_context(|| format!("Credential '{}' has no saved password", cred.name))?;
    Ok(secret)
}

pub fn get_password(id: &str) -> Result<String> {
    read_secret(
        &keyring::Entry::new(KEYCHAIN_SERVICE, id)
            .with_context(|| secret_store_context("initialize"))?,
    )?
    .ok_or_else(|| anyhow::anyhow!("No saved password; save it in Credentials"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring::mock::MockCredential;

    fn entry() -> keyring::Entry {
        keyring::Entry::new_with_credential(Box::new(MockCredential::default()))
    }

    #[test]
    fn empty_name_or_username_rejected() {
        assert!(add_credential("", "user", CredentialKind::Key, None, None).is_err());
        assert!(add_credential("name", "", CredentialKind::Key, None, None).is_err());
        assert!(add_credential("   ", "user", CredentialKind::Key, None, None).is_err());
    }

    #[test]
    fn password_kind_requires_nonempty_password() {
        for secret in [None, Some("")] {
            assert!(
                add_credential("name", "user", CredentialKind::Password, None, secret).is_err()
            );
        }
    }

    #[test]
    fn editing_preserves_secret_only_for_same_kind() {
        use CredentialKind::{Key, Password};
        for kind in [Key, Password] {
            assert_eq!(
                next_secret(&kind, Some(&kind), Some("old"), Some("")).unwrap(),
                Some("old")
            );
            assert_eq!(
                next_secret(&kind, Some(&kind), Some("old"), Some("new")).unwrap(),
                Some("new")
            );
        }
        assert_eq!(
            next_secret(&Key, Some(&Password), Some("password"), None).unwrap(),
            None
        );
        assert!(next_secret(&Password, Some(&Key), Some("passphrase"), None).is_err());
        assert_eq!(
            next_secret(&Key, None, None, Some("passphrase")).unwrap(),
            Some("passphrase")
        );
    }

    #[test]
    fn secret_roundtrip_and_delete() {
        let entry = entry();
        assert_eq!(read_secret(&entry).unwrap(), None);
        write_secret(&entry, Some("secret\\with\ncharacters")).unwrap();
        assert_eq!(
            read_secret(&entry).unwrap().as_deref(),
            Some("secret\\with\ncharacters")
        );
        write_secret(&entry, None).unwrap();
        assert_eq!(read_secret(&entry).unwrap(), None);
        write_secret(&entry, None).unwrap();
    }

    #[test]
    fn metadata_failure_restores_secret() {
        for old in [None, Some("old")] {
            for new in [None, Some("new")] {
                let entry = entry();
                write_secret(&entry, old).unwrap();
                assert!(save_with_secret(&entry, old, new, || anyhow::bail!("disk full")).is_err());
                assert_eq!(read_secret(&entry).unwrap().as_deref(), old);
            }
        }
    }

    #[test]
    fn backend_errors_are_not_missing_secrets() {
        let entry = entry();
        let mock = entry
            .get_credential()
            .downcast_ref::<MockCredential>()
            .unwrap();
        mock.set_error(keyring::Error::Invalid("backend".into(), "locked".into()));
        let error = read_secret(&entry).unwrap_err();
        assert!(error.to_string().contains(std::env::consts::OS));
        assert!(error.to_string().contains("MCP uses the secret store"));
        assert!(format!("{error:#}").contains("locked"));
        mock.set_error(keyring::Error::Invalid("backend".into(), "locked".into()));
        assert!(save_with_secret(&entry, None, Some("new"), || panic!(
            "must not save metadata"
        ))
        .is_err());
    }
}
