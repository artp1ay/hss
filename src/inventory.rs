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

