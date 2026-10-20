use keyring::credential::{
    Credential, CredentialApi, CredentialBuilder, CredentialBuilderApi, CredentialPersistence,
};
use keyring::Error;
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
    let name = format!("{service}_{user}");
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    keystore_dir().join(safe)
}

#[derive(Debug)]
struct FileCredential {
    path: PathBuf,
}

impl CredentialApi for FileCredential {
    fn set_secret(&self, secret: &[u8]) -> keyring::Result<()> {
        use base64::Engine;
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

    if force_file || (!force_system && !probe_system_keyring()) {
        keyring::set_default_credential_builder(default_credential_builder());
        USING_FILE_KEYSTORE.store(true, std::sync::atomic::Ordering::Relaxed);
        if std::io::IsTerminal::is_terminal(&std::io::stderr())
            && !std::env::args().any(|a| a == "--mcp-stdio" || a == "--ansible-inventory")
        {
            eprintln!(
                "hss: system keyring unavailable, using file-based keystore (~/.config/hss/keystore/)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_credential(name: &str) -> FileCredential {
        let dir = std::env::temp_dir().join("hss_test_keystore");
        let _ = fs::create_dir_all(&dir);
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
    }

    #[test]
    fn probe_system_keyring_does_not_panic() {
        let _ = probe_system_keyring();
    }

    #[test]
    fn file_keystore_flag_initially_matches_state() {
        assert_eq!(
            is_file_keystore(),
            USING_FILE_KEYSTORE.load(std::sync::atomic::Ordering::Relaxed)
        );
    }
}
