use crate::types::Host;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use uuid::Uuid;

#[derive(Default)]
struct GroupVars {
    user: Option<String>,
    port: Option<u16>,
}

/// Strip an inline comment (`#` or `;` preceded by whitespace) outside quotes.
/// Returns `(before_comment, comment_body)`.
fn split_inline_comment(line: &str) -> (&str, &str) {
    let mut in_single = false;
    let mut in_double = false;
    let mut prev_ws = false;

    for (idx, ch) in line.char_indices() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                prev_ws = false;
            }
            '"' if !in_single => {
                in_double = !in_double;
                prev_ws = false;
            }
            '#' | ';' if !in_single && !in_double && prev_ws => {
                return (line[..idx].trim_end(), line[idx + 1..].trim());
            }
            c if c.is_ascii_whitespace() => {
                prev_ws = true;
            }
            _ => {
                prev_ws = false;
            }
        }
    }
    (line.trim_end(), "")
}

/// Parse an INI section header like `[webservers]` or `[webservers] # comment`.
fn parse_section_header(line: &str) -> Option<String> {
    let (head, _) = split_inline_comment(line);
    let inner = head.trim().strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

/// Split host token from the rest of the line on any ASCII whitespace (spaces or tabs).
fn split_first_token(s: &str) -> Option<(&str, &str)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    match s.find(|c: char| c.is_ascii_whitespace()) {
        Some(idx) => Some((&s[..idx], s[idx..].trim_start())),
        None => Some((s, "")),
    }
}

/// Tokenize whitespace-separated `key=value` pairs while preserving quoted values.
fn tokenize_ini_vars(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;

    for ch in s.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            c if c.is_ascii_whitespace() && !in_single && !in_double => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn unquote_ini_value(v: &str) -> &str {
    let v = v.trim();
    if v.len() >= 2
        && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')))
    {
        &v[1..v.len() - 1]
    } else {
        v
    }
}

/// Split optional `host:port` or `[ipv6]:port` notation.
fn split_host_and_port(spec: &str) -> (&str, Option<u16>) {
    if let Some(rest) = spec.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            let host = &rest[..close];
            let after = &rest[close + 1..];
            if let Some(port_str) = after.strip_prefix(':') {
                if let Ok(port) = port_str.parse::<u16>() {
                    if port > 0 {
                        return (host, Some(port));
                    }
                }
            } else if after.is_empty() {
                return (host, None);
            }
        }
        return (spec, None);
    }

    // Only split `host:port` when there is a single colon (do not break bare IPv6 literals)
    if let Some(idx) = spec.find(':') {
        if !spec[idx + 1..].contains(':') {
            if let Ok(port) = spec[idx + 1..].parse::<u16>() {
                if port > 0 {
                    return (&spec[..idx], Some(port));
                }
            }
        }
    }
    (spec, None)
}

fn parse_ini_kv(
    token: &str,
    explicit_ip: &mut Option<String>,
    explicit_port: &mut Option<u16>,
    explicit_user: &mut Option<String>,
) {
    if let Some(v) = token
        .strip_prefix("ansible_host=")
        .or_else(|| token.strip_prefix("ansible_ssh_host="))
    {
        let val = unquote_ini_value(v);
        if !val.is_empty() {
            *explicit_ip = Some(val.to_string());
        }
    } else if let Some(v) = token
        .strip_prefix("ansible_port=")
        .or_else(|| token.strip_prefix("ansible_ssh_port="))
    {
        if let Ok(p) = unquote_ini_value(v).parse::<u16>() {
            if p > 0 {
                *explicit_port = Some(p);
            }
        }
    } else if let Some(v) = token
        .strip_prefix("ansible_user=")
        .or_else(|| token.strip_prefix("ansible_ssh_user="))
    {
        let val = unquote_ini_value(v);
        if !val.is_empty() {
            *explicit_user = Some(val.to_string());
        }
    }
}

fn apply_group_var_line(line: &str, gv: &mut GroupVars) {
    let (clean, _) = split_inline_comment(line);
    let Some((k, v)) = clean.split_once('=') else {
        return;
    };
    let key = k.trim();
    let val = unquote_ini_value(v);
    if val.is_empty() {
        return;
    }
    match key {
        "ansible_user" | "ansible_ssh_user" => {
            if crate::ssh::validate_username(val).is_ok() {
                gv.user = Some(val.to_string());
            }
        }
        "ansible_port" | "ansible_ssh_port" => {
            if let Ok(p) = val.parse::<u16>() {
                if p > 0 {
                    gv.port = Some(p);
                }
            }
        }
        _ => {}
    }
}

fn sanitize_single_line(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c == '\r' || c == '\n' || c.is_control() {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn sanitize_token(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !c.is_ascii_whitespace())
        .collect()
}

fn sanitize_ansible_group_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "ungrouped".to_string();
    }
    if trimmed.eq_ignore_ascii_case("all") || trimmed.eq_ignore_ascii_case("_meta") {
        // Namespace reserved group names so they do not collide with top-level Ansible keys
        return "grp_reserved".to_string();
    }
    let sanitized: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "ungrouped".to_string()
    } else if sanitized.eq_ignore_ascii_case("all") || sanitized.eq_ignore_ascii_case("_meta") {
        "grp_reserved".to_string()
    } else {
        sanitized
    }
}

struct ParsedIniHost {
    host: Host,
    has_explicit_port: bool,
}

pub fn parse_inventory(content: &str) -> Vec<Host> {
    let mut parsed: Vec<ParsedIniHost> = Vec::new();
    let mut group_vars: HashMap<String, GroupVars> = HashMap::new();
    let mut current_group = String::from("ungrouped");

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = parse_section_header(line) {
            current_group = section;
            continue;
        }
        if let Some(group_key) = current_group.strip_suffix(":vars") {
            apply_group_var_line(line, group_vars.entry(group_key.to_string()).or_default());
            continue;
        }
        if current_group.ends_with(":children") {
            continue;
        }

        let (vars_part, _) = split_inline_comment(line);
        let Some((raw_name, rest)) = split_first_token(vars_part) else {
            continue;
        };

        let (host_spec, inline_port) = split_host_and_port(raw_name);
        if host_spec.is_empty() {
            continue;
        }
        let name = host_spec.to_string();
        let mut explicit_ip: Option<String> = None;
        let mut explicit_port: Option<u16> = inline_port;
        let mut explicit_user: Option<String> = None;

        if !rest.is_empty() {
            for token in tokenize_ini_vars(rest) {
                parse_ini_kv(
                    &token,
                    &mut explicit_ip,
                    &mut explicit_port,
                    &mut explicit_user,
                );
            }
        }

        let ip = explicit_ip.unwrap_or_else(|| name.clone());
        let has_explicit_port = explicit_port.is_some();

        parsed.push(ParsedIniHost {
            host: Host {
                id: Uuid::new_v4().to_string(),
                name,
                ip,
                group: current_group.clone(),
                port: explicit_port.unwrap_or(22),
                user: explicit_user,
                tags: vec![],
                description: None,
                jump_host_id: None,
                port_forwards: vec![],
                socks_port: None,
                http_proxy_port: None,
            },
            has_explicit_port,
        });
    }

    // Apply `[group:vars]` and `[all:vars]` fallbacks without overriding explicit host settings
    let all_vars = group_vars.get("all");
    for entry in &mut parsed {
        let gv = group_vars.get(&entry.host.group);
        if !entry.has_explicit_port {
            if let Some(p) = gv.and_then(|v| v.port).or_else(|| all_vars.and_then(|v| v.port)) {
                entry.host.port = p;
            }
        }
        if entry.host.user.is_none() {
            if let Some(u) = gv
                .and_then(|v| v.user.clone())
                .or_else(|| all_vars.and_then(|v| v.user.clone()))
            {
                entry.host.user = Some(u);
            }
        }
    }

    parsed.into_iter().map(|p| p.host).collect()
}

pub fn import_from_ini(content: &str, hosts: &mut Vec<Host>) -> usize {
    let mut current_group = String::from("ungrouped");
    let mut group_vars: HashMap<String, GroupVars> = HashMap::new();
    let mut added_indices: Vec<(usize, bool)> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(section) = parse_section_header(line) {
            current_group = section;
            continue;
        }
        if let Some(group_key) = current_group.strip_suffix(":vars") {
            apply_group_var_line(line, group_vars.entry(group_key.to_string()).or_default());
            continue;
        }
        if current_group.ends_with(":children") {
            continue;
        }

        let (vars_part, comment_part) = split_inline_comment(line);

        // Parse tags from comment: "# tags=a,b"
        let ini_tags: Vec<String> = comment_part
            .trim()
            .strip_prefix("tags=")
            .map(|t| {
                t.split(',')
                    .map(sanitize_single_line)
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let Some((raw_name, rest)) = split_first_token(vars_part) else {
            continue;
        };

        let (host_spec, inline_port) = split_host_and_port(raw_name);
        if host_spec.is_empty() || host_spec.starts_with('-') {
            continue;
        }
        let name = host_spec.to_string();

        let mut explicit_ip: Option<String> = None;
        let mut explicit_port: Option<u16> = inline_port;
        let mut explicit_user: Option<String> = None;

        if !rest.is_empty() {
            for token in tokenize_ini_vars(rest) {
                parse_ini_kv(
                    &token,
                    &mut explicit_ip,
                    &mut explicit_port,
                    &mut explicit_user,
                );
            }
        }

        // Validate host address and username to block SSH option injection on import
        let candidate_ip = explicit_ip.as_deref().unwrap_or(&name);
        if crate::ssh::validate_host(candidate_ip).is_err() {
            continue;
        }
        let explicit_user = explicit_user.filter(|u| crate::ssh::validate_username(u).is_ok());

        if let Some(existing) = hosts.iter_mut().find(|h| h.name == name) {
            // Partially merge: update ONLY fields that were explicitly present
            if let Some(ip) = explicit_ip {
                existing.ip = ip;
            }
            if let Some(port) = explicit_port {
                existing.port = port;
            }
            if explicit_user.is_some() {
                existing.user = explicit_user;
            }
            for tag in ini_tags {
                if !existing.tags.contains(&tag) {
                    existing.tags.push(tag);
                }
            }
        } else {
            let has_explicit_port = explicit_port.is_some();
            hosts.push(Host {
                id: Uuid::new_v4().to_string(),
                name: name.clone(),
                ip: explicit_ip.unwrap_or(name),
                group: current_group.clone(),
                port: explicit_port.unwrap_or(22),
                user: explicit_user,
                tags: ini_tags,
                description: None,
                jump_host_id: None,
                port_forwards: vec![],
                socks_port: None,
                http_proxy_port: None,
            });
            added_indices.push((hosts.len() - 1, has_explicit_port));
        }
    }

    // Apply `[group:vars]` and `[all:vars]` fallbacks to newly imported hosts
    let all_vars = group_vars.get("all");
    for (idx, has_explicit_port) in &added_indices {
        let host = &mut hosts[*idx];
        let gv = group_vars.get(&host.group);
        if !has_explicit_port {
            if let Some(p) = gv.and_then(|v| v.port).or_else(|| all_vars.and_then(|v| v.port)) {
                host.port = p;
            }
        }
        if host.user.is_none() {
            if let Some(u) = gv
                .and_then(|v| v.user.clone())
                .or_else(|| all_vars.and_then(|v| v.user.clone()))
            {
                host.user = Some(u);
            }
        }
    }

    added_indices.len()
}

/// Export hosts as an OpenSSH client config. Group, tags and description
/// go in as comments above each Host block.
pub fn export_to_ssh_config(hosts: &[Host]) -> String {
    let mut sorted: Vec<&Host> = hosts.iter().collect();
    sorted.sort_by(|a, b| a.group.cmp(&b.group).then(a.name.cmp(&b.name)));

    let mut out = String::from("# Generated by hss\n");
    for h in sorted {
        let safe_name = sanitize_token(&h.name);
        let safe_ip = sanitize_token(&h.ip);
        if safe_name.is_empty() || safe_ip.is_empty() {
            continue;
        }
        out.push('\n');
        let safe_group = sanitize_single_line(&h.group);
        if !safe_group.is_empty() {
            out.push_str(&format!("# group: {safe_group}\n"));
        }
        if !h.tags.is_empty() {
            let safe_tags: Vec<String> = h
                .tags
                .iter()
                .map(|t| sanitize_single_line(t))
                .filter(|t| !t.is_empty())
                .collect();
            if !safe_tags.is_empty() {
                out.push_str(&format!("# tags: {}\n", safe_tags.join(", ")));
            }
        }
        if let Some(ref d) = h.description {
            for line in d.lines() {
                let clean = sanitize_single_line(line);
                out.push_str(&format!("# {clean}\n"));
            }
        }
        out.push_str(&format!("Host {safe_name}\n"));
        out.push_str(&format!("    HostName {safe_ip}\n"));
        if h.port != 22 {
            out.push_str(&format!("    Port {}\n", h.port));
        }
        if let Some(ref u) = h.user {
            let safe_user = sanitize_token(u);
            if !safe_user.is_empty() {
                out.push_str(&format!("    User {safe_user}\n"));
            }
        }
        if let Some(jump) = crate::ssh::jump_spec(hosts, h) {
            let safe_jump = sanitize_token(&jump);
            if !safe_jump.is_empty() {
                out.push_str(&format!("    ProxyJump {safe_jump}\n"));
            }
        }
    }
    out
}

pub fn export_to_ini(hosts: &[Host]) -> String {
    let mut grouped: BTreeMap<String, Vec<&Host>> = BTreeMap::new();
    for host in hosts {
        let raw_group = sanitize_single_line(&host.group)
            .replace(['[', ']'], "_");
        let group = if raw_group.is_empty() {
            "ungrouped".to_string()
        } else {
            raw_group
        };
        grouped.entry(group).or_default().push(host);
    }

    let mut out = String::new();
    for (i, (group, mut group_hosts)) in grouped.into_iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("[{group}]\n"));
        group_hosts.sort_by(|a, b| a.name.cmp(&b.name));
        for host in group_hosts {
            let safe_name = sanitize_token(&host.name);
            let safe_ip = sanitize_token(&host.ip);
            if safe_name.is_empty() || safe_ip.is_empty() {
                continue;
            }
            let mut line = format!(
                "{safe_name} ansible_host={safe_ip} ansible_port={}",
                host.port
            );
            if let Some(u) = &host.user {
                let safe_user = sanitize_token(u);
                if !safe_user.is_empty() {
                    line.push_str(&format!(" ansible_user={safe_user}"));
                }
            }
            let safe_tags: Vec<String> = host
                .tags
                .iter()
                .map(|t| sanitize_single_line(t))
                .filter(|t| !t.is_empty())
                .collect();
            if !safe_tags.is_empty() {
                line.push_str(&format!(" # tags={}", safe_tags.join(",")));
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

fn build_hostvars(
    host: &Host,
    hosts: &[Host],
    creds: &[crate::types::Credential],
    records: &[crate::types::ServerRecord],
    cfg: &crate::config::AppConfig,
    include_secrets: bool,
    secret_cache: &mut HashMap<String, Option<String>>,
) -> serde_json::Map<String, serde_json::Value> {
    use crate::types::CredentialKind;
    use serde_json::json;

    let record = records.iter().find(|r| r.host_id == host.id);

    let resolved_cred = crate::ssh::resolve_credential(
        creds,
        cfg,
        record.and_then(|r| r.last_credential_id.as_deref()),
        record.and_then(|r| r.default_credential_id.as_deref()),
    )
    .ok()
    .flatten();

    let mut vars = serde_json::Map::new();
    vars.insert("ansible_host".into(), json!(host.ip));
    vars.insert("ansible_port".into(), json!(host.port));

    // For password credentials, always pair the password with the credential's own username
    // (matches ssh::effective_username and avoids pairing another user with a root password).
    let user = match resolved_cred {
        Some(cred) if cred.kind == CredentialKind::Password => Some(cred.username.clone()),
        Some(cred) => host
            .user
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .map(str::to_string)
            .or_else(|| Some(cred.username.clone()))
            .or_else(|| cfg.default_user.clone()),
        None => host
            .user
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .map(str::to_string)
            .or_else(|| cfg.default_user.clone()),
    };
    if let Some(ref u) = user {
        vars.insert("ansible_user".into(), json!(u));
    }

    let mut has_private_key = false;
    if let Some(cred) = resolved_cred {
        match cred.kind {
            CredentialKind::Password => {
                if include_secrets {
                    let cached = secret_cache
                        .entry(cred.id.clone())
                        .or_insert_with(|| crate::credentials::get_password(&cred.id).ok());
                    if let Some(pw) = cached.as_deref().filter(|p| !p.is_empty()) {
                        vars.insert("ansible_password".into(), json!(pw));
                        // Non-root users with passwords typically use the same password for sudo
                        if user.as_deref() != Some("root") {
                            vars.insert("ansible_become_password".into(), json!(pw));
                        }
                    }
                }
            }
            CredentialKind::Key => {
                if let Some(ref key_path) = cred.key_path {
                    let trimmed = key_path.trim();
                    if !trimmed.is_empty() {
                        let expanded = crate::config::expand_tilde(trimmed);
                        vars.insert("ansible_ssh_private_key_file".into(), json!(expanded));
                        has_private_key = true;
                    }
                }
            }
        }
    }

    let mut ssh_args = Vec::new();
    if has_private_key {
        ssh_args.push("-o IdentitiesOnly=yes".to_string());
    }
    if let Some(jump) = crate::ssh::jump_spec(hosts, host) {
        ssh_args.push(format!("-o ProxyJump={jump}"));
    }
    if !cfg.strict_host_checking.trim().is_empty() {
        if let Ok(checked) = crate::config::validate_strict_host_checking(&cfg.strict_host_checking)
        {
            ssh_args.push(format!("-o StrictHostKeyChecking={checked}"));
        }
    }
    if !cfg.ssh_extra_args.trim().is_empty() {
        ssh_args.push(cfg.ssh_extra_args.trim().to_string());
    }
    if !ssh_args.is_empty() {
        vars.insert("ansible_ssh_common_args".into(), json!(ssh_args.join(" ")));
    }

    vars.insert("hss_group".into(), json!(host.group));
    vars.insert("hss_tags".into(), json!(host.tags));
    if let Some(ref desc) = host.description {
        vars.insert("hss_description".into(), json!(desc));
    }

    vars
}

/// Generate dynamic Ansible inventory JSON with groups, tags, credentials and jump hosts.
pub fn generate_ansible_inventory(
    hosts: &[Host],
    creds: &[crate::types::Credential],
    records: &[crate::types::ServerRecord],
    cfg: &crate::config::AppConfig,
) -> serde_json::Value {
    generate_ansible_inventory_with_options(hosts, creds, records, cfg, true)
}

/// Generate dynamic Ansible inventory JSON, optionally omitting plaintext passwords.
pub fn generate_ansible_inventory_with_options(
    hosts: &[Host],
    creds: &[crate::types::Credential],
    records: &[crate::types::ServerRecord],
    cfg: &crate::config::AppConfig,
    include_secrets: bool,
) -> serde_json::Value {
    use serde_json::json;

    let mut hostvars = serde_json::Map::new();
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut secret_cache: HashMap<String, Option<String>> = HashMap::new();

    for host in hosts {
        let vars = build_hostvars(
            host,
            hosts,
            creds,
            records,
            cfg,
            include_secrets,
            &mut secret_cache,
        );

        let group_name = sanitize_ansible_group_name(&host.group);
        groups
            .entry(group_name)
            .or_default()
            .insert(host.name.clone());

        for tag in &host.tags {
            let t = tag.trim();
            if !t.is_empty() {
                let tag_group = sanitize_ansible_group_name(&format!("tag_{t}"));
                // Merge into `groups` so a tag group never overwrites a regular group with the same name
                groups
                    .entry(tag_group)
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

    let all_children: Vec<String> = groups.keys().cloned().collect();
    let mut all_group = serde_json::Map::new();
    all_group.insert("children".into(), json!(all_children));
    inventory.insert("all".into(), serde_json::Value::Object(all_group));

    for (group_name, members) in groups {
        let mut grp = serde_json::Map::new();
        grp.insert(
            "hosts".into(),
            json!(members.into_iter().collect::<Vec<_>>()),
        );
        inventory.insert(group_name, serde_json::Value::Object(grp));
    }

    serde_json::Value::Object(inventory)
}

/// Generate Ansible hostvars for a single target host (used by `--host <name>`).
/// Avoids fetching credentials for unrelated hosts.
pub fn generate_ansible_hostvars(
    target: &str,
    hosts: &[Host],
    creds: &[crate::types::Credential],
    records: &[crate::types::ServerRecord],
    cfg: &crate::config::AppConfig,
    include_secrets: bool,
) -> serde_json::Value {
    let Some(host) = hosts.iter().find(|h| h.name == target) else {
        return serde_json::Value::Object(serde_json::Map::new());
    };
    let mut secret_cache: HashMap<String, Option<String>> = HashMap::new();
    serde_json::Value::Object(build_hostvars(
        host,
        hosts,
        creds,
        records,
        cfg,
        include_secrets,
        &mut secret_cache,
    ))
}
