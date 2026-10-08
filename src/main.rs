use clap::Parser;

#[derive(Parser)]
#[command(name = "hss", about = "SSH manager — connect to your servers", version)]
struct Cli {
    /// Quick fzf picker mode
    #[arg(long)]
    fzf: bool,
    /// Check for a newer release and replace this binary automatically
    #[arg(long)]
    update: bool,
    /// Output dynamic Ansible inventory JSON
    #[arg(long, alias = "list")]
    ansible_inventory: bool,
    /// Ansible inventory host query (accepted for compatibility with Ansible inventory scripts)
    #[arg(long)]
    host: Option<String>,
    /// Connect directly to host by name or IP
    direct_host: Option<String>,
    /// Run headless MCP server directly in terminal without TUI
    #[arg(long)]
    mcp: bool,
    /// Run MCP server over stdio for agents (Zed, Claude Desktop, Cursor)
    #[arg(long)]
    mcp_stdio: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.ansible_inventory {
        let hosts = hss::config::load_hosts()?;
        let creds = hss::config::load_credentials()?;
        let records = hss::config::load_server_records()?;
        let cfg = hss::config::load_config()?;
        let inv = hss::inventory::generate_ansible_inventory(&hosts, &creds, &records, &cfg);
        println!("{}", serde_json::to_string_pretty(&inv)?);
        return Ok(());
    }
    if let Some(ref target) = cli.host {
        let hosts = hss::config::load_hosts()?;
        let creds = hss::config::load_credentials()?;
        let records = hss::config::load_server_records()?;
        let cfg = hss::config::load_config()?;
        let inv = hss::inventory::generate_ansible_inventory(&hosts, &creds, &records, &cfg);
        if let Some(hostvars) = inv
            .get("_meta")
            .and_then(|m| m.get("hostvars"))
            .and_then(|hv| hv.get(target))
        {
            println!("{}", serde_json::to_string_pretty(hostvars)?);
        } else {
            println!("{{}}");
        }
        return Ok(());
    }

    if cli.mcp_stdio {
        return hss::mcp::run_stdio();
    }

    if cli.mcp {
        println!("Starting hss MCP server in standalone mode...");
        let server = hss::mcp::McpServer::start()?;
        println!("hss MCP server listening on {}", server.server_url());
        println!("Press Ctrl+C to stop.");

        let running = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let r = running.clone();

        unsafe {
            // Signal handler flag
            static RUNNING_FLAG: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(true);
            extern "C" fn handle_sigint(_: libc::c_int) {
                RUNNING_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
            }
            libc::signal(
                libc::SIGINT,
                handle_sigint as *const () as libc::sighandler_t,
            );
            while RUNNING_FLAG.load(std::sync::atomic::Ordering::SeqCst)
                && r.load(std::sync::atomic::Ordering::SeqCst)
            {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }

        println!("\nStopping MCP server...");
        server.stop();
        println!("MCP server stopped.");
        return Ok(());
    }

    match (cli.update, cli.fzf, cli.direct_host) {
        (true, _, _) => hss::update::run(),
        (_, true, _) => hss::fzf::run(),
        (_, _, Some(host)) => hss::ssh::connect_direct(&host),
        _ => hss::tui::run(),
    }
}
