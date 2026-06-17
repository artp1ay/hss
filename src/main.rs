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
    /// Connect directly to host by name or IP
    host: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match (cli.update, cli.fzf, cli.host) {
        (true, _, _) => hss::update::run(),
        (_, true, _) => hss::fzf::run(),
        (_, _, Some(host)) => hss::ssh::connect_direct(&host),
        _ => hss::tui::run(),
    }
}
