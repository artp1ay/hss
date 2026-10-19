use clap::{CommandFactory, Parser};

#[derive(Parser, Debug)]
#[command(name = "hss", about = "SSH manager — connect to your servers", version)]
pub struct Cli {
    /// Quick fzf picker mode
    #[arg(long)]
    pub fzf: bool,
    /// Check for a newer release and replace this binary automatically
    #[arg(long)]
    pub update: bool,
    /// Output dynamic Ansible inventory JSON
    #[arg(long, alias = "list")]
    pub ansible_inventory: bool,
    /// Ansible inventory host query (accepted for compatibility with Ansible inventory scripts)
    #[arg(long)]
    pub host: Option<String>,
    /// Connect directly to host by name or IP
    pub direct_host: Option<String>,
    /// Run headless MCP server directly in terminal without TUI
    #[arg(long)]
    pub mcp: bool,
    /// Run MCP server over stdio for agents (Zed, Claude Desktop, Cursor)
    #[arg(long)]
    pub mcp_stdio: bool,
}

impl Cli {
    pub fn build_cli() -> clap::Command {
        <Self as CommandFactory>::command()
    }
}
