use anyhow::{bail, Result};
use skim::prelude::*;
use std::io::Cursor;
use crate::config;
use crate::ssh;
use crate::types::CredentialKind;

pub fn run() -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;

    let hosts = config::load_hosts()?;
    if hosts.is_empty() {
        bail!("No hosts configured. Run `hss` to add hosts.");
    }

    // Build display lines: "name  group  host:port"
    let lines: Vec<String> = hosts.iter()
        .map(|h| format!("{:<20} {:<14} {}:{}", h.name, h.group, h.ip, h.port))
        .collect();

    let selected_line = pick_one(&lines, "ssh> ")?;
    let Some(line) = selected_line else { return Ok(()) };

    // Match back to host by name (first token)
    let host_name = line.split_whitespace().next().unwrap_or("").trim().to_string();
    let host = hosts.iter().find(|h| h.name == host_name)
        .ok_or_else(|| anyhow::anyhow!("Host not found: {host_name}"))?;

    let last_cred_id = records.iter()
        .find(|r| r.host_id == host.name)
        .and_then(|r| r.last_credential_id.clone());

    let cred = ssh::resolve_credential(&creds, &cfg, last_cred_id.as_deref())?;

    let cred = if let Some(c) = cred {
        c.clone()
    } else if creds.is_empty() {
        bail!("No credentials configured. Run `hss` to set up credentials.");
    } else {
        // Need to pick credential
        let cred_pairs: Vec<(&crate::types::Credential, String)> = creds.iter()
            .map(|c| {
                let kind = if c.kind == CredentialKind::Key { "key" } else { "password" };
                (c, format!("{:<20} {:<10} {}", c.name, kind, c.username))
            })
            .collect();
        let cred_lines: Vec<String> = cred_pairs.iter().map(|(_, s)| s.clone()).collect();

        let selected_cred = pick_one(&cred_lines, "credential> ")?;
        let Some(cred_line) = selected_cred else { return Ok(()) };
        cred_pairs.iter()
            .find(|(_, s)| s == &cred_line)
            .map(|(c, _)| (*c).clone())
            .ok_or_else(|| anyhow::anyhow!("Credential not found"))?
    };

    let status = ssh::spawn_ssh(&host.ip, host.port, &cred)?;
    if status.success() {
        // Save last credential
        let mut records = config::load_server_records()?;
        if let Some(r) = records.iter_mut().find(|r| r.host_id == host.name) {
            r.last_credential_id = Some(cred.id.clone());
        } else {
            records.push(crate::types::ServerRecord {
                host_id: host.name.clone(),
                last_credential_id: Some(cred.id.clone()),
            });
        }
        config::save_server_records(&records)?;
    }

    Ok(())
}

fn pick_one(items: &[String], prompt: &str) -> Result<Option<String>> {
    let options = SkimOptionsBuilder::default()
        .prompt(Some(prompt))
        .height(Some("40%"))
        .build()
        .map_err(|e| anyhow::anyhow!("skim build error: {e}"))?;

    let input = items.join("\n");
    let item_reader = SkimItemReader::default();
    let items_arc = item_reader.of_bufread(Cursor::new(input));

    let output = Skim::run_with(&options, Some(items_arc));
    let Some(out) = output else { return Ok(None) };
    if out.is_abort { return Ok(None); }

    Ok(out.selected_items.into_iter().next().map(|i| i.output().to_string()))
}
