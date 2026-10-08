#[test]
fn test_appconfig_default() {
    let cfg = hss::config::AppConfig::default();
    assert!(cfg.default_credential_id.is_none());
    assert!(cfg.default_user.is_none());
    assert_eq!(cfg.default_port, 22);
    assert_eq!(cfg.connect_timeout, 10);
    assert_eq!(cfg.strict_host_checking, "accept-new");
    assert!(cfg.auto_save_credential);
}

#[test]
fn test_hosts_roundtrip() {
    use hss::types::Host;
    let hosts = vec![Host {
        id: "id-1".into(),
        name: "web1".into(),
        ip: "192.168.1.10".into(),
        group: "webservers".into(),
        port: 2222,
        user: Some("deploy".into()),
        tags: vec!["production".into(), "nginx".into()],
        description: Some("Primary web server".into()),
        jump_host_id: None,
    }];
    let s = hss::config::serialize_hosts(&hosts).unwrap();
    let parsed = hss::config::parse_hosts(&s).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "id-1");
    assert_eq!(parsed[0].port, 2222);
    assert_eq!(parsed[0].tags, vec!["production", "nginx"]);
    assert_eq!(parsed[0].description, Some("Primary web server".into()));
}

#[test]
fn test_credentials_roundtrip() {
    use hss::types::{Credential, CredentialKind};
    let creds = vec![Credential {
        id: "id1".into(),
        name: "test key".into(),
        username: "deploy".into(),
        kind: CredentialKind::Key,
        key_path: Some("/home/user/.ssh/id_rsa".into()),
    }];
    let s = hss::config::serialize_credentials(&creds).unwrap();
    let parsed = hss::config::parse_credentials(&s).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].id, "id1");
    assert_eq!(parsed[0].kind, hss::types::CredentialKind::Key);
}

#[test]
fn test_server_records_roundtrip() {
    use hss::types::ServerRecord;
    let records = vec![
        ServerRecord {
            host_id: "host-uuid-1".into(),
            last_credential_id: Some("cred-1".into()),
        },
        ServerRecord {
            host_id: "host-uuid-2".into(),
            last_credential_id: None,
        },
    ];
    let s = hss::config::serialize_server_records(&records).unwrap();
    let parsed = hss::config::parse_server_records(&s).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].host_id, "host-uuid-1");
    assert_eq!(parsed[0].last_credential_id, Some("cred-1".into()));
    assert_eq!(parsed[1].last_credential_id, None);
}

#[test]
fn test_migrate_server_records_rewrites_name_keyed_entries() {
    use hss::types::{Host, ServerRecord};
    let hosts = vec![Host {
        id: "uuid-1".into(),
        name: "web1".into(),
        ip: "10.0.0.1".into(),
        group: "g".into(),
        port: 22,
        user: None,
        tags: vec![],
        description: None,
        jump_host_id: None,
    }];
    let records = vec![
        // Old-style entry: host_id happens to equal the host's name
        ServerRecord {
            host_id: "web1".into(),
            last_credential_id: Some("cred-a".into()),
        },
        // Already-migrated entry: stays untouched
        ServerRecord {
            host_id: "uuid-2".into(),
            last_credential_id: Some("cred-b".into()),
        },
    ];
    let migrated = hss::config::migrate_server_records(records, &hosts);
    assert_eq!(migrated[0].host_id, "uuid-1");
    assert_eq!(migrated[1].host_id, "uuid-2");
}

#[test]
fn test_write_secure_file_sets_0600_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp_dir = std::env::temp_dir().join(format!("hss-test-perm-{}", uuid::Uuid::new_v4()));
    hss::config::ensure_config_dir(&tmp_dir).unwrap();
    let dir_mode = std::fs::metadata(&tmp_dir).unwrap().permissions().mode() & 0o777;
    assert_eq!(dir_mode, 0o700);

    let test_file = tmp_dir.join("secret.toml");
    hss::config::write_secure_file(&test_file, "secret = 123").unwrap();
    let file_mode = std::fs::metadata(&test_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(file_mode, 0o600);

    let _ = std::fs::remove_dir_all(tmp_dir);
}

#[test]
fn test_write_secure_atomic_file_creates_unique_temp_and_renames() {
    use std::os::unix::fs::PermissionsExt;
    let tmp_dir = std::env::temp_dir().join(format!("hss-test-atomic-{}", uuid::Uuid::new_v4()));
    let target_file = tmp_dir.join("test.toml");

    hss::config::write_secure_atomic_file(&target_file, "key = 'val'").unwrap();
    assert!(target_file.exists());

    let file_mode = std::fs::metadata(&target_file)
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(file_mode, 0o600);

    let _ = std::fs::remove_dir_all(tmp_dir);
}

#[test]
fn test_server_record_deserializes_with_legacy_name_alias() {
    let toml_str = r#"
        [[server]]
        name = "host-foo"
        last_credential_id = "cred-123"
    "#;
    let records = hss::config::parse_server_records(toml_str).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].host_id, "host-foo");
    assert_eq!(records[0].last_credential_id, Some("cred-123".into()));
}
