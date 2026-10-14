# hss

Terminal SSH server manager. Browse servers, manage credentials, and connect — all from the keyboard.

## Features

- **TUI** — searchable server list, keyboard-driven
- **Credential store** — passwords and SSH keys in the system keychain (macOS Keychain / Linux keyutils), never stored on disk
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
| `PgUp` / `PgDn`, `Home` / `End` | Page, first / last host |
| `Enter` | Connect |
| `/` | Search (Esc to clear, Esc again to exit) |
| `N` | Add host |
| `E` | Edit selected host |
| `D` | Delete (with confirmation) |
| `A` | Connectivity & auth audit |
| `P` | Copy public keys (`ssh-copy-id`) |
| `I` | Import / Export Ansible INI / SSH config |
| `R` | Connect as another credential for selected host |
| `M` | Start/view local MCP server |
| `C` | Manage credentials |
| `S` | Settings |
| `Q` | Quit |
| `?` / `F1` | Keyboard help (`F1` works in forms too) |
| `Ctrl+L` | Dismiss the current notification |

The table adapts to terminal width; selected-host details show the resolved SSH username, credential and jump route. Long fields scroll horizontally to keep the cursor visible. Forms support `Tab` / `Shift+Tab`, `Home` / `End`, and `Ctrl+A` / `Ctrl+E` / `Ctrl+U` editing.

- **Errors:** warnings and errors stay visible until dismissed or replaced; `F1` shows their full text. SSH exit `255` is a connection failure, not necessarily a bad password.
- **Settings:** changes remain a draft until validated and saved. Leaving a changed draft offers Save / Discard / Continue. Token reveal is limited to the focused field.
- **Delete:** only `Y` confirms; `Enter`, `N` and `Esc` cancel. Skip-confirmation choices apply separately to hosts and credentials, for the current session. Bastions with dependent hosts cannot be deleted until jump routes are reassigned.
- **Export:** existing files require explicit overwrite confirmation. Export uses a same-directory staging file and atomic installation; symlinks and non-regular destinations are refused.
- **Copy ID:** duplicate starts and editing while running are blocked. `Esc` returns to the list **without stopping**; `P` reopens progress. `Ctrl+C` in Copy ID stops its local process tree. Quitting also stops it. Installed keys cannot be undone, and an interrupted key may already be installed.
- **Host User:** used for Copy ID defaults, jump hosts and static INI/SSH-config export. Normal connections use the selected credential's username; the details panel shows that actual identity. Dynamic inventory uses that username for an explicit host credential; Automatic retains the existing User hint behavior.

### Per-host credentials

Add/Edit Host has a **Credentials** selector immediately after **Jump host**. Use `Tab` / `Shift+Tab` to reach it, `←` / `→` to cycle saved credentials (shown as name and username), and `Backspace` to reset to **Automatic**. Selection hints appear only on the focused selector; Port and Tags hints appear only when those fields are empty.

The explicit choice is stored as optional `credential_id` in `hosts.toml`, separately from the last-used credential in `servers.toml`. Existing hosts without this field remain Automatic. Resolution for normal TUI connections, direct connect, fzf, audit checks, MCP command execution and dynamic inventory is:

1. Explicit host credential.
2. Remembered last-used credential.
3. Global default credential.
4. The only saved credential.
5. Credential picker where available (TUI/fzf); otherwise unresolved.

`R` remains a deliberate **Connect as** override for that connection; it does not change the saved host selection. Even if another credential is remembered after a successful connection, the explicit host choice takes priority next time. Automatic removes the explicit choice, not the remembered credential.

A removed/missing explicit credential never silently falls back to another identity. The form shows the missing ID and refuses to save until it is replaced or reset to Automatic. Normal connections and MCP report an error; audit reports No Credential with the reason. Dynamic inventory omits that host from hostvars and groups and warns on stderr, rather than letting Ansible use an agent/default identity. Static INI/SSH-config export does not encode credential bindings.

## MCP Server (Model Context Protocol)

`hss` can serve as an MCP server for LLM agents (Claude Desktop, Zed, Cursor, OpenCode):

- **Tools exposed:** `list-servers`, `execute-command`.
- **Local credential handling:** MCP does not expose stored passwords or keys as tool fields; SSH authentication happens locally. Remote command output can still contain secrets.
- **PEM filtering:** detected private-key and certificate blocks are redacted from command output by default. Logs always redact detected PEM blocks, even when key output is enabled. This is not a general-purpose secret detector.
- **Read-Only guard:** a best-effort command filter for destructive commands (`rm`, `sudo`, `dd`, `shutdown`), **not a security sandbox**.
- **Host scoping:** restrict MCP access to specific server globs or tags (e.g. `tag:mcp`).

Stopping the TUI HTTP server blocks new requests but does **not** cancel active SSH commands. The eight-tool concurrency limit applies across stop/restart cycles. HTTP request workers are also bounded; stalled clients can occupy those slots until they disconnect (tiny_http does not expose request socket timeouts).

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

## Development

PRs target `main`. The release workflow on every push to `main` tags a release (`v1.0.XXXX`), builds binaries for all four platforms, and publishes a GitHub release with a changelog generated from commit messages.

To protect `main`: **Settings → Branches → Add rule** — require PR, require status checks (`ci / test`), no direct push.
