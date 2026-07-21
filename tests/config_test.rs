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
        ServerRecord { host_id: "host-uuid-1".into(), last_credential_id: Some("cred-1".into()) },
        ServerRecord { host_id: "host-uuid-2".into(), last_credential_id: None },
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
        id: "uuid-1".into(), name: "web1".into(), ip: "10.0.0.1".into(), group: "g".into(),
        port: 22, user: None, tags: vec![], description: None, jump_host_id: None,
    }];
    let records = vec![
        // Old-style entry: host_id happens to equal the host's name
        ServerRecord { host_id: "web1".into(), last_credential_id: Some("cred-a".into()) },
        // Already-migrated entry: stays untouched
        ServerRecord { host_id: "uuid-2".into(), last_credential_id: Some("cred-b".into()) },
    ];
    let migrated = hss::config::migrate_server_records(records, &hosts);
    assert_eq!(migrated[0].host_id, "uuid-1");
    assert_eq!(migrated[1].host_id, "uuid-2");
}
