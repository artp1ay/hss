#[test]
fn test_parse_basic_inventory() {
    let ini = r#"
[webservers]
web1 ansible_host=192.168.1.10 ansible_port=2222 ansible_user=deploy
web2 ansible_host=192.168.1.11

[databases]
db1 ansible_host=10.0.0.5
"#;
    let hosts = hss::inventory::parse_inventory(ini);
    assert_eq!(hosts.len(), 3);

    let web1 = hosts.iter().find(|h| h.name == "web1").unwrap();
    assert_eq!(web1.ip, "192.168.1.10");
    assert_eq!(web1.port, 2222);
    assert_eq!(web1.group, "webservers");
    assert_eq!(web1.ansible_user, Some("deploy".into()));

    let web2 = hosts.iter().find(|h| h.name == "web2").unwrap();
    assert_eq!(web2.port, 22);
    assert_eq!(web2.ansible_user, None);

    let db1 = hosts.iter().find(|h| h.name == "db1").unwrap();
    assert_eq!(db1.group, "databases");
}

#[test]
fn test_parse_skips_vars_and_children_sections() {
    let ini = r#"
[webservers]
web1 ansible_host=10.0.0.1

[webservers:vars]
ansible_user=deploy

[all:children]
webservers
"#;
    let hosts = hss::inventory::parse_inventory(ini);
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].name, "web1");
}

#[test]
fn test_parse_host_without_ansible_host_falls_back_to_name() {
    let ini = "[servers]\nmyserver\n";
    let hosts = hss::inventory::parse_inventory(ini);
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].name, "myserver");
    assert_eq!(hosts[0].ip, "myserver");
    assert_eq!(hosts[0].port, 22);
}

#[test]
fn test_parse_ignores_comments_and_blank_lines() {
    let ini = "# comment\n\n[servers]\n; another comment\nhost1 ansible_host=1.2.3.4\n";
    let hosts = hss::inventory::parse_inventory(ini);
    assert_eq!(hosts.len(), 1);
}

#[test]
fn test_sync_removes_deleted_hosts() {
    use hss::types::ServerRecord;
    let records = vec![
        ServerRecord { name: "web1".into(), last_credential_id: Some("c1".into()) },
        ServerRecord { name: "old-gone".into(), last_credential_id: None },
    ];
    let active = vec!["web1".to_string()];
    let synced = hss::inventory::sync_server_records(records, &active);
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].name, "web1");
}

#[test]
fn test_sync_preserves_last_credential() {
    use hss::types::ServerRecord;
    let records = vec![
        ServerRecord { name: "web1".into(), last_credential_id: Some("cred-42".into()) },
    ];
    let active = vec!["web1".to_string()];
    let synced = hss::inventory::sync_server_records(records, &active);
    assert_eq!(synced[0].last_credential_id, Some("cred-42".into()));
}
