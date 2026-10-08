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
fn test_import_partial_merge_does_not_overwrite_with_defaults() {
    let existing = hss::types::Host {
        id: "id1".into(),
        name: "web1".into(),
        ip: "10.0.0.1".into(),
        group: "web".into(),
        port: 2222,
        user: Some("original-user".into()),
        tags: vec![],
        description: None,
        jump_host_id: None,
    };
    let mut hosts = vec![existing];

    // INI only specifies user; no host or port
    let ini = "web1 ansible_user=deploy\n";
    let count = hss::inventory::import_from_ini(ini, &mut hosts);
    assert_eq!(count, 0);
    // ip and port must be preserved from existing host!
    assert_eq!(hosts[0].ip, "10.0.0.1");
    assert_eq!(hosts[0].port, 2222);
    assert_eq!(hosts[0].user, Some("deploy".into()));
}

#[test]
fn test_generate_ansible_inventory_handles_reserved_group_names() {
    let hosts = vec![hss::types::Host {
        id: "id1".into(),
        name: "node1".into(),
        ip: "10.0.0.1".into(),
        group: "all".into(),
        port: 22,
        user: None,
        tags: vec![],
        description: None,
        jump_host_id: None,
    }];
    let creds = vec![];
    let records = vec![];
    let cfg = hss::config::AppConfig::default();

    let inv = hss::inventory::generate_ansible_inventory(&hosts, &creds, &records, &cfg);
    // top-level 'all' must still have a children key and not be overridden by group membership
    assert!(inv.get("all").unwrap().get("children").is_some());
    assert!(inv.get("_meta").unwrap().get("hostvars").is_some());
}

#[test]
fn test_export_to_ssh_config() {
    use hss::types::Host;
    let hosts = vec![
        Host {
            id: "b".into(),
            name: "bastion".into(),
            ip: "1.2.3.4".into(),
            group: "infra".into(),
            port: 2222,
            user: Some("ops".into()),
            tags: vec![],
            description: None,
            jump_host_id: None,
        },
        Host {
            id: "w".into(),
            name: "web1".into(),
            ip: "10.0.0.1".into(),
            group: "webservers".into(),
            port: 22,
            user: Some("deploy".into()),
            tags: vec!["web".into(), "prod".into()],
            description: Some("Main server".into()),
            jump_host_id: Some("b".into()),
        },
    ];
    let cfg = hss::inventory::export_to_ssh_config(&hosts);
    assert!(cfg.contains("# group: webservers"));
    assert!(cfg.contains("# tags: web, prod"));
    assert!(cfg.contains("# Main server"));
    assert!(cfg.contains(
        "Host web1\n    HostName 10.0.0.1\n    User deploy\n    ProxyJump ops@1.2.3.4:2222"
    ));
    assert!(cfg.contains("Host bastion\n    HostName 1.2.3.4\n    Port 2222\n    User ops"));
    // default port omitted for web1
    assert!(!cfg.contains("Host web1\n    HostName 10.0.0.1\n    Port"));
}

#[test]
fn test_export_to_ini_basic() {
    use hss::types::Host;
    let hosts = vec![
        Host {
            id: "1".into(),
            name: "web1".into(),
            ip: "10.0.0.1".into(),
            group: "webservers".into(),
            port: 22,
            user: Some("deploy".into()),
            tags: vec![],
            description: None,
            jump_host_id: None,
        },
        Host {
            id: "2".into(),
            name: "db1".into(),
            ip: "10.0.0.2".into(),
            group: "databases".into(),
            port: 5432,
            user: None,
            tags: vec![],
            description: None,
            jump_host_id: None,
        },
    ];
    let ini = hss::inventory::export_to_ini(&hosts);
    assert!(ini.contains("[webservers]"));
    assert!(ini.contains("[databases]"));
    assert!(ini.contains("web1 ansible_host=10.0.0.1 ansible_port=22 ansible_user=deploy"));
    assert!(ini.contains("db1 ansible_host=10.0.0.2 ansible_port=5432"));
    assert!(!ini.contains("ansible_user=postgres") && !ini.contains("ansible_user=\n"));
}

#[test]
fn test_generate_ansible_inventory_structure() {
    use hss::config::AppConfig;
    use hss::types::{Credential, CredentialKind, Host, ServerRecord};

    let hosts = vec![
        Host {
            id: "bastion-id".into(),
            name: "bastion".into(),
            ip: "198.51.100.1".into(),
            group: "gateways".into(),
            port: 22,
            user: Some("jumpuser".into()),
            tags: vec!["edge".into()],
            description: None,
            jump_host_id: None,
        },
        Host {
            id: "app-id".into(),
            name: "app1".into(),
            ip: "10.0.1.10".into(),
            group: "web".into(),
            port: 2222,
            user: None,
            tags: vec!["prod".into(), "k8s".into()],
            description: Some("Production node".into()),
            jump_host_id: Some("bastion-id".into()),
        },
    ];

    let creds = vec![Credential {
        id: "cred-key".into(),
        name: "SSH Key".into(),
        username: "ubuntu".into(),
        kind: CredentialKind::Key,
        key_path: Some("/home/user/.ssh/id_rsa".into()),
    }];

    let records = vec![ServerRecord {
        host_id: "app-id".into(),
        last_credential_id: Some("cred-key".into()),
    }];

    let cfg = AppConfig {
        strict_host_checking: "no".into(),
        ssh_extra_args: "-o ForwardAgent=yes".into(),
        ..Default::default()
    };

    let inv = hss::inventory::generate_ansible_inventory(&hosts, &creds, &records, &cfg);

    // Verify _meta.hostvars
    let hostvars = inv.get("_meta").unwrap().get("hostvars").unwrap();
    let app1_vars = hostvars.get("app1").unwrap();

    assert_eq!(app1_vars.get("ansible_host").unwrap(), "10.0.1.10");
    assert_eq!(app1_vars.get("ansible_port").unwrap(), 2222);
    assert_eq!(app1_vars.get("ansible_user").unwrap(), "ubuntu");
    assert_eq!(
        app1_vars.get("ansible_ssh_private_key_file").unwrap(),
        "/home/user/.ssh/id_rsa"
    );
    assert_eq!(app1_vars.get("hss_group").unwrap(), "web");
    assert_eq!(app1_vars.get("hss_description").unwrap(), "Production node");

    let ssh_common = app1_vars
        .get("ansible_ssh_common_args")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(ssh_common.contains("-o ProxyJump=jumpuser@198.51.100.1"));
    assert!(ssh_common.contains("-o StrictHostKeyChecking=no"));
    assert!(ssh_common.contains("-o ForwardAgent=yes"));

    // Verify groups
    let web_group = inv
        .get("web")
        .unwrap()
        .get("hosts")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(web_group.contains(&serde_json::json!("app1")));

    let gateways_group = inv
        .get("gateways")
        .unwrap()
        .get("hosts")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(gateways_group.contains(&serde_json::json!("bastion")));

    // Verify tag groups
    let tag_prod = inv
        .get("tag_prod")
        .unwrap()
        .get("hosts")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(tag_prod.contains(&serde_json::json!("app1")));

    let tag_k8s = inv
        .get("tag_k8s")
        .unwrap()
        .get("hosts")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(tag_k8s.contains(&serde_json::json!("app1")));

    let tag_edge = inv
        .get("tag_edge")
        .unwrap()
        .get("hosts")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(tag_edge.contains(&serde_json::json!("bastion")));

    // Verify "all" children
    let all_children = inv
        .get("all")
        .unwrap()
        .get("children")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(all_children.contains(&serde_json::json!("gateways")));
    assert!(all_children.contains(&serde_json::json!("web")));
    assert!(all_children.contains(&serde_json::json!("tag_prod")));
    assert!(all_children.contains(&serde_json::json!("tag_k8s")));
    assert!(all_children.contains(&serde_json::json!("tag_edge")));
}
