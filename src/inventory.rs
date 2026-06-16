use anyhow::Result;
use crate::types::{Host, ServerRecord};

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
        let mut ansible_user = None;

        if let Some(vars) = parts.next() {
            for token in vars.split_whitespace() {
                if let Some(v) = token.strip_prefix("ansible_host=") {
                    ip = v.to_string();
                } else if let Some(v) = token.strip_prefix("ansible_port=") {
                    port = v.parse().unwrap_or(22);
                } else if let Some(v) = token.strip_prefix("ansible_user=") {
                    ansible_user = Some(v.to_string());
                }
            }
        }

        hosts.push(Host { name, ip, group: current_group.clone(), port, ansible_user });
    }
    hosts
}

pub fn sync_server_records(records: Vec<ServerRecord>, active_names: &[String]) -> Vec<ServerRecord> {
    records.into_iter().filter(|r| active_names.contains(&r.name)).collect()
}

pub fn load_and_sync(inventory_path: &str) -> Result<(Vec<Host>, Vec<ServerRecord>)> {
    let content = std::fs::read_to_string(inventory_path)?;
    let hosts = parse_inventory(&content);
    let active_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
    let records = crate::config::load_server_records()?;
    let synced = sync_server_records(records, &active_names);
    crate::config::save_server_records(&synced)?;
    Ok((hosts, synced))
}
