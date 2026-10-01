use uuid::Uuid;
use crate::types::Host;

pub fn parse_inventory(content: &str) -> Vec<Host> {
    let mut hosts = Vec::new();
    let mut current_group = String::from("ungrouped");

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_group = line[1..line.len() - 1].to_string();
            continue;
        }
        if current_group.ends_with(":vars") || current_group.ends_with(":children") {
            continue;
        }

        let mut parts = line.splitn(2, ' ');
        let name = match parts.next() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };

        let mut ip = name.clone();
        let mut port = 22u16;
        let mut user = None;

        if let Some(vars) = parts.next() {
            for token in vars.split_whitespace() {
                if let Some(v) = token.strip_prefix("ansible_host=").or_else(|| token.strip_prefix("ansible_ssh_host=")) {
                    ip = v.to_string();
                } else if let Some(v) = token.strip_prefix("ansible_port=").or_else(|| token.strip_prefix("ansible_ssh_port=")) {
                    port = v.parse().unwrap_or(22);
                } else if let Some(v) = token.strip_prefix("ansible_user=").or_else(|| token.strip_prefix("ansible_ssh_user=")) {
                    user = Some(v.to_string());
                }
            }
        }

        hosts.push(Host {
            id: Uuid::new_v4().to_string(),
            name,
            ip,
            group: current_group.clone(),
            port,
            user,
            tags: vec![],
            description: None,
            jump_host_id: None,
        });
    }
    hosts
}

pub fn import_from_ini(content: &str, hosts: &mut Vec<Host>) -> usize {
    let mut current_group = String::from("ungrouped");
    let mut added = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_group = line[1..line.len() - 1].to_string();
            continue;
        }
        if current_group.ends_with(":vars") || current_group.ends_with(":children") {
            continue;
        }

        // Split off inline comment: everything after " #" is a comment
        let (vars_part, comment_part) = match line.find(" #") {
            Some(idx) => (&line[..idx], &line[idx + 2..]),
            None => (line, ""),
        };

        // Parse tags from comment: "# tags=a,b" or " tags=a,b" after stripping leading " #"
        let ini_tags: Vec<String> = comment_part
            .trim()
            .strip_prefix("tags=")
            .map(|t| t.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
            .unwrap_or_default();

        let mut parts = vars_part.splitn(2, ' ');
        let name = match parts.next() {
            Some(n) if !n.is_empty() => n.to_string(),
            _ => continue,
        };

        let mut ip = name.clone();
        let mut port = 22u16;
        let mut user: Option<String> = None;

        if let Some(vars) = parts.next() {
            for token in vars.split_whitespace() {
                if let Some(v) = token.strip_prefix("ansible_host=").or_else(|| token.strip_prefix("ansible_ssh_host=")) {
                    ip = v.to_string();
                } else if let Some(v) = token.strip_prefix("ansible_port=").or_else(|| token.strip_prefix("ansible_ssh_port=")) {
                    port = v.parse().unwrap_or(22);
                } else if let Some(v) = token.strip_prefix("ansible_user=").or_else(|| token.strip_prefix("ansible_ssh_user=")) {
                    user = Some(v.to_string());
                }
            }
        }

        if let Some(existing) = hosts.iter_mut().find(|h| h.name == name) {
            existing.ip = ip;
            existing.port = port;
            if user.is_some() {
                existing.user = user;
            }
            for tag in ini_tags {
                if !existing.tags.contains(&tag) {
                    existing.tags.push(tag);
                }
            }
        } else {
            hosts.push(Host {
                id: Uuid::new_v4().to_string(),
                name,
                ip,
                group: current_group.clone(),
                port,
                user,
                tags: ini_tags,
                description: None,
                jump_host_id: None,
            });
            added += 1;
        }
    }
    added
}

/// Export hosts as an OpenSSH client config. Group, tags and description
/// go in as comments above each Host block.
pub fn export_to_ssh_config(hosts: &[Host]) -> String {
    let mut sorted: Vec<&Host> = hosts.iter().collect();
    sorted.sort_by(|a, b| a.group.cmp(&b.group).then(a.name.cmp(&b.name)));

    let mut out = String::from("# Generated by hss\n");
    for h in sorted {
        out.push('\n');
        if !h.group.is_empty() {
            out.push_str(&format!("# group: {}\n", h.group));
        }
        if !h.tags.is_empty() {
            out.push_str(&format!("# tags: {}\n", h.tags.join(", ")));
        }
        if let Some(ref d) = h.description {
            for line in d.lines() {
                out.push_str(&format!("# {line}\n"));
            }
        }
        out.push_str(&format!("Host {}\n", h.name));
        out.push_str(&format!("    HostName {}\n", h.ip));
        if h.port != 22 {
            out.push_str(&format!("    Port {}\n", h.port));
        }
        if let Some(ref u) = h.user {
            out.push_str(&format!("    User {u}\n"));
        }
        if let Some(jump) = crate::ssh::jump_spec(hosts, h) {
            out.push_str(&format!("    ProxyJump {jump}\n"));
        }
    }
    out
}

pub fn export_to_ini(hosts: &[Host]) -> String {
    let mut groups: Vec<String> = hosts.iter().map(|h| h.group.clone()).collect();
    groups.sort();
    groups.dedup();

    let mut out = String::new();
    for (i, group) in groups.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("[{}]\n", group));
        let mut group_hosts: Vec<&Host> = hosts.iter().filter(|h| &h.group == group).collect();
        group_hosts.sort_by(|a, b| a.name.cmp(&b.name));
        for host in group_hosts {
            let mut line = format!("{} ansible_host={} ansible_port={}", host.name, host.ip, host.port);
            if let Some(u) = &host.user {
                line.push_str(&format!(" ansible_user={}", u));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// Generate dynamic Ansible inventory JSON with groups, tags, credentials and jump hosts.
pub fn generate_ansible_inventory(
    hosts: &[Host],
    creds: &[crate::types::Credential],
    records: &[crate::types::ServerRecord],
    cfg: &crate::config::AppConfig,
) -> serde_json::Value {
    use std::collections::{BTreeMap, BTreeSet};
    use serde_json::json;
    use crate::types::CredentialKind;

    let mut hostvars = serde_json::Map::new();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut tag_groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for host in hosts {
        let last_cred_id = records
            .iter()
            .find(|r| r.host_id == host.id)
            .and_then(|r| r.last_credential_id.as_deref());

        let resolved_cred = crate::ssh::resolve_credential(creds, cfg, last_cred_id).ok().flatten();

        let mut vars = serde_json::Map::new();
        vars.insert("ansible_host".into(), json!(host.ip));
        vars.insert("ansible_port".into(), json!(host.port));

        let user = host
            .user
            .clone()
            .or_else(|| resolved_cred.map(|c| c.username.clone()))
            .or_else(|| cfg.default_user.clone());
        if let Some(u) = user {
            vars.insert("ansible_user".into(), json!(u));
        }

        if let Some(cred) = resolved_cred {
            match cred.kind {
                CredentialKind::Password => {
                    if let Ok(pw) = crate::credentials::get_password(&cred.id) {
                        vars.insert("ansible_password".into(), json!(pw));
                        // Non-root users with passwords typically use the same password for sudo
                        if user.as_deref() != Some("root") {
                            vars.insert("ansible_become_password".into(), json!(pw));
                        }
                    }
                }
                CredentialKind::Key => {
                    if let Some(ref key_path) = cred.key_path {
                        vars.insert("ansible_ssh_private_key_file".into(), json!(key_path));
                    }
                }
            }
        }

        let mut ssh_args = Vec::new();
        if let Some(jump) = crate::ssh::jump_spec(hosts, host) {
            ssh_args.push(format!("-o ProxyJump={jump}"));
        }
        if !cfg.strict_host_checking.is_empty() {
            ssh_args.push(format!("-o StrictHostKeyChecking={}", cfg.strict_host_checking));
        }
        if !cfg.ssh_extra_args.is_empty() {
            ssh_args.push(cfg.ssh_extra_args.clone());
        }
        if !ssh_args.is_empty() {
            vars.insert("ansible_ssh_common_args".into(), json!(ssh_args.join(" ")));
        }

        vars.insert("hss_group".into(), json!(host.group));
        vars.insert("hss_tags".into(), json!(host.tags));
        if let Some(ref desc) = host.description {
            vars.insert("hss_description".into(), json!(desc));
        }

        let group_name = if host.group.is_empty() { "ungrouped" } else { &host.group };
        groups
            .entry(group_name.to_string())
            .or_default()
            .insert(host.name.clone());

        for tag in &host.tags {
            let t = tag.trim();
            if !t.is_empty() {
                tag_groups
                    .entry(format!("tag_{t}"))
                    .or_default()
                    .insert(host.name.clone());
            }
        }

        hostvars.insert(host.name.clone(), serde_json::Value::Object(vars));
    }

    let mut inventory = serde_json::Map::new();

    let mut meta = serde_json::Map::new();
    meta.insert("hostvars".into(), serde_json::Value::Object(hostvars));
    inventory.insert("_meta".into(), serde_json::Value::Object(meta));

    let mut all_children: Vec<String> = groups.keys().cloned().collect();
    for tag_name in tag_groups.keys() {
        all_children.push(tag_name.clone());
    }
    all_children.sort();
    all_children.dedup();

    let mut all_group = serde_json::Map::new();
    all_group.insert("children".into(), json!(all_children));
    inventory.insert("all".into(), serde_json::Value::Object(all_group));

    for (group_name, members) in groups {
        let mut grp = serde_json::Map::new();
        grp.insert("hosts".into(), json!(members.into_iter().collect::<Vec<_>>()));
        inventory.insert(group_name, serde_json::Value::Object(grp));
    }

    for (tag_name, members) in tag_groups {
        let mut grp = serde_json::Map::new();
        grp.insert("hosts".into(), json!(members.into_iter().collect::<Vec<_>>()));
        inventory.insert(tag_name, serde_json::Value::Object(grp));
    }

    serde_json::Value::Object(inventory)
}

