use anyhow::Result;
use clap_complete::{generate_to, shells};
use clap_mangen::Man;
use hss::cli::Cli;
use std::fs::{create_dir_all, File};
use std::path::Path;

fn main() -> Result<()> {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets".to_string());
    let out_path = Path::new(&out_dir);

    let comp_dir = out_path.join("completions");
    create_dir_all(&comp_dir)?;

    let mut cmd = Cli::build_cli();

    generate_to(shells::Bash, &mut cmd, "hss", &comp_dir)?;
    generate_to(shells::Zsh, &mut cmd, "hss", &comp_dir)?;
    generate_to(shells::Fish, &mut cmd, "hss", &comp_dir)?;
    generate_to(shells::PowerShell, &mut cmd, "hss", &comp_dir)?;

    let man_dir = out_path.join("man");
    create_dir_all(&man_dir)?;
    let man_file = man_dir.join("hss.1");
    let mut file = File::create(&man_file)?;
    Man::new(cmd).render(&mut file)?;

    println!(
        "Assets generated at: {} (completions in {:?}, man page in {:?})",
        out_dir, comp_dir, man_file
    );
    Ok(())
}
