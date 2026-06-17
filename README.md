# hss

Terminal SSH server manager. Browse servers, manage credentials, and connect — all from the keyboard.

## Features

- **TUI** — searchable server list, keyboard-driven
- **Credential store** — passwords and SSH keys in the system keychain (macOS Keychain / Linux keyutils), never stored on disk
- **Ansible INI** — import and export inventory files
- **Quick connect** — `hss hostname` bypasses the TUI
- **Fuzzy picker** — `hss --fzf` for skim-powered selection

## Install

Download a binary from [Releases](../../releases):

| Platform | File |
|---|---|
| Linux x86\_64 | `hss-linux-x86_64` |
| Linux ARM64 | `hss-linux-aarch64` |
| macOS Intel | `hss-macos-x86_64` |
| macOS Apple Silicon | `hss-macos-aarch64` |

```sh
chmod +x hss-linux-x86_64
sudo mv hss-linux-x86_64 /usr/local/bin/hss
```

## Usage

```
hss           open TUI
hss HOST      connect directly to a named host or IP
hss --fzf     fuzzy picker
```

### TUI keys

| Key | Action |
|---|---|
| `↑↓` / `j k` | Navigate |
| `Enter` | Connect |
| `/` | Search |
| `N` | Add host |
| `E` | Edit selected host |
| `D` | Delete (with confirmation) |
| `I` | Import / Export Ansible INI |
| `R` | Switch credential for selected host |
| `C` | Manage credentials |
| `S` | Settings |
| `Q` | Quit |

## Config

| OS | Path |
|---|---|
| Linux | `~/.config/hss/` |
| macOS | `~/Library/Application Support/hss/` |

`config.toml` — settings · `hosts.toml` — server list · `records.toml` — last-used credentials

Passwords are stored in the system keychain, not in any config file.

## Development

PRs target `main`. The release workflow on every push to `main` tags a release (`v1.0.XXXX`), builds binaries for all four platforms, and publishes a GitHub release with a changelog generated from commit messages.

To protect `main`: **Settings → Branches → Add rule** — require PR, require status checks (`ci / test`), no direct push.
