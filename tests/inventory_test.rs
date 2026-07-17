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
    assert_eq!(web1.user, Some("deploy".into()));

    let web2 = hosts.iter().find(|h| h.name == "web2").unwrap();
    assert_eq!(web2.port, 22);
    assert_eq!(web2.user, None);

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
fn test_import_adds_new_hosts() {
    let ini = "[webservers]\nweb1 ansible_host=10.0.0.1 ansible_user=deploy\n";
    let mut hosts = vec![];
    let count = hss::inventory::import_from_ini(ini, &mut hosts);
    assert_eq!(count, 1);
    assert_eq!(hosts[0].name, "web1");
    assert_eq!(hosts[0].ip, "10.0.0.1");
    assert_eq!(hosts[0].user, Some("deploy".into()));
    assert_eq!(hosts[0].group, "webservers");
}

#[test]
fn test_import_updates_existing_host() {
    let existing = hss::types::Host {
        id: "existing-id".into(),
        name: "web1".into(),
        ip: "old-ip".into(),
        group: "old-group".into(),
        port: 22,
        user: None,
        tags: vec!["custom-tag".into()],
        description: Some("My server".into()),
        jump_host_id: None,
    };
    let mut hosts = vec![existing];
    let ini = "[webservers]\nweb1 ansible_host=10.0.0.1 ansible_user=deploy\n";
    let count = hss::inventory::import_from_ini(ini, &mut hosts);
    assert_eq!(count, 0);
    assert_eq!(hosts[0].id, "existing-id");
    assert_eq!(hosts[0].ip, "10.0.0.1");
    assert_eq!(hosts[0].user, Some("deploy".into()));
    assert_eq!(hosts[0].description, Some("My server".into()));
    assert!(hosts[0].tags.contains(&"custom-tag".to_string()));
}

#[test]
fn test_import_merges_tags() {
    let existing = hss::types::Host {
        id: "id1".into(),
        name: "web1".into(),
        ip: "10.0.0.1".into(),
        group: "web".into(),
        port: 22,
        user: None,
        tags: vec!["existing-tag".into()],
        description: None,
        jump_host_id: None,
    };
    let mut hosts = vec![existing];
    let ini = "[web]\nweb1 ansible_host=10.0.0.1 # tags=new-tag,existing-tag\n";
    hss::inventory::import_from_ini(ini, &mut hosts);
    assert!(hosts[0].tags.contains(&"existing-tag".to_string()));
    assert!(hosts[0].tags.contains(&"new-tag".to_string()));
    assert_eq!(hosts[0].tags.len(), 2);
}

#[test]
fn test_export_to_ini_basic() {
    use hss::types::Host;
    let hosts = vec![
        Host { id: "1".into(), name: "web1".into(), ip: "10.0.0.1".into(), group: "webservers".into(), port: 22, user: Some("deploy".into()), tags: vec![], description: None, jump_host_id: None },
        Host { id: "2".into(), name: "db1".into(), ip: "10.0.0.2".into(), group: "databases".into(), port: 5432, user: None, tags: vec![], description: None, jump_host_id: None },
    ];
    let ini = hss::inventory::export_to_ini(&hosts);
    assert!(ini.contains("[webservers]"));
    assert!(ini.contains("[databases]"));
    assert!(ini.contains("web1 ansible_host=10.0.0.1 ansible_port=22 ansible_user=deploy"));
    assert!(ini.contains("db1 ansible_host=10.0.0.2 ansible_port=5432"));
    assert!(!ini.contains("ansible_user=postgres") && !ini.contains("ansible_user=\n"));
}
