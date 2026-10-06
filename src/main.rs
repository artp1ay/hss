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
        if let Some(hostvars) = inv.get("_meta").and_then(|m| m.get("hostvars")).and_then(|hv| hv.get(target)) {
            println!("{}", serde_json::to_string_pretty(hostvars)?);
        } else {
            println!("{{}}");
        }
        return Ok(());
    }

    match (cli.update, cli.fzf, cli.direct_host) {
        (true, _, _) => hss::update::run(),
        (_, true, _) => hss::fzf::run(),
        (_, _, Some(host)) => hss::ssh::connect_direct(&host),
        _ => hss::tui::run(),
    }
}
