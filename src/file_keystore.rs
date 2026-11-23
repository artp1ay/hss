use keyring::credential::{
    Credential, CredentialApi, CredentialBuilder, CredentialBuilderApi, CredentialPersistence,
};
use keyring::Error;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

static USING_FILE_KEYSTORE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn is_file_keystore() -> bool {
    USING_FILE_KEYSTORE.load(std::sync::atomic::Ordering::Relaxed)
}

fn keystore_dir() -> PathBuf {
    crate::config::config_dir().join("keystore")
}

fn entry_paths(service: &str, user: &str) -> (PathBuf, PathBuf) {
    let name = format!("{service}:{user}");
    let safe: String = name.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    let dir = keystore_dir();
    let legacy = dir.join(
        format!("{service}_{user}")
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>(),
    );
    (dir.join(safe), legacy)
}

#[cfg(test)]
fn entry_path(service: &str, user: &str) -> PathBuf {
    let (safe, legacy) = entry_paths(service, user);
    if std::fs::symlink_metadata(&legacy)
        .map(|m| m.is_file())
        .unwrap_or(false)
    {
        legacy
    } else {
        safe
    }
}

fn reject_symlink(path: &std::path::Path) -> keyring::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(Error::PlatformFailure(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Refusing to access symlinked keystore entry: {}",
                    path.display()
                ),
            ))))
        }
        _ => Ok(()),
    }
}

fn overwrite_and_remove(path: &std::path::Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Refusing to remove symlinked keystore entry",
        ));
    }
    if meta.is_file() {
        let len = meta.len() as usize;
        if len > 0 {
            if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
                use std::io::Write;
                let zeros = vec![0u8; len.min(64 * 1024)];
                let _ = f.write_all(&zeros);
                let _ = f.sync_all();
            }
        }
    }
    std::fs::remove_file(path)
}

#[derive(Debug)]
struct FileCredential {
    path: PathBuf,
    alt_path: Option<PathBuf>,
}

impl CredentialApi for FileCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        use base64::Engine;
        reject_symlink(&self.path)?;
        if let Some(dir) = self.path.parent() {
            crate::config::ensure_config_dir(dir).map_err(|e| {
                Error::PlatformFailure(Box::new(std::io::Error::other(e.to_string())))
            })?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::PlatformFailure(Box::new(e)))?;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
        crate::config::write_secure_atomic_file(&self.path, &encoded)
            .map_err(|e| Error::PlatformFailure(Box::new(std::io::Error::other(e.to_string()))))
    }

    fn get_secret(&self) -> keyring::Result<Vec<u8>> {
        use base64::Engine;
        let meta = match std::fs::symlink_metadata(&self.path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Error::NoEntry),
            Err(e) => return Err(Error::PlatformFailure(Box::new(e))),
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(Error::PlatformFailure(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Keystore entry is not a regular file",
            ))));
        }
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(Error::PlatformFailure(Box::new(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "Keystore entry {} has overly permissive mode {:04o}; expected 0600",
                    self.path.display(),
                    mode
                ),
            ))));
        }
        match std::fs::read_to_string(&self.path) {
            Ok(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| Error::PlatformFailure(Box::new(e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::NoEntry),
            Err(e) => Err(Error::PlatformFailure(Box::new(e))),
        }
    }

    fn delete_credential(&self) -> keyring::Result<()> {
        let mut deleted_any = false;
        match overwrite_and_remove(&self.path) {
            Ok(()) => deleted_any = true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::PlatformFailure(Box::new(e))),
        }
        if let Some(ref alt) = self.alt_path {
            if alt != &self.path {
                match overwrite_and_remove(alt) {
                    Ok(()) => deleted_any = true,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(Error::PlatformFailure(Box::new(e))),
                }
            }
        }
        if deleted_any {
            Ok(())
        } else {
            Err(Error::NoEntry)
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn debug_fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self, f)
    }
}

struct FileCredentialBuilder;

impl CredentialBuilderApi for FileCredentialBuilder {
    fn build(
        &self,
        _target: Option<&str>,
        service: &str,
        user: &str,
    ) -> keyring::Result<Box<Credential>> {
        let (safe, legacy) = entry_paths(service, user);
        let (path, alt_path) = if std::fs::symlink_metadata(&legacy)
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            (legacy, Some(safe))
        } else {
            (safe, Some(legacy))
        };
        Ok(Box::new(FileCredential { path, alt_path }))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn persistence(&self) -> CredentialPersistence {
        CredentialPersistence::UntilDelete
    }
}

pub fn default_credential_builder() -> Box<CredentialBuilder> {
    Box::new(FileCredentialBuilder)
}

pub fn probe_system_keyring() -> bool {
    let probe = keyring::Entry::new("hss_probe", "probe_test");
    match probe {
        Ok(entry) => match entry.get_password() {
            Err(keyring::Error::NoEntry) => true,
            Ok(_) => true,
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn keystore_has_entries(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .map(|mut rd| {
            rd.any(|entry| {
                entry
                    .ok()
                    .and_then(|e| e.file_type().ok())
                    .is_some_and(|ft| ft.is_file())
            })
        })
        .unwrap_or(false)
}

pub fn init_keystore_fallback() {
    let raw_mode = std::env::var("HSS_KEYSTORE").ok();
    let force_file = raw_mode
        .as_deref()
        .map(|v| v.trim().eq_ignore_ascii_case("file"))
        .unwrap_or(false);
    let force_system = raw_mode
        .as_deref()
        .map(|v| v.trim().eq_ignore_ascii_case("system"))
        .unwrap_or(false);

    let quiet_cli = std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--mcp-stdio" | "--ansible-inventory" | "--list" | "--host"
        )
    });

    if let Some(ref mode) = raw_mode {
        let trimmed = mode.trim();
        if !trimmed.is_empty() && !force_file && !force_system && !quiet_cli {
            eprintln!(
                "hss: unrecognized HSS_KEYSTORE='{trimmed}' (expected 'file' or 'system'); using auto-detection"
            );
        }
    }

    let dir = keystore_dir();
    let no_session_bus =
        cfg!(target_os = "linux") && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none();
    if force_file
        || (!force_system
            && (keystore_has_entries(&dir) || no_session_bus || !probe_system_keyring()))
    {
        keyring::set_default_credential_builder(default_credential_builder());
        USING_FILE_KEYSTORE.store(true, std::sync::atomic::Ordering::Relaxed);
        if std::io::IsTerminal::is_terminal(&std::io::stderr()) && !quiet_cli {
            eprintln!(
                "hss: using file-based keystore ({}); secrets are NOT encrypted, protect this directory and your backups",
                dir.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_credential(name: &str) -> FileCredential {
        let dir = std::env::temp_dir().join(format!("hss_test_keystore_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        FileCredential {
            path: dir.join(name),
            alt_path: None,
        }
    }

    #[test]
    fn roundtrip_secret() {
        let cred = tmp_credential("roundtrip");
        let _ = cred.delete_credential();

        assert!(matches!(cred.get_secret(), Err(Error::NoEntry)));

        cred.set_secret(b"hello\x00world").unwrap();
        assert_eq!(cred.get_secret().unwrap(), b"hello\x00world");

        cred.set_password("updated").unwrap();
        assert_eq!(cred.get_password().unwrap(), "updated");

        cred.delete_credential().unwrap();
        assert!(matches!(cred.get_secret(), Err(Error::NoEntry)));
    }

    #[test]
    fn delete_missing_is_no_entry() {
        let cred = tmp_credential("no_such_entry");
        let _ = cred.delete_credential();
        assert!(matches!(cred.delete_credential(), Err(Error::NoEntry)));
    }

    #[test]
    fn builder_creates_credential() {
        let builder = FileCredentialBuilder;
        let cred = builder.build(None, "svc", "usr").unwrap();
        assert!(cred.as_any().downcast_ref::<FileCredential>().is_some());
        assert!(matches!(
            builder.persistence(),
            CredentialPersistence::UntilDelete
        ));
    }

    #[test]
    fn entry_path_sanitizes() {
        let p = entry_path("my/service", "user@host");
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(!name.contains('/'));
        assert!(!name.contains('@'));
        assert_ne!(entry_path("hss", "a/b"), entry_path("hss", "a_b"));
        assert_ne!(entry_path("ab", "c"), entry_path("a", "bc"));
    }

    #[test]
    fn file_secret_persists_with_restricted_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let cred = tmp_credential("persist_permissions");
        cred.set_password("key passphrase").unwrap();
        let reopened = FileCredential {
            path: cred.path.clone(),
            alt_path: None,
        };
        assert_eq!(reopened.get_password().unwrap(), "key passphrase");
        assert_eq!(
            fs::metadata(&cred.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(cred.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        reopened.delete_credential().unwrap();
    }

    #[test]
    fn rejects_loose_permissions_and_deletes_alt_path() {
        let dir = std::env::temp_dir().join(format!(
            "hss_test_keystore_hardened_{}",
            uuid::Uuid::new_v4()
        ));
        crate::config::ensure_config_dir(&dir).unwrap();
        let primary = dir.join("primary");
        let alt = dir.join("alt");
        let cred = FileCredential {
            path: primary.clone(),
            alt_path: Some(alt.clone()),
        };
        cred.set_password("secret-1").unwrap();
        crate::config::write_secure_atomic_file(&alt, "c2VjcmV0LTI=").unwrap();

        fs::set_permissions(&primary, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(cred.get_password().is_err());

        fs::set_permissions(&primary, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(cred.get_password().unwrap(), "secret-1");

        cred.delete_credential().unwrap();
        assert!(!primary.exists());
        assert!(!alt.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_keystore_dir_does_not_count_as_having_entries() {
        let dir = std::env::temp_dir().join(format!("hss_test_empty_ks_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        assert!(!keystore_has_entries(&dir));
        fs::write(dir.join("entry"), "abc").unwrap();
        assert!(keystore_has_entries(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn probe_system_keyring_does_not_panic() {
        let _ = probe_system_keyring();
    }
}
