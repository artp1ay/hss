use anyhow::{anyhow, bail, Result};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;

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
    let tmp_file_name = format!(".hss.update.{}.tmp", uuid::Uuid::new_v4());
    let tmp_path = exe_path.with_file_name(tmp_file_name);

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

    if bytes == 0 {
        let _ = std::fs::remove_file(&tmp_path);
        bail!("Downloaded file is empty — update aborted");
    }

    // Attempt to verify SHA256 if SHA256SUMS is available in the release assets
    if let Some(sums_asset) = release.assets.iter().find(|a| a.name == "SHA256SUMS") {
        print!("Verifying checksum from {}...", sums_asset.name);
        if let Ok(sums_resp) = ureq::get(&sums_asset.browser_download_url)
            .set("User-Agent", &format!("hss/{current}"))
            .call()
        {
            if let Ok(sums_text) = sums_resp.into_string() {
                if let Some(expected_hash) = parse_checksum_for_asset(&sums_text, asset_name) {
                    if let Ok(actual_hash) = compute_file_sha256(&tmp_path) {
                        if expected_hash.to_lowercase() != actual_hash.to_lowercase() {
                            let _ = std::fs::remove_file(&tmp_path);
                            bail!(
                                "SHA256 checksum mismatch!\nExpected: {expected_hash}\nActual:   {actual_hash}\nUpdate aborted for security."
                            );
                        }
                        println!(" OK ({actual_hash})");
                    }
                }
            }
        }
    }

    std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
    std::fs::rename(&tmp_path, &exe_path)
        .map_err(|e| anyhow!("Cannot replace binary: {e}\nHint: try 'sudo hss --update'"))?;

    println!("Updated to {tag}. Restart hss to use the new version.");
    Ok(())
}

fn platform_asset() -> Result<&'static str> {
    Ok(if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "hss-linux-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "hss-linux-aarch64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "hss-macos-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "hss-macos-aarch64"
    } else {
        bail!("Unsupported platform — download from: https://github.com/{GITHUB_REPO}/releases")
    })
}

fn clean_version(s: &str) -> &str {
    s.trim_start_matches('v')
}

// Returns true if `latest` version string is higher than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    if let (Ok(l), Ok(c)) = (
        semver::Version::parse(clean_version(latest)),
        semver::Version::parse(clean_version(current)),
    ) {
        l > c
    } else {
        // Fallback for simple numeric dot components if non-standard semver
        let parse = |v: &str| -> Vec<u64> {
            clean_version(v)
                .split('.')
                .map(|n| n.parse().unwrap_or(0))
                .collect()
        };
        parse(latest) > parse(current)
    }
}

fn parse_checksum_for_asset(sums_text: &str, asset_name: &str) -> Option<String> {
    for line in sums_text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0];
            let filename = parts[1].trim_start_matches('*');
            if filename == asset_name || filename.ends_with(&format!("/{asset_name}")) {
                return Some(hash.to_string());
            }
        }
    }
    None
}

fn compute_file_sha256(path: &std::path::Path) -> Result<String> {
    // Try system utilities first (sha256sum on Linux, shasum on macOS)
    if let Ok(output) = std::process::Command::new("sha256sum").arg(path).output() {
        if output.status.success() {
            if let Some(hash) = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
            {
                return Ok(hash.to_string());
            }
        }
    }
    if let Ok(output) = std::process::Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
    {
        if output.status.success() {
            if let Some(hash) = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
            {
                return Ok(hash.to_string());
            }
        }
    }
    bail!("No system sha256 utility available (sha256sum or shasum)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_checksum_table() {
        let text = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  hss-linux-x86_64\n\
                    01ba4719c80b6fe911b091a7c05124b64eeece964e09c058ef8f9805daca546b  hss-macos-aarch64";
        assert_eq!(
            parse_checksum_for_asset(text, "hss-linux-x86_64"),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
        );
        assert_eq!(
            parse_checksum_for_asset(text, "hss-macos-aarch64"),
            Some("01ba4719c80b6fe911b091a7c05124b64eeece964e09c058ef8f9805daca546b".into())
        );
        assert_eq!(parse_checksum_for_asset(text, "hss-windows"), None);
    }

    #[test]
    fn newer_build_detected() {
        assert!(is_newer("1.0.0002", "1.0.0001"));
        assert!(is_newer("1.0.0010", "1.0.0009"));
        assert!(!is_newer("1.0.0001", "1.0.0001"));
        assert!(!is_newer("1.0.0001", "1.0.0002"));
    }
}
