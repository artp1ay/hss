use anyhow::{anyhow, bail, Result};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;

// Replace OWNER with your GitHub username after creating the repo.
const GITHUB_REPO: &str = "artp1ay/hss";

#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub fn run() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    println!("hss v{current} — checking for updates...");

    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/releases/latest");
    let release: Release = ureq::get(&url)
        .set("User-Agent", &format!("hss/{current}"))
        .call()
        .map_err(|e| anyhow!("Cannot reach GitHub API: {e}"))?
        .into_json()
        .map_err(|e| anyhow!("Unexpected API response: {e}"))?;

    let tag = &release.tag_name;
    let latest = tag.trim_start_matches('v');

    if !is_newer(latest, current) {
        println!("Already up to date (v{current}).");
        return Ok(());
    }

    println!("Update available: v{current} → {tag}");

    let asset_name = platform_asset()?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == asset_name)
        .ok_or_else(|| {
            anyhow!(
                "No binary for '{asset_name}' in release {tag}.\n\
                 Download manually: https://github.com/{GITHUB_REPO}/releases"
            )
        })?;

    println!("Downloading {}...", asset.name);

    let response = ureq::get(&asset.browser_download_url)
        .set("User-Agent", &format!("hss/{current}"))
        .call()
        .map_err(|e| anyhow!("Download failed: {e}"))?;

    let exe_path = std::env::current_exe()?.canonicalize()?;
    let tmp_path = exe_path.with_file_name(".hss.update.tmp");

    let bytes = {
        let mut reader = response.into_reader();
        let mut tmp = std::fs::File::create(&tmp_path).map_err(|e| {
            anyhow!(
                "Cannot write to {}: {e}\nHint: try 'sudo hss --update'",
                tmp_path.display()
            )
        })?;
        let mut buf = [0u8; 65536];
        let mut total = 0u64;
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            tmp.write_all(&buf[..n])?;
            total += n as u64;
            eprint!("\r  {:.1} MB", total as f64 / 1_048_576.0);
        }
        eprintln!();
        tmp.flush()?;
        total
    };

    println!("  {:.1} MB downloaded", bytes as f64 / 1_048_576.0);

    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&tmp_path, &exe_path).map_err(|e| {
        anyhow!(
            "Cannot replace binary: {e}\nHint: try 'sudo hss --update'"
        )
    })?;

    println!("Updated to {tag}. Restart hss to use the new version.");
    Ok(())
}

fn platform_asset() -> Result<&'static str> {
    Ok(
        if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            "hss-linux-x86_64"
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            "hss-linux-aarch64"
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            "hss-macos-x86_64"
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "hss-macos-aarch64"
        } else {
            bail!(
                "Unsupported platform — download from: https://github.com/{GITHUB_REPO}/releases"
            )
        },
    )
}

// Returns true if `latest` version string is higher than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.').map(|n| n.parse().unwrap_or(0)).collect()
    };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn newer_build_detected() {
        assert!(is_newer("1.0.0002", "1.0.0001"));
        assert!(is_newer("1.0.0010", "1.0.0009"));
        assert!(!is_newer("1.0.0001", "1.0.0001"));
        assert!(!is_newer("1.0.0001", "1.0.0002"));
    }
}
