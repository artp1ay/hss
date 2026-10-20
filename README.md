# hss

[![CI](https://github.com/artp1ay/hss/actions/workflows/ci.yml/badge.svg)](https://github.com/artp1ay/hss/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/artp1ay/hss)](https://github.com/artp1ay/hss/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

Terminal SSH server manager. Browse servers, manage credentials, and connect — all from the keyboard.

## Features

- **TUI** — searchable server list, keyboard-driven
- **Credential store** — passwords and SSH key passphrases in macOS Keychain / Linux Secret Service, never in plaintext config files; private keys stay in their existing files
- **Ansible INI** — import and export inventory files
- **Quick connect** — `hss hostname` bypasses the TUI
- **Fuzzy picker** — `hss --fzf` for skim-powered selection

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/artp1ay/hss/main/install.sh | bash
```

Downloads the latest release for your platform and installs it to `~/.local/bin/hss`.
No root required. The script will tell you if `~/.local/bin` needs to be added to `$PATH`.

To update an existing installation:

```sh
hss --update
```

<details>
<summary>Manual install</summary>

Download a binary from [Releases](../../releases):

| Platform | File |
|---|---|
| Linux x86\_64 | `hss-linux-x86_64` |
| Linux ARM64 | `hss-linux-aarch64` |
| macOS Intel | `hss-macos-x86_64` |
| macOS Apple Silicon | `hss-macos-aarch64` |

```sh
chmod +x hss-linux-x86_64
mv hss-linux-x86_64 ~/.local/bin/hss
```

</details>

## Usage

```
hss           open TUI
hss HOST      connect directly to a named host or IP
hss --fzf     fuzzy picker
hss --ansible-inventory  output dynamic Ansible inventory JSON
```

### TUI keys

| Key | Action |
|---|---|
| `↑↓` / `j k` | Navigate |
| `Enter` | Connect |
| `/` | Search (Esc to clear, Esc again to exit) |
| `N` | Add host |
| `E` | Edit selected host |
| `D` | Delete (with confirmation) |
| `A` | Connectivity & auth audit |
| `P` | Copy public keys (`ssh-copy-id`) |
| `I` | Import / Export Ansible INI / SSH config |
| `R` | Switch credential for selected host |
| `M` | Start/view local MCP server |
| `C` | Manage credentials |
| `S` | Settings |
| `Q` | Quit |

## Credentials

Open **Manage credentials** with `C`; press `*` on a credential to make it the default.

- **Automatic selection:** the configured default takes priority over the host's last-used credential. If the configured default is not found, hss tries the last-used credential, then the only saved credential if there is exactly one.
- **Password:** requires a non-empty password.
- **Key:** uses an SSH private key file and supports an optional saved passphrase. The private key itself is not copied into the secret store.
- **Editing:** leaving the password/passphrase blank preserves the saved secret only when the credential type stays the same. When switching types, the old secret is not reused: Password requires a new password; Key accepts an optional passphrase.

Secrets are stored persistently in **macOS Keychain** or **Linux Secret Service**, not in hss config files. Secret-store failures are reported; there is no plaintext config fallback.

On Linux, hss needs a running **session D-Bus** and a Secret Service provider (such as GNOME Keyring) with an accessible, unlocked persistent collection. Configure an encrypted collection: encryption at rest depends on the provider and its settings. hss encrypts secret transfers over D-Bus. Headless/SSH sessions need access to the session bus and an unlocked collection too; the release binary's bundled libdbus does not provide these services.

**Upgrading from Linux keyutils:** old secrets are not migrated automatically. Re-enter and save each password/passphrase in Credentials, even if its metadata still appears. The old kernel-backed store does not survive a reboot.

On macOS, Keychain persists across reboots, but unlocking or access approval may be required, particularly in headless/SSH sessions.

**`org.freedesktop.secrets` / D-Bus errors:** the Secret Service provider is unavailable to the process running hss. A successful save requires a running provider and an unlocked persistent collection; reinstalling hss or re-entering a password does not start that service. On Linux, check the session bus from the same user/session as MCP:

```sh
busctl --user status org.freedesktop.secrets
```

Native macOS hss uses Keychain, not D-Bus. If a macOS client shows this error, check whether its MCP command/endpoint runs hss in Linux, a container, or a remote session. TUI and MCP must run as the same user in the same environment to share credentials. After upgrading hss, restart the MCP server in the client: replacing the binary does not update an already-running process.

## MCP Server (Model Context Protocol)

`hss` can serve as an MCP server for LLM agents (Claude Desktop, Zed, Cursor, OpenCode):

- **Tools exposed:** `list-servers`, `execute-command`.
- **Zero-knowledge security:** private keys and passwords never leave `hss`; commands are executed locally through SSH sessions.
- **DLP Protection:** automatic redaction of detected private keys and certificates in command outputs.
- **Read-Only guard:** optional blocking of destructive commands (`rm`, `sudo`, `dd`, `shutdown`).
- **Host scoping:** restrict MCP access to specific server globs or tags (e.g. `tag:mcp`).

Run over Stdio (recommended for Zed / Claude Desktop config):
```sh
hss --mcp-stdio
```

Or standalone HTTP daemon:
```sh
hss --mcp
```

## Config

| OS | Path |
|---|---|
| Linux | `~/.config/hss/` |
| macOS | `~/Library/Application Support/hss/` |

`config.toml` — settings · `hosts.toml` — server list · `servers.toml` — last-used credentials

Passwords are stored in the system keychain, not in any config file.

To configure a custom GitHub repository for self-updates (`hss --update`), set `update_repo` in `config.toml`:

```toml
update_repo = "owner/repo"
```

Or set the environment variable `HSS_UPDATE_REPO="owner/repo"`.

## Development & Releases

- **CI**: Runs format check, clippy with `-D warnings`, tests, and release build on pull requests and pushes to `main`, `master`, and `dev`.
- **Release**: Released manually on demand. You control the version and changelog notes:
  - Via release script:
    ```sh
    ./scripts/release.sh 1.0.0 "Initial public release"
    # or from file
    ./scripts/release.sh 1.0.0 @docs/release-notes.md
    ```
  - Via GitHub Actions UI (`workflow_dispatch` on the `release` workflow) specifying the tag and custom notes.
  - Or manually pushing an annotated tag:
    ```sh
    git tag -a v1.0.0 -m "Release description"
    git push origin v1.0.0
    ```
- **Artifacts**: GitHub Actions builds binaries for all 4 platforms (`hss-linux-x86_64`, `hss-linux-aarch64`, `hss-macos-x86_64`, `hss-macos-aarch64`) and computes `SHA256SUMS` and `MD5SUMS`.
