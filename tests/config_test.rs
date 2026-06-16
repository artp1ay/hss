#[test]
fn test_appconfig_default_has_no_fields() {
    let cfg = hss::config::AppConfig::default();
    assert!(cfg.inventory_path.is_none());
    assert!(cfg.default_credential_id.is_none());
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
    assert_eq!(parsed[0].key_path, Some("/home/user/.ssh/id_rsa".into()));
}

#[test]
fn test_password_credential_roundtrip() {
    use hss::types::{Credential, CredentialKind};
    let creds = vec![Credential {
        id: "pw1".into(),
        name: "admin".into(),
        username: "admin".into(),
        kind: CredentialKind::Password,
        key_path: None,
    }];
    let s = hss::config::serialize_credentials(&creds).unwrap();
    let parsed = hss::config::parse_credentials(&s).unwrap();
    assert_eq!(parsed[0].kind, hss::types::CredentialKind::Password);
    assert!(parsed[0].key_path.is_none());
}

#[test]
fn test_server_records_roundtrip() {
    use hss::types::ServerRecord;
    let records = vec![
        ServerRecord { name: "web1".into(), last_credential_id: Some("cred-1".into()) },
        ServerRecord { name: "db1".into(), last_credential_id: None },
    ];
    let s = hss::config::serialize_server_records(&records).unwrap();
    let parsed = hss::config::parse_server_records(&s).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "web1");
    assert_eq!(parsed[0].last_credential_id, Some("cred-1".into()));
    assert_eq!(parsed[1].last_credential_id, None);
}
