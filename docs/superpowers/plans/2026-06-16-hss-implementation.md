# hss SSH Manager Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `hss`, a minimalist Rust TUI for SSH server management with Ansible inventory as the source of truth.

**Architecture:** Single binary with lib crate (for testability), three entry modes (TUI/fzf/direct). App data lives in `~/.config/hss/` as TOML files; passwords stored in OS keychain. SSH is spawned as a child process with inherited stdio; passwords injected via `SSH_ASKPASS` helper script in `/tmp`.

**Tech Stack:** Rust 2021 edition · ratatui 0.29 · crossterm 0.28 · keyring 3 · serde + toml 0.8 · skim 0.10 · clap 4 · uuid 1 · anyhow 1 · dirs 5

---

## File Map

```
src/
  lib.rs                        — pub re-exports of all modules (enables integration tests)
  main.rs                       — CLI parsing (clap), mode routing
  types.rs                      — Host, Credential, CredentialKind, ServerRecord, CredentialForm
  config.rs                     — read/write ~/.config/hss/{config,credentials,servers}.toml
  inventory.rs                  — parse Ansible INI; sync servers.toml on startup
  credentials.rs                — CRUD + keychain get/set/delete
  ssh.rs                        — spawn_ssh(), resolve_credential(), connect_direct()
  fzf.rs                        — skim picker for --fzf mode
  tui/
    mod.rs                      — App state, Screen enum, terminal helpers, run(), do_connect()
    main_screen.rs              — server list + fuzzy search + key handler
    popup.rs                    — credential picker overlay
    credentials_screen.rs       — credential list + add/edit/delete form
    settings_screen.rs          — inventory path input + default credential
tests/
  config_test.rs                — TOML roundtrip tests
  inventory_test.rs             — INI parsing tests
```

---

### Task 1: Project Bootstrap

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/types.rs`
- Create: stubs for all other `src/` files

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "hss"
version = "0.1.0"
edition = "2021"

[lib]
name = "hss"
path = "src/lib.rs"

[[bin]]
name = "hss"
path = "src/main.rs"

[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
crossterm = "0.28"
dirs = "5"
keyring = "3"
ratatui = "0.29"
serde = { version = "1", features = ["derive"] }
skim = "0.10"
toml = "0.8"
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 2: Create src/types.rs**

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    pub ip: String,
    pub group: String,
    pub port: u16,
    pub ansible_user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CredentialKind {
    Password,
    Key,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub name: String,
    pub username: String,
    pub kind: CredentialKind,
    pub key_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRecord {
    pub name: String,
    pub last_credential_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CredentialForm {
    pub editing_id: Option<String>, // None = new, Some(id) = editing
    pub is_key: bool,
    pub name: String,
    pub username: String,
    pub password: String,
    pub key_path: String,
    pub focused: usize, // 0=type toggle, 1=name, 2=username, 3=secret
}
```

- [ ] **Step 3: Create src/lib.rs**

```rust
pub mod config;
pub mod credentials;
pub mod fzf;
pub mod inventory;
pub mod ssh;
pub mod tui;
pub mod types;
```

- [ ] **Step 4: Create src/main.rs**

```rust
use clap::Parser;

#[derive(Parser)]
#[command(name = "hss", about = "SSH manager — connect to your servers")]
struct Cli {
    /// Quick fzf picker mode
    #[arg(long)]
    fzf: bool,
    /// Connect directly to host by name or IP
    host: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match (cli.fzf, cli.host) {
        (true, _) => hss::fzf::run(),
        (_, Some(host)) => hss::ssh::connect_direct(&host),
        _ => hss::tui::run(),
    }
}
```

- [ ] **Step 5: Create stub files so the project compiles**

`src/config.rs`:
```rust
use serde::{Deserialize, Serialize};
use crate::types::{Credential, ServerRecord};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub inventory_path: Option<String>,
    pub default_credential_id: Option<String>,
}

pub fn load_config() -> anyhow::Result<AppConfig> { Ok(AppConfig::default()) }
pub fn save_config(_: &AppConfig) -> anyhow::Result<()> { Ok(()) }
pub fn load_credentials() -> anyhow::Result<Vec<Credential>> { Ok(vec![]) }
pub fn save_credentials(_: &[Credential]) -> anyhow::Result<()> { Ok(()) }
pub fn load_server_records() -> anyhow::Result<Vec<ServerRecord>> { Ok(vec![]) }
pub fn save_server_records(_: &[ServerRecord]) -> anyhow::Result<()> { Ok(()) }
pub fn serialize_credentials(_: &[Credential]) -> anyhow::Result<String> { Ok(String::new()) }
pub fn parse_credentials(_: &str) -> anyhow::Result<Vec<Credential>> { Ok(vec![]) }
pub fn serialize_server_records(_: &[ServerRecord]) -> anyhow::Result<String> { Ok(String::new()) }
pub fn parse_server_records(_: &str) -> anyhow::Result<Vec<ServerRecord>> { Ok(vec![]) }
```

`src/inventory.rs`:
```rust
use crate::types::{Host, ServerRecord};
pub fn parse_inventory(_: &str) -> Vec<Host> { vec![] }
pub fn sync_server_records(r: Vec<ServerRecord>, _: &[String]) -> Vec<ServerRecord> { r }
pub fn load_and_sync(_: &str) -> anyhow::Result<(Vec<Host>, Vec<ServerRecord>)> { Ok((vec![], vec![])) }
```

`src/credentials.rs`:
```rust
use crate::types::{Credential, CredentialKind};
pub fn add_credential(_: &str, _: &str, _: CredentialKind, _: Option<String>, _: Option<&str>) -> anyhow::Result<Credential> { todo!() }
pub fn update_credential(_: &str, _: &str, _: &str, _: CredentialKind, _: Option<String>, _: Option<&str>) -> anyhow::Result<()> { todo!() }
pub fn delete_credential(_: &str) -> anyhow::Result<()> { todo!() }
pub fn get_password(_: &str) -> anyhow::Result<String> { todo!() }
```

`src/ssh.rs`:
```rust
use crate::types::{Credential};
use crate::config::AppConfig;
pub fn spawn_ssh(_: &str, _: u16, _: &Credential) -> anyhow::Result<std::process::ExitStatus> { todo!() }
pub fn resolve_credential<'a>(_: &'a [Credential], _: &AppConfig, _: Option<&str>) -> anyhow::Result<Option<&'a Credential>> { Ok(None) }
pub fn connect_direct(_: &str) -> anyhow::Result<()> { todo!() }
```

`src/fzf.rs`:
```rust
pub fn run() -> anyhow::Result<()> { todo!() }
```

`src/tui/mod.rs`:
```rust
pub mod credentials_screen;
pub mod main_screen;
pub mod popup;
pub mod settings_screen;

pub fn run() -> anyhow::Result<()> { todo!() }
```

`src/tui/main_screen.rs`, `src/tui/credentials_screen.rs`, `src/tui/settings_screen.rs`, `src/tui/popup.rs` — each empty.

- [ ] **Step 6: Verify it compiles**

```bash
cargo build 2>&1 | grep "^error" | head -20
```

Expected: no errors (todo! panics and unused warnings are fine).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/
git commit -m "feat: bootstrap project structure"
```

---

### Task 2: Config Module

**Files:**
- Modify: `src/config.rs`
- Create: `tests/config_test.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/config_test.rs`:
```rust
#[test]
fn test_appconfig_default_has_no_fields() {
    let cfg = hss::config::AppConfig::default();
    assert!(cfg.inventory_path.is_none());
    assert!(cfg.default_credential_id.is_none());
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
    assert_eq!(parsed[0].key_path, Some("/home/user/.ssh/id_rsa".into()));
}

#[test]
fn test_password_credential_roundtrip() {
    use hss::types::{Credential, CredentialKind};
    let creds = vec![Credential {
        id: "pw1".into(),
        name: "admin".into(),
        username: "admin".into(),
        kind: CredentialKind::Password,
        key_path: None,
    }];
    let s = hss::config::serialize_credentials(&creds).unwrap();
    let parsed = hss::config::parse_credentials(&s).unwrap();
    assert_eq!(parsed[0].kind, hss::types::CredentialKind::Password);
    assert!(parsed[0].key_path.is_none());
}

#[test]
fn test_server_records_roundtrip() {
    use hss::types::ServerRecord;
    let records = vec![
        ServerRecord { name: "web1".into(), last_credential_id: Some("cred-1".into()) },
        ServerRecord { name: "db1".into(), last_credential_id: None },
    ];
    let s = hss::config::serialize_server_records(&records).unwrap();
    let parsed = hss::config::parse_server_records(&s).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].name, "web1");
    assert_eq!(parsed[0].last_credential_id, Some("cred-1".into()));
    assert_eq!(parsed[1].last_credential_id, None);
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test --test config_test 2>&1 | tail -15
```

Expected: failures because stub implementations return empty/default values.

- [ ] **Step 3: Implement config.rs**

```rust
use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::types::{Credential, ServerRecord};

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()))
        .join("hss")
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub inventory_path: Option<String>,
    pub default_credential_id: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct CredentialsFile {
    #[serde(default, rename = "credential")]
    credentials: Vec<Credential>,
}

#[derive(Serialize, Deserialize, Default)]
struct ServersFile {
    #[serde(default, rename = "server")]
    servers: Vec<ServerRecord>,
}

pub fn load_config() -> Result<AppConfig> {
    let path = config_dir().join("config.toml");
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}

pub fn save_config(cfg: &AppConfig) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_dir().join("config.toml"), toml::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn serialize_credentials(creds: &[Credential]) -> Result<String> {
    Ok(toml::to_string_pretty(&CredentialsFile { credentials: creds.to_vec() })?)
}

pub fn parse_credentials(s: &str) -> Result<Vec<Credential>> {
    Ok(toml::from_str::<CredentialsFile>(s)?.credentials)
}

pub fn load_credentials() -> Result<Vec<Credential>> {
    let path = config_dir().join("credentials.toml");
    if !path.exists() {
        return Ok(vec![]);
    }
    parse_credentials(&std::fs::read_to_string(path)?)
}

pub fn save_credentials(creds: &[Credential]) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_dir().join("credentials.toml"), serialize_credentials(creds)?)?;
    Ok(())
}

pub fn serialize_server_records(records: &[ServerRecord]) -> Result<String> {
    Ok(toml::to_string_pretty(&ServersFile { servers: records.to_vec() })?)
}

pub fn parse_server_records(s: &str) -> Result<Vec<ServerRecord>> {
    Ok(toml::from_str::<ServersFile>(s)?.servers)
}

pub fn load_server_records() -> Result<Vec<ServerRecord>> {
    let path = config_dir().join("servers.toml");
    if !path.exists() {
        return Ok(vec![]);
    }
    parse_server_records(&std::fs::read_to_string(path)?)
}

pub fn save_server_records(records: &[ServerRecord]) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(config_dir().join("servers.toml"), serialize_server_records(records)?)?;
    Ok(())
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test --test config_test 2>&1
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs tests/config_test.rs Cargo.toml
git commit -m "feat: config module with TOML persistence"
```

---

### Task 3: Inventory Module

**Files:**
- Modify: `src/inventory.rs`
- Create: `tests/inventory_test.rs`

- [ ] **Step 1: Write failing tests**

Create `tests/inventory_test.rs`:
```rust
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
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --test inventory_test 2>&1 | tail -10
```

Expected: test failures (stub returns empty vec).

- [ ] **Step 3: Implement inventory.rs**

```rust
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
```

- [ ] **Step 4: Run tests**

```bash
cargo test --test inventory_test 2>&1
```

Expected: all 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/inventory.rs tests/inventory_test.rs
git commit -m "feat: inventory module with Ansible INI parser"
```

---

### Task 4: Credentials Module

**Files:**
- Modify: `src/credentials.rs`

- [ ] **Step 1: Write inline tests**

The credentials module calls the OS keychain so we test the logic we can test — argument validation and config interaction. Add to `src/credentials.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_key_credential_does_not_require_password() {
        // Key credentials should not attempt keychain access
        // We test by verifying the function signature accepts None for password
        // and kind=Key with a key_path
        use crate::types::CredentialKind;
        // Can't call add_credential without real config dir; test the helper only
        assert_eq!(keychain_key("some-uuid"), "some-uuid");
    }
}
```

- [ ] **Step 2: Implement credentials.rs**

```rust
use anyhow::{bail, Result};
use uuid::Uuid;
use crate::config;
use crate::types::{Credential, CredentialKind};

const KEYCHAIN_SERVICE: &str = "hss";

fn keychain_key(id: &str) -> &str {
    id
}

pub fn add_credential(
    name: &str,
    username: &str,
    kind: CredentialKind,
    key_path: Option<String>,
    password: Option<&str>,
) -> Result<Credential> {
    let id = Uuid::new_v4().to_string();
    if kind == CredentialKind::Password {
        let pw = password.ok_or_else(|| anyhow::anyhow!("password required for Password credential"))?;
        keyring::Entry::new(KEYCHAIN_SERVICE, &id)?.set_password(pw)?;
    }
    let cred = Credential { id, name: name.to_string(), username: username.to_string(), kind, key_path };
    let mut creds = config::load_credentials()?;
    creds.push(cred.clone());
    config::save_credentials(&creds)?;
    Ok(cred)
}

pub fn update_credential(
    id: &str,
    name: &str,
    username: &str,
    kind: CredentialKind,
    key_path: Option<String>,
    password: Option<&str>,
) -> Result<()> {
    if kind == CredentialKind::Password {
        if let Some(pw) = password {
            if !pw.is_empty() {
                keyring::Entry::new(KEYCHAIN_SERVICE, id)?.set_password(pw)?;
            }
        }
    }
    let mut creds = config::load_credentials()?;
    let cred = creds.iter_mut().find(|c| c.id == id)
        .ok_or_else(|| anyhow::anyhow!("credential not found: {id}"))?;
    cred.name = name.to_string();
    cred.username = username.to_string();
    cred.kind = kind;
    cred.key_path = key_path;
    config::save_credentials(&creds)
}

pub fn delete_credential(id: &str) -> Result<()> {
    // Ignore keychain error if entry doesn't exist
    let _ = keyring::Entry::new(KEYCHAIN_SERVICE, id).and_then(|e| e.delete_credential());
    let mut creds = config::load_credentials()?;
    creds.retain(|c| c.id != id);
    config::save_credentials(&creds)
}

pub fn get_password(id: &str) -> Result<String> {
    Ok(keyring::Entry::new(KEYCHAIN_SERVICE, id)?.get_password()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keychain_key_is_id() {
        assert_eq!(keychain_key("some-uuid"), "some-uuid");
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test credentials::tests 2>&1
```

Expected: 1 test passes.

- [ ] **Step 4: Commit**

```bash
git add src/credentials.rs
git commit -m "feat: credentials module with keychain integration"
```

---

### Task 5: SSH Module

**Files:**
- Modify: `src/ssh.rs`

- [ ] **Step 1: Write inline tests**

These tests cover `resolve_credential` — the only logic we can unit-test without spawning a real SSH process.

- [ ] **Step 2: Implement ssh.rs**

```rust
use anyhow::Result;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use crate::config::{self, AppConfig};
use crate::credentials;
use crate::inventory;
use crate::types::{Credential, CredentialKind};

pub fn resolve_credential<'a>(
    creds: &'a [Credential],
    cfg: &AppConfig,
    last_id: Option<&str>,
) -> Result<Option<&'a Credential>> {
    if let Some(id) = last_id {
        if let Some(c) = creds.iter().find(|c| c.id == id) {
            return Ok(Some(c));
        }
    }
    if let Some(ref id) = cfg.default_credential_id {
        if let Some(c) = creds.iter().find(|c| &c.id == id) {
            return Ok(Some(c));
        }
    }
    if creds.len() == 1 {
        return Ok(Some(&creds[0]));
    }
    Ok(None)
}

pub fn spawn_ssh(host: &str, port: u16, cred: &Credential) -> Result<std::process::ExitStatus> {
    let password = if cred.kind == CredentialKind::Password {
        Some(credentials::get_password(&cred.id)?)
    } else {
        None
    };

    let mut ssh_args: Vec<String> = vec![
        "-o".into(), "StrictHostKeyChecking=accept-new".into(),
        "-p".into(), port.to_string(),
        "-l".into(), cred.username.clone(),
    ];
    if let Some(ref key) = cred.key_path {
        ssh_args.extend_from_slice(&["-i".into(), key.clone()]);
    }
    ssh_args.push(host.to_string());

    let mut cmd = Command::new("ssh");
    cmd.args(&ssh_args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());

    let askpass_path = if let Some(ref pw) = password {
        let path = write_askpass_helper(pw)?;
        cmd.env("SSH_ASKPASS", &path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("HSS_PASSWORD", pw);
        Some(path)
    } else {
        None
    };

    let status = cmd.spawn()?.wait()?;

    if let Some(path) = askpass_path {
        let _ = std::fs::remove_file(path);
    }

    Ok(status)
}

fn write_askpass_helper(password: &str) -> Result<std::path::PathBuf> {
    let path = std::env::temp_dir().join(format!("hss-askpass-{}", std::process::id()));
    std::fs::write(&path, "#!/bin/sh\necho \"$HSS_PASSWORD\"\n")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path)
}

pub fn connect_direct(host: &str) -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;

    let (ssh_host, port, last_cred_id) = if let Some(ref inv_path) = cfg.inventory_path {
        if std::path::Path::new(inv_path).exists() {
            let (hosts, records) = inventory::load_and_sync(inv_path)?;
            if let Some(h) = hosts.into_iter().find(|h| h.name == host || h.ip == host) {
                let last_id = records.iter()
                    .find(|r| r.name == h.name)
                    .and_then(|r| r.last_credential_id.clone());
                (h.ip, h.port, last_id)
            } else {
                (host.to_string(), 22, None)
            }
        } else {
            (host.to_string(), 22, None)
        }
    } else {
        (host.to_string(), 22, None)
    };

    let cred = resolve_credential(&creds, &cfg, last_cred_id.as_deref())?
        .ok_or_else(|| anyhow::anyhow!("No credentials configured. Run `hss` to set up credentials."))?;

    spawn_ssh(&ssh_host, port, cred)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Credential, CredentialKind};

    fn key_cred(id: &str) -> Credential {
        Credential { id: id.into(), name: "t".into(), username: "u".into(), kind: CredentialKind::Key, key_path: None }
    }

    #[test]
    fn resolve_last_id_wins() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let result = resolve_credential(&creds, &AppConfig::default(), Some("b")).unwrap();
        assert_eq!(result.unwrap().id, "b");
    }

    #[test]
    fn resolve_default_fallback() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig { default_credential_id: Some("a".into()), ..Default::default() };
        let result = resolve_credential(&creds, &cfg, None).unwrap();
        assert_eq!(result.unwrap().id, "a");
    }

    #[test]
    fn resolve_single_cred_auto() {
        let creds = vec![key_cred("only")];
        let result = resolve_credential(&creds, &AppConfig::default(), None).unwrap();
        assert_eq!(result.unwrap().id, "only");
    }

    #[test]
    fn resolve_none_when_ambiguous() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let result = resolve_credential(&creds, &AppConfig::default(), None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn resolve_unknown_last_id_falls_to_default() {
        let creds = vec![key_cred("a"), key_cred("b")];
        let cfg = AppConfig { default_credential_id: Some("b".into()), ..Default::default() };
        // last_id points to non-existent credential
        let result = resolve_credential(&creds, &cfg, Some("nonexistent")).unwrap();
        assert_eq!(result.unwrap().id, "b");
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test ssh::tests 2>&1
```

Expected: all 5 tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/ssh.rs
git commit -m "feat: SSH module with askpass helper and credential resolution"
```

---

### Task 6: TUI Foundation

**Files:**
- Modify: `src/tui/mod.rs`

- [ ] **Step 1: Implement tui/mod.rs**

```rust
use std::io::{self, Stdout};
use anyhow::Result;
use crossterm::{execute, terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen}};
use ratatui::{backend::CrosstermBackend, Terminal};
use crate::config::AppConfig;
use crate::types::{Credential, CredentialForm, Host, ServerRecord};

pub mod credentials_screen;
pub mod main_screen;
pub mod popup;
pub mod settings_screen;

pub type Term = Terminal<CrosstermBackend<Stdout>>;

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Main,
    Credentials,
    Settings,
    CredentialPicker { host_idx: usize, after_failure: bool },
}

pub struct App {
    pub screen: Screen,
    pub hosts: Vec<Host>,
    pub credentials: Vec<Credential>,
    pub config: AppConfig,
    pub server_records: Vec<ServerRecord>,
    // Main screen state
    pub search_query: String,
    pub selected_row: usize,
    pub search_focused: bool,
    // Credentials screen state
    pub cred_selected: usize,
    pub cred_form: Option<CredentialForm>,
    // Settings screen state
    pub settings_inventory_input: String,
    pub settings_focused_field: usize,
    // Popup state
    pub popup_selected: usize,
    // Global
    pub should_quit: bool,
    pub status_message: Option<String>,
}

impl App {
    pub fn new(hosts: Vec<Host>, credentials: Vec<Credential>, config: AppConfig, server_records: Vec<ServerRecord>) -> Self {
        let settings_inventory_input = config.inventory_path.clone().unwrap_or_default();
        Self {
            screen: Screen::Main,
            hosts,
            credentials,
            config,
            server_records,
            search_query: String::new(),
            selected_row: 0,
            search_focused: true,
            cred_selected: 0,
            cred_form: None,
            settings_inventory_input,
            settings_focused_field: 0,
            popup_selected: 0,
            should_quit: false,
            status_message: None,
        }
    }

    pub fn filtered_hosts(&self) -> Vec<&Host> {
        if self.search_query.is_empty() {
            return self.hosts.iter().collect();
        }
        let q = self.search_query.to_lowercase();
        self.hosts.iter().filter(|h| {
            h.name.to_lowercase().contains(&q)
                || h.group.to_lowercase().contains(&q)
                || h.ip.to_lowercase().contains(&q)
        }).collect()
    }

    pub fn last_credential_id(&self, host_name: &str) -> Option<&str> {
        self.server_records.iter()
            .find(|r| r.name == host_name)
            .and_then(|r| r.last_credential_id.as_deref())
    }

    pub fn save_last_credential(&mut self, host_name: &str, cred_id: &str) -> Result<()> {
        if let Some(r) = self.server_records.iter_mut().find(|r| r.name == host_name) {
            r.last_credential_id = Some(cred_id.to_string());
        } else {
            self.server_records.push(ServerRecord {
                name: host_name.to_string(),
                last_credential_id: Some(cred_id.to_string()),
            });
        }
        crate::config::save_server_records(&self.server_records)
    }

    pub fn reload_credentials(&mut self) -> Result<()> {
        self.credentials = crate::config::load_credentials()?;
        Ok(())
    }
}

pub fn setup_terminal() -> Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

pub fn restore_terminal(terminal: &mut Term) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

/// Tear down TUI, run SSH, re-init TUI. Saves credential on success; shows picker on auth failure.
pub fn do_connect(terminal: &mut Term, app: &mut App, host_name: &str, cred: &Credential) -> Result<()> {
    let host = app.hosts.iter().find(|h| h.name == host_name || h.ip == host_name).cloned();
    let (ip, port) = host.as_ref().map(|h| (h.ip.clone(), h.port)).unwrap_or_else(|| (host_name.to_string(), 22));

    restore_terminal(terminal)?;
    let status = crate::ssh::spawn_ssh(&ip, port, cred)?;
    *terminal = setup_terminal()?;

    if status.success() {
        app.save_last_credential(host_name, &cred.id)?;
        app.status_message = None;
    } else if status.code() == Some(255) {
        // Likely auth failure — show picker
        if let Some(idx) = app.hosts.iter().position(|h| h.name == host_name) {
            app.screen = Screen::CredentialPicker { host_idx: idx, after_failure: true };
        }
        app.status_message = Some("Authentication failed. Choose different credentials.".into());
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let cfg = crate::config::load_config()?;

    if cfg.inventory_path.is_none() {
        let mut terminal = setup_terminal()?;
        let mut app = App::new(vec![], crate::config::load_credentials()?, cfg, vec![]);
        app.screen = Screen::Settings;
        let result = run_loop(&mut terminal, &mut app);
        restore_terminal(&mut terminal)?;
        return result;
    }

    let inv_path = cfg.inventory_path.as_deref().unwrap();
    let (hosts, records) = if std::path::Path::new(inv_path).exists() {
        crate::inventory::load_and_sync(inv_path)?
    } else {
        (vec![], vec![])
    };
    let credentials = crate::config::load_credentials()?;

    let mut terminal = setup_terminal()?;
    let mut app = App::new(hosts, credentials, cfg, records);
    let result = run_loop(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(terminal: &mut Term, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if crossterm::event::poll(std::time::Duration::from_millis(100))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                handle_key(terminal, app, key)?;
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

fn draw(f: &mut ratatui::Frame, app: &App) {
    match &app.screen {
        Screen::Main => main_screen::draw(f, app),
        Screen::Credentials => credentials_screen::draw(f, app),
        Screen::Settings => settings_screen::draw(f, app),
        Screen::CredentialPicker { .. } => {
            main_screen::draw(f, app);
            popup::draw(f, app);
        }
    }
}

fn handle_key(terminal: &mut Term, app: &mut App, key: crossterm::event::KeyEvent) -> Result<()> {
    let screen = app.screen.clone();
    match screen {
        Screen::Main => main_screen::handle_key(terminal, app, key),
        Screen::Credentials => credentials_screen::handle_key(terminal, app, key),
        Screen::Settings => settings_screen::handle_key(app, key),
        Screen::CredentialPicker { host_idx, after_failure } => {
            popup::handle_key(terminal, app, key, host_idx, after_failure)
        }
    }
}
```

- [ ] **Step 2: Update stub tui submodules to match new signatures**

`src/tui/main_screen.rs`:
```rust
use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Term};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if key.code == crossterm::event::KeyCode::Char('q') { app.should_quit = true; }
    Ok(())
}
```

`src/tui/credentials_screen.rs`:
```rust
use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if key.code == crossterm::event::KeyCode::Esc { app.screen = Screen::Main; }
    Ok(())
}
```

`src/tui/settings_screen.rs`:
```rust
use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Screen};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.code == crossterm::event::KeyCode::Esc { app.screen = Screen::Main; }
    Ok(())
}
```

`src/tui/popup.rs`:
```rust
use ratatui::Frame;
use crossterm::event::KeyEvent;
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) { let _ = (f, app); }
pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent, _host_idx: usize, _after_failure: bool) -> Result<()> {
    if key.code == crossterm::event::KeyCode::Esc { app.screen = Screen::Main; }
    Ok(())
}
```

- [ ] **Step 3: Verify compilation and all tests still pass**

```bash
cargo build 2>&1 | grep "^error" && cargo test 2>&1 | tail -5
```

Expected: no build errors, all previous tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/tui/
git commit -m "feat: TUI foundation — App state, terminal helpers, event loop"
```

---

### Task 7: Main Screen

**Files:**
- Modify: `src/tui/main_screen.rs`

- [ ] **Step 1: Implement draw()**

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let hosts = app.filtered_hosts();

    // Title bar
    let title = Line::from(vec![
        Span::styled("hss", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        Span::styled(format!(" · {} hosts", hosts.len()), Style::default().fg(Color::DarkGray)),
        if let Some(ref msg) = app.status_message {
            Span::styled(format!("  ⚠ {msg}"), Style::default().fg(Color::Yellow))
        } else {
            Span::raw("")
        },
    ]);
    f.render_widget(Paragraph::new(title), chunks[0]);

    // Search box
    let border_style = if app.search_focused {
        Style::default().fg(Color::Blue)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let search_content = Line::from(vec![
        Span::styled("🔍 ", Style::default().fg(Color::DarkGray)),
        Span::styled(&app.search_query, Style::default().fg(Color::White)),
        if app.search_focused { Span::styled("█", Style::default().fg(Color::Blue)) } else { Span::raw("") },
    ]);
    f.render_widget(
        Paragraph::new(search_content).block(Block::default().borders(Borders::ALL).border_style(border_style)),
        chunks[1],
    );

    // Server table
    let header = Row::new(vec!["NAME", "GROUP", "HOST", "PORT", "LAST CONN"])
        .style(Style::default().fg(Color::DarkGray));

    let rows: Vec<Row> = hosts.iter().map(|h| {
        Row::new(vec![
            Cell::from(h.name.clone()),
            Cell::from(h.group.clone()).style(Style::default().fg(group_color(&h.group))),
            Cell::from(h.ip.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(h.port.to_string()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(
                app.server_records.iter()
                    .find(|r| r.name == h.name && r.last_credential_id.is_some())
                    .map(|_| "✓")
                    .unwrap_or("—")
            ).style(Style::default().fg(Color::DarkGray)),
        ])
    }).collect();

    let selected = app.selected_row.min(hosts.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if hosts.is_empty() { None } else { Some(selected) });

    let table = Table::new(rows, [
        Constraint::Length(22),
        Constraint::Length(16),
        Constraint::Length(18),
        Constraint::Length(6),
        Constraint::Min(8),
    ])
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[2], &mut state);

    // Hotkey bar
    let hotkeys = if app.search_focused {
        hotkey_line(&[("Tab", "table"), ("Esc", "clear")])
    } else {
        hotkey_line(&[("Enter", "connect"), ("R", "switch creds"), ("C", "credentials"), ("S", "settings"), ("Tab", "search"), ("Q", "quit")])
    };
    f.render_widget(Paragraph::new(hotkeys), chunks[3]);
}

fn hotkey_line<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = vec![];
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 { spans.push(Span::raw("  ")); }
        spans.push(Span::styled(*key, Style::default().fg(Color::Blue)));
        spans.push(Span::styled(format!("={label}"), Style::default().fg(Color::DarkGray)));
    }
    Line::from(spans)
}

fn group_color(group: &str) -> Color {
    let hash: usize = group.bytes().map(|b| b as usize).sum::<usize>() % 5;
    [Color::Green, Color::Cyan, Color::Yellow, Color::Magenta, Color::LightBlue][hash]
}
```

- [ ] **Step 2: Implement handle_key()**

Append to `src/tui/main_screen.rs`:

```rust
pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    let hosts_len = app.filtered_hosts().len();

    match key.code {
        KeyCode::Tab => {
            app.search_focused = !app.search_focused;
        }
        KeyCode::Esc if app.search_focused => {
            if !app.search_query.is_empty() {
                app.search_query.clear();
                app.selected_row = 0;
            } else {
                app.search_focused = false;
            }
        }
        KeyCode::Char(c) if app.search_focused => {
            app.search_query.push(c);
            app.selected_row = 0;
        }
        KeyCode::Backspace if app.search_focused => {
            app.search_query.pop();
            app.selected_row = 0;
        }
        KeyCode::Down | KeyCode::Char('j') if !app.search_focused => {
            if hosts_len > 0 {
                app.selected_row = (app.selected_row + 1).min(hosts_len - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') if !app.search_focused => {
            app.selected_row = app.selected_row.saturating_sub(1);
        }
        KeyCode::Char('q') | KeyCode::Char('Q') if !app.search_focused => {
            app.should_quit = true;
        }
        KeyCode::Char('c') | KeyCode::Char('C') if !app.search_focused => {
            app.screen = Screen::Credentials;
            app.cred_selected = 0;
        }
        KeyCode::Char('s') | KeyCode::Char('S') if !app.search_focused => {
            app.settings_inventory_input = app.config.inventory_path.clone().unwrap_or_default();
            app.screen = Screen::Settings;
        }
        KeyCode::Char('r') | KeyCode::Char('R') if !app.search_focused && hosts_len > 0 => {
            let idx_in_all = get_host_idx_in_all(app);
            if let Some(idx) = idx_in_all {
                app.popup_selected = 0;
                app.screen = Screen::CredentialPicker { host_idx: idx, after_failure: false };
            }
        }
        KeyCode::Enter if !app.search_focused && hosts_len > 0 => {
            connect_selected(terminal, app)?;
        }
        _ => {}
    }
    Ok(())
}

fn get_host_idx_in_all(app: &App) -> Option<usize> {
    let filtered = app.filtered_hosts();
    if filtered.is_empty() { return None; }
    let host_name = &filtered[app.selected_row.min(filtered.len() - 1)].name;
    app.hosts.iter().position(|h| &h.name == host_name)
}

fn connect_selected(terminal: &mut Term, app: &mut App) -> Result<()> {
    let filtered = app.filtered_hosts();
    if filtered.is_empty() { return Ok(()); }
    let host = filtered[app.selected_row.min(filtered.len() - 1)].clone();

    let last_cred_id = app.last_credential_id(&host.name).map(|s| s.to_string());
    let cred = crate::ssh::resolve_credential(&app.credentials, &app.config, last_cred_id.as_deref())?
        .cloned();

    if let Some(cred) = cred {
        crate::tui::do_connect(terminal, app, &host.name, &cred)?;
    } else {
        // No credential resolved — show picker
        if let Some(idx) = app.hosts.iter().position(|h| h.name == host.name) {
            app.popup_selected = 0;
            app.screen = Screen::CredentialPicker { host_idx: idx, after_failure: false };
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Build and smoke-test manually**

```bash
cargo build 2>&1 | grep "^error"
```

Then run `cargo run` — the TUI should launch with an empty table (no inventory set yet) and `Q` should quit.

Expected: TUI launches, shows "hss · 0 hosts", `Q` quits cleanly.

- [ ] **Step 4: Commit**

```bash
git add src/tui/main_screen.rs
git commit -m "feat: main screen with search, table, and keyboard navigation"
```

---

### Task 8: Credential Picker Popup

**Files:**
- Modify: `src/tui/popup.rs`

- [ ] **Step 1: Implement popup draw()**

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen, Term};

pub fn draw(f: &mut Frame, app: &App) {
    let Screen::CredentialPicker { host_idx, after_failure } = &app.screen else { return };
    let host = &app.hosts[*host_idx];

    let area = centered_rect(60, 50, f.area());
    f.render_widget(Clear, area);

    let title = if *after_failure {
        format!(" Auth failed — choose credentials for {} ", host.name)
    } else {
        format!(" Credentials for {} ", host.name)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    // Credential list
    let rows: Vec<Row> = app.credentials.iter().map(|c| {
        let kind = match c.kind {
            crate::types::CredentialKind::Key => "key",
            crate::types::CredentialKind::Password => "password",
        };
        let default_marker = if app.config.default_credential_id.as_deref() == Some(&c.id) { "★" } else { "" };
        Row::new(vec![
            Cell::from(c.name.clone()),
            Cell::from(kind).style(Style::default().fg(Color::DarkGray)),
            Cell::from(c.username.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(default_marker).style(Style::default().fg(Color::Yellow)),
        ])
    }).collect();

    let selected = app.popup_selected.min(app.credentials.len().saturating_sub(1));
    let mut state = TableState::default().with_selected(if app.credentials.is_empty() { None } else { Some(selected) });

    let table = Table::new(rows, [
        Constraint::Min(16),
        Constraint::Length(10),
        Constraint::Length(14),
        Constraint::Length(2),
    ])
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[0], &mut state);

    // Hotkeys
    let hotkeys = Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Blue)),
        Span::styled("=connect  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Blue)),
        Span::styled("=cancel", Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(hotkeys), chunks[1]);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
```

- [ ] **Step 2: Implement handle_key()**

Append to `src/tui/popup.rs`:

```rust
pub fn handle_key(terminal: &mut Term, app: &mut App, key: KeyEvent, host_idx: usize, _after_failure: bool) -> Result<()> {
    let creds_len = app.credentials.len();

    match key.code {
        KeyCode::Esc => {
            app.screen = Screen::Main;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if creds_len > 0 {
                app.popup_selected = (app.popup_selected + 1).min(creds_len - 1);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.popup_selected = app.popup_selected.saturating_sub(1);
        }
        KeyCode::Enter if creds_len > 0 => {
            let cred = app.credentials[app.popup_selected.min(creds_len - 1)].clone();
            let host_name = app.hosts[host_idx].name.clone();
            app.screen = Screen::Main;
            crate::tui::do_connect(terminal, app, &host_name, &cred)?;
        }
        _ => {}
    }
    Ok(())
}
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/tui/popup.rs
git commit -m "feat: credential picker popup overlay"
```

---

### Task 9: Credentials Screen

**Files:**
- Modify: `src/tui/credentials_screen.rs`

- [ ] **Step 1: Implement draw()**

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen, Term};
use crate::types::{CredentialForm, CredentialKind};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("hss", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" · credentials", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    let header = Row::new(vec!["NAME", "TYPE", "USERNAME", "DEFAULT"])
        .style(Style::default().fg(Color::DarkGray));

    let rows: Vec<Row> = app.credentials.iter().map(|c| {
        let kind_str = if c.kind == CredentialKind::Key { "key" } else { "password" };
        let default_marker = if app.config.default_credential_id.as_deref() == Some(&c.id) {
            "★ default"
        } else {
            ""
        };
        Row::new(vec![
            Cell::from(c.name.clone()),
            Cell::from(kind_str).style(Style::default().fg(Color::DarkGray)),
            Cell::from(c.username.clone()).style(Style::default().fg(Color::DarkGray)),
            Cell::from(default_marker).style(Style::default().fg(Color::Yellow)),
        ])
    }).collect();

    let selected = app.cred_selected.min(app.credentials.len().saturating_sub(1));
    let mut state = TableState::default()
        .with_selected(if app.credentials.is_empty() { None } else { Some(selected) });

    let table = Table::new(rows, [
        Constraint::Min(18),
        Constraint::Length(10),
        Constraint::Length(16),
        Constraint::Length(10),
    ])
    .header(header)
    .row_highlight_style(Style::default().bg(Color::Rgb(31, 41, 55)))
    .highlight_symbol("▶ ");

    f.render_stateful_widget(table, chunks[1], &mut state);

    f.render_widget(
        Paragraph::new(hotkey_line(&[("A", "add"), ("E", "edit"), ("D", "delete"), ("*", "set default"), ("Esc", "back")])),
        chunks[2],
    );

    // Draw form overlay if active
    if let Some(ref form) = app.cred_form {
        draw_form(f, form, f.area());
    }
}

fn draw_form(f: &mut Frame, form: &CredentialForm, area: Rect) {
    let popup = centered_rect(55, 60, area);
    f.render_widget(Clear, popup);

    let title = if form.editing_id.is_some() { " Edit credential " } else { " New credential " };
    let block = Block::default().title(title).borders(Borders::ALL).border_style(Style::default().fg(Color::Blue));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // type toggle
            Constraint::Length(2), // name
            Constraint::Length(2), // username
            Constraint::Length(2), // password or key_path
            Constraint::Length(1), // hotkeys
        ])
        .margin(1)
        .split(inner);

    // Type toggle
    let (pw_style, key_style) = if form.is_key {
        (Style::default().fg(Color::DarkGray), Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
    } else {
        (Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD), Style::default().fg(Color::DarkGray))
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Type: ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Password]", pw_style),
            Span::raw("  "),
            Span::styled("[Key]", key_style),
        ])),
        chunks[0],
    );

    // Fields
    render_field(f, "Name:    ", &form.name, form.focused == 1, chunks[1]);
    render_field(f, "Username:", &form.username, form.focused == 2, chunks[2]);
    if form.is_key {
        render_field(f, "Key path:", &form.key_path, form.focused == 3, chunks[3]);
    } else {
        let masked: String = "•".repeat(form.password.len());
        render_field(f, "Password:", &masked, form.focused == 3, chunks[3]);
    }

    f.render_widget(
        Paragraph::new(hotkey_line(&[("Tab", "next field"), ("Space", "toggle type"), ("Enter", "save"), ("Esc", "cancel")])),
        chunks[4],
    );
}

fn render_field(f: &mut Frame, label: &str, value: &str, focused: bool, area: Rect) {
    let border_style = if focused { Style::default().fg(Color::Blue) } else { Style::default().fg(Color::DarkGray) };
    let display = if focused { format!("{value}█") } else { value.to_string() };
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(10), Constraint::Min(0)])
        .split(area);
    f.render_widget(Paragraph::new(label).style(Style::default().fg(Color::DarkGray)), chunks[0]);
    f.render_widget(
        Paragraph::new(display).block(Block::default().borders(Borders::BOTTOM).border_style(border_style)),
        chunks[1],
    );
}

fn hotkey_line<'a>(pairs: &[(&'a str, &'a str)]) -> Line<'a> {
    let mut spans = vec![];
    for (i, (key, label)) in pairs.iter().enumerate() {
        if i > 0 { spans.push(Span::raw("  ")); }
        spans.push(Span::styled(*key, Style::default().fg(Color::Blue)));
        spans.push(Span::styled(format!("={label}"), Style::default().fg(Color::DarkGray)));
    }
    Line::from(spans)
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
```

- [ ] **Step 2: Implement handle_key()**

Append to `src/tui/credentials_screen.rs`:

```rust
pub fn handle_key(_terminal: &mut Term, app: &mut App, key: KeyEvent) -> Result<()> {
    if let Some(ref mut form) = app.cred_form.clone() {
        handle_form_key(app, key, form)?;
    } else {
        handle_list_key(app, key)?;
    }
    Ok(())
}

fn handle_list_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let creds_len = app.credentials.len();
    match key.code {
        KeyCode::Esc => { app.screen = Screen::Main; }
        KeyCode::Down | KeyCode::Char('j') => {
            if creds_len > 0 { app.cred_selected = (app.cred_selected + 1).min(creds_len - 1); }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.cred_selected = app.cred_selected.saturating_sub(1);
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            app.cred_form = Some(CredentialForm::default());
        }
        KeyCode::Char('e') | KeyCode::Char('E') if creds_len > 0 => {
            let c = &app.credentials[app.cred_selected.min(creds_len - 1)];
            app.cred_form = Some(CredentialForm {
                editing_id: Some(c.id.clone()),
                is_key: c.kind == CredentialKind::Key,
                name: c.name.clone(),
                username: c.username.clone(),
                key_path: c.key_path.clone().unwrap_or_default(),
                password: String::new(), // don't pre-fill password
                focused: 1,
            });
        }
        KeyCode::Char('d') | KeyCode::Char('D') if creds_len > 0 => {
            let id = app.credentials[app.cred_selected.min(creds_len - 1)].id.clone();
            crate::credentials::delete_credential(&id)?;
            // Also clear default if it was this one
            if app.config.default_credential_id.as_deref() == Some(&id) {
                app.config.default_credential_id = None;
                crate::config::save_config(&app.config)?;
            }
            app.reload_credentials()?;
            app.cred_selected = app.cred_selected.saturating_sub(1);
        }
        KeyCode::Char('*') if creds_len > 0 => {
            let id = app.credentials[app.cred_selected.min(creds_len - 1)].id.clone();
            app.config.default_credential_id = Some(id);
            crate::config::save_config(&app.config)?;
        }
        _ => {}
    }
    Ok(())
}

fn handle_form_key(app: &mut App, key: KeyEvent, mut form: CredentialForm) -> Result<()> {
    match key.code {
        KeyCode::Esc => { app.cred_form = None; }
        KeyCode::Tab => {
            form.focused = (form.focused + 1) % 4;
            app.cred_form = Some(form);
        }
        KeyCode::Char(' ') if form.focused == 0 => {
            form.is_key = !form.is_key;
            app.cred_form = Some(form);
        }
        KeyCode::Enter => {
            save_form(app, &form)?;
            app.cred_form = None;
        }
        KeyCode::Backspace => {
            match form.focused {
                1 => { form.name.pop(); }
                2 => { form.username.pop(); }
                3 if form.is_key => { form.key_path.pop(); }
                3 => { form.password.pop(); }
                _ => {}
            }
            app.cred_form = Some(form);
        }
        KeyCode::Char(c) => {
            match form.focused {
                1 => form.name.push(c),
                2 => form.username.push(c),
                3 if form.is_key => form.key_path.push(c),
                3 => form.password.push(c),
                _ => {}
            }
            app.cred_form = Some(form);
        }
        _ => {}
    }
    Ok(())
}

fn save_form(app: &mut App, form: &CredentialForm) -> Result<()> {
    let kind = if form.is_key { CredentialKind::Key } else { CredentialKind::Password };
    let key_path = if form.is_key && !form.key_path.is_empty() { Some(form.key_path.clone()) } else { None };
    let password = if !form.is_key && !form.password.is_empty() { Some(form.password.as_str()) } else { None };

    if let Some(ref id) = form.editing_id {
        crate::credentials::update_credential(id, &form.name, &form.username, kind, key_path, password)?;
    } else {
        crate::credentials::add_credential(&form.name, &form.username, kind, key_path, password)?;
    }
    app.reload_credentials()?;
    Ok(())
}
```

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/tui/credentials_screen.rs
git commit -m "feat: credentials screen with list, add, edit, delete, and set-default"
```

---

### Task 10: Settings Screen

**Files:**
- Modify: `src/tui/settings_screen.rs`

- [ ] **Step 1: Implement settings_screen.rs**

```rust
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use crossterm::event::{KeyCode, KeyEvent};
use anyhow::Result;
use crate::tui::{App, Screen};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .margin(1)
        .split(area);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("hss", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(" · settings", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[0],
    );

    // Inventory path field
    let inv_focused = app.settings_focused_field == 0;
    let border_style = if inv_focused { Style::default().fg(Color::Blue) } else { Style::default().fg(Color::DarkGray) };
    let inv_display = format!(
        "{}{}",
        app.settings_inventory_input,
        if inv_focused { "█" } else { "" }
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("ANSIBLE INVENTORY PATH", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM))),
            Line::from(Span::styled(inv_display, Style::default().fg(Color::White))),
            Line::from(Span::styled("Read-only source of host data", Style::default().fg(Color::DarkGray))),
        ])
        .block(Block::default().borders(Borders::ALL).border_style(border_style)),
        chunks[1],
    );

    // Default credential
    let default_name = app.config.default_credential_id.as_deref()
        .and_then(|id| app.credentials.iter().find(|c| c.id == id))
        .map(|c| format!("★ {} ({})", c.name, c.username))
        .unwrap_or_else(|| "not set — go to Credentials to set".into());
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("DEFAULT CREDENTIAL", Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM))),
            Line::from(Span::styled(default_name, Style::default().fg(Color::Yellow))),
        ])
        .block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::DarkGray))),
        chunks[2],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Blue)),
            Span::styled("=save  ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::Blue)),
            Span::styled("=back", Style::default().fg(Color::DarkGray)),
        ])),
        chunks[4],
    );
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc => {
            // Discard changes
            app.settings_inventory_input = app.config.inventory_path.clone().unwrap_or_default();
            if app.config.inventory_path.is_some() {
                app.screen = Screen::Main;
            }
            // If first-run (no inventory set), stay on settings until Enter
        }
        KeyCode::Enter => {
            let path = app.settings_inventory_input.trim().to_string();
            if !path.is_empty() {
                app.config.inventory_path = Some(path.clone());
                crate::config::save_config(&app.config)?;
                // Reload inventory
                if std::path::Path::new(&path).exists() {
                    let (hosts, records) = crate::inventory::load_and_sync(&path)?;
                    app.hosts = hosts;
                    app.server_records = records;
                }
                app.screen = Screen::Main;
            }
        }
        KeyCode::Backspace => {
            app.settings_inventory_input.pop();
        }
        KeyCode::Char(c) => {
            app.settings_inventory_input.push(c);
        }
        _ => {}
    }
    Ok(())
}
```

- [ ] **Step 2: Build and run**

```bash
cargo build 2>&1 | grep "^error"
```

Then `cargo run` — navigate to Settings with `S`, type a path, press Enter. Verify it returns to main.

- [ ] **Step 3: Commit**

```bash
git add src/tui/settings_screen.rs
git commit -m "feat: settings screen with inventory path and default credential display"
```

---

### Task 11: FZF Mode

**Files:**
- Modify: `src/fzf.rs`

- [ ] **Step 1: Implement fzf.rs**

```rust
use anyhow::{bail, Result};
use skim::prelude::*;
use std::io::Cursor;
use crate::config::{self, AppConfig};
use crate::ssh;
use crate::types::Credential;

pub fn run() -> Result<()> {
    let cfg = config::load_config()?;
    let creds = config::load_credentials()?;
    let records = config::load_server_records()?;

    let hosts = if let Some(ref path) = cfg.inventory_path {
        if std::path::Path::new(path).exists() {
            crate::inventory::load_and_sync(path)?.0
        } else {
            bail!("Inventory file not found: {path}");
        }
    } else {
        bail!("No inventory configured. Run `hss` to set up.");
    };

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
        .find(|r| r.name == host.name)
        .and_then(|r| r.last_credential_id.clone());

    let cred = ssh::resolve_credential(&creds, &cfg, last_cred_id.as_deref())?;

    let cred = if let Some(c) = cred {
        c.clone()
    } else if creds.is_empty() {
        bail!("No credentials configured. Run `hss` to set up credentials.");
    } else {
        // Need to pick credential
        let cred_lines: Vec<String> = creds.iter()
            .map(|c| {
                let kind = if c.kind == crate::types::CredentialKind::Key { "key" } else { "password" };
                format!("{:<20} {:<10} {}", c.name, kind, c.username)
            })
            .collect();

        let selected_cred = pick_one(&cred_lines, "credential> ")?;
        let Some(cred_line) = selected_cred else { return Ok(()) };
        let cred_name = cred_line.split_whitespace().next().unwrap_or("").to_string();
        creds.iter().find(|c| c.name == cred_name)
            .ok_or_else(|| anyhow::anyhow!("Credential not found"))?
            .clone()
    };

    let status = ssh::spawn_ssh(&host.ip, host.port, &cred)?;
    if status.success() {
        // Save last credential
        let mut records = config::load_server_records()?;
        if let Some(r) = records.iter_mut().find(|r| r.name == host.name) {
            r.last_credential_id = Some(cred.id.clone());
        } else {
            records.push(crate::types::ServerRecord {
                name: host.name.clone(),
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
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | grep "^error"
```

Expected: no errors.

- [ ] **Step 3: Manual test (requires real inventory)**

```bash
# If you have an inventory file, test with:
# cargo run -- --fzf
# Otherwise just verify it compiles cleanly
echo "Build OK"
```

- [ ] **Step 4: Commit**

```bash
git add src/fzf.rs
git commit -m "feat: fzf quick mode with skim picker and credential selection"
```

---

### Task 12: Integration and Direct Connect

**Files:**
- Verify: `src/ssh.rs` `connect_direct()` (already implemented in Task 5)
- Verify: `src/main.rs` routing (already in Task 1)

- [ ] **Step 1: Verify all tests pass**

```bash
cargo test 2>&1
```

Expected: all unit and integration tests pass.

- [ ] **Step 2: Verify CLI help output**

```bash
cargo run -- --help
```

Expected:
```
SSH manager — connect to your servers

Usage: hss [OPTIONS] [HOST]

Arguments:
  [HOST]  Connect directly to host by name or IP

Options:
      --fzf    Quick fzf picker mode
  -h, --help   Print help
  -V, --version  Print version
```

- [ ] **Step 3: Test direct connect routing**

```bash
# Verify the binary accepts host argument (will fail to connect — that's fine)
cargo run -- --help
cargo run -- fakehost 2>&1 | head -5
```

Expected: starts and fails with a clear error (no inventory configured or ssh connection refused), not a panic.

- [ ] **Step 4: Final integration commit**

```bash
cargo build --release 2>&1 | grep "^error"
git add -A
git commit -m "feat: complete hss SSH manager"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement | Covered |
|---|---|
| TUI mode: search box active on start | Task 7 — `search_focused: true` in App::new |
| TUI mode: server table with name/group/ip/port | Task 7 — draw() |
| TUI mode: hotkey bar | Task 7 — draw() |
| Credentials: add/edit/delete login+pass or login+key | Task 9 |
| Credentials: set default | Task 9 — `*` key |
| Credentials: popup on connect if no saved choice | Task 8 + Task 7 connect_selected() |
| Passwords not visible, not in history | Task 5 — SSH_ASKPASS helper |
| Save last-used credential per server | Task 6 do_connect(), Task 11 fzf.rs |
| R key to force credential repick | Task 7 handle_key |
| Settings: ansible inventory path | Task 10 |
| First-run: open settings if no inventory | Task 6 tui/mod.rs run() |
| Inventory sync on startup (remove deleted hosts) | Task 3 — load_and_sync |
| Read name/ip/group/port/ansible_user from inventory | Task 3 — parse_inventory |
| Quick fzf mode (--fzf) | Task 11 |
| Second skim picker for credentials in fzf mode | Task 11 — pick_one for credentials |
| Use saved creds first in fzf mode | Task 11 — resolve_credential |
| Direct connect (hss <host>) | Task 5 — connect_direct |
| Direct connect works even if not in inventory | Task 5 — fallback to (host, 22) |
| Exit button | Task 7 — Q to quit |
