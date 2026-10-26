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

fn entry_path(service: &str, user: &str) -> PathBuf {
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
    if legacy.exists() {
        legacy
    } else {
        dir.join(safe)
    }
}

#[derive(Debug)]
struct FileCredential {
    path: PathBuf,
}

impl CredentialApi for FileCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        use base64::Engine;
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
        match std::fs::read_to_string(&self.path) {
            Ok(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| Error::PlatformFailure(Box::new(e))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::NoEntry),
            Err(e) => Err(Error::PlatformFailure(Box::new(e))),
        }
    }

    fn delete_credential(&self) -> keyring::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::NoEntry),
            Err(e) => Err(Error::PlatformFailure(Box::new(e))),
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
        let path = entry_path(service, user);
        Ok(Box::new(FileCredential { path }))
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

pub fn init_keystore_fallback() {
    let force_file = std::env::var("HSS_KEYSTORE")
        .map(|v| v.eq_ignore_ascii_case("file"))
        .unwrap_or(false);
    let force_system = std::env::var("HSS_KEYSTORE")
        .map(|v| v.eq_ignore_ascii_case("system"))
        .unwrap_or(false);

    let dir = keystore_dir();
    let no_session_bus =
        cfg!(target_os = "linux") && std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none();
    if force_file || (!force_system && (dir.exists() || no_session_bus || !probe_system_keyring()))
    {
        keyring::set_default_credential_builder(default_credential_builder());
        USING_FILE_KEYSTORE.store(true, std::sync::atomic::Ordering::Relaxed);
        if std::io::IsTerminal::is_terminal(&std::io::stderr())
            && !std::env::args().any(|a| a == "--mcp-stdio" || a == "--ansible-inventory")
        {
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
    fn probe_system_keyring_does_not_panic() {
        let _ = probe_system_keyring();
    }
}
