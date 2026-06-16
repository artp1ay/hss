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
