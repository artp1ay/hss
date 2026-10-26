use crate::config;
use crate::ssh;
use crate::types::CredentialKind;
use anyhow::{bail, Result};
use skim::prelude::*;
use std::io::Cursor;

pub fn run() -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;

    let hosts = config::load_hosts()?;
    if hosts.is_empty() {
        bail!("No hosts configured. Run `hss` to add hosts.");
    }
    let records = config::migrate_server_records(config::load_server_records()?, &hosts);

    let lines: Vec<String> = hosts
        .iter()
        .map(|h| {
            format!(
                "{:<24} {:<14} {}:{} \x00{}",
                h.name, h.group, h.ip, h.port, h.id
            )
        })
        .collect();

    let selected_line = pick_one(&lines, "ssh> ")?;
    let Some(line) = selected_line else {
        return Ok(());
    };

    let host = if let Some((_, id)) = line.split_once('\x00') {
        hosts.iter().find(|h| h.id == id.trim())
    } else {
        let picked_name: String = line
            .chars()
            .take(24)
            .collect::<String>()
            .trim_end()
            .to_string();
        hosts.iter().find(|h| h.name == picked_name)
    }
    .ok_or_else(|| anyhow::anyhow!("Host not found for selection: {line}"))?;

    let record = records.iter().find(|r| r.host_id == host.id);

    let cred = ssh::resolve_credential(
        &creds,
        &cfg,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )?;

    let mut explicitly_selected = false;
    let cred = if let Some(c) = cred {
        c.clone()
    } else if creds.is_empty() {
        bail!("No credentials configured. Run `hss` to set up credentials.");
    } else {
        // Need to pick credential
        let cred_pairs: Vec<(&crate::types::Credential, String)> = creds
            .iter()
            .map(|c| {
                let kind = if c.kind == CredentialKind::Key {
                    "key"
                } else {
                    "password"
                };
                (c, format!("{:<20} {:<10} {}", c.name, kind, c.username))
            })
            .collect();
        let cred_lines: Vec<String> = cred_pairs.iter().map(|(_, s)| s.clone()).collect();

        let selected_cred = pick_one(&cred_lines, "credential> ")?;
        let Some(cred_line) = selected_cred else {
            return Ok(());
        };
        explicitly_selected = true;
        cred_pairs
            .iter()
            .find(|(_, s)| s == &cred_line)
            .map(|(c, _)| (*c).clone())
            .ok_or_else(|| anyhow::anyhow!("Credential not found"))?
    };

    let jump = ssh::resolve_jump_spec(&hosts, host)?;
    if explicitly_selected || cfg.auto_save_credential {
        let mut records = config::migrate_server_records(config::load_server_records()?, &hosts);
        if let Some(r) = records.iter_mut().find(|r| r.host_id == host.id) {
            r.last_credential_id = Some(cred.id.clone());
        } else {
            records.push(crate::types::ServerRecord {
                host_id: host.id.clone(),
                last_credential_id: Some(cred.id.clone()),
                default_credential_id: None,
            });
        }
        config::save_server_records(&records)?;
    }
    ssh::spawn_ssh(&host.ip, host.port, &cred, &cfg, jump.as_deref())?;

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
    if out.is_abort {
        return Ok(None);
    }

    Ok(out
        .selected_items
        .into_iter()
        .next()
        .map(|i| i.output().to_string()))
}
