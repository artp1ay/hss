use anyhow::{anyhow, bail, Context, Result};
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

pub fn resolve_repo() -> String {
    if let Ok(repo) = std::env::var("HSS_UPDATE_REPO") {
        let trimmed = repo.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    if let Ok(cfg) = crate::config::load_config() {
        let trimmed = cfg.update_repo.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }

    GITHUB_REPO.to_string()
}

pub fn resolve_update_url(repo: &str) -> String {
    if repo.starts_with("http://") || repo.starts_with("https://") {
        if repo.ends_with("/releases/latest") {
            repo.to_string()
        } else {
            format!("{}/releases/latest", repo.trim_end_matches('/'))
        }
    } else {
        format!("https://api.github.com/repos/{repo}/releases/latest")
    }
}

pub fn run() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let repo = resolve_repo();
    println!("hss v{current} — checking for updates from {repo}...");

    let url = resolve_update_url(&repo);

    let release = fetch_release(&url, current)?;

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
                 Download manually: https://github.com/{repo}/releases"
            )
        })?;

    let sums_asset = release
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS")
        .ok_or_else(|| anyhow!("Missing SHA256SUMS in release {tag} — update aborted"))?;

    let exe_path = std::env::current_exe()?.canonicalize()?;
    let tmp_file_name = format!(".hss.update.{}.tmp", uuid::Uuid::new_v4());
    let tmp_path = exe_path.with_file_name(tmp_file_name);

    let result = (|| -> Result<()> {
        download_and_verify(asset, sums_asset, &tmp_path, current)?;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp_path, &exe_path)
            .map_err(|e| anyhow!("Cannot replace binary: {e}\nHint: try 'sudo hss --update'"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    result?;

    println!("Updated to {tag}. Restart hss to use the new version.");
    Ok(())
}

fn fetch_release(url: &str, current: &str) -> Result<Release> {
    ureq::get(url)
        .set("User-Agent", &format!("hss/{current}"))
        .call()
        .context("Cannot fetch published release from update API")?
        .into_json()
        .context("Unexpected API response")
}

fn download_and_verify(
    asset: &Asset,
    sums_asset: &Asset,
    tmp_path: &std::path::Path,
    current: &str,
) -> Result<()> {
    let sums_text = ureq::get(&sums_asset.browser_download_url)
        .set("User-Agent", &format!("hss/{current}"))
        .call()
        .context("Cannot download SHA256SUMS — update aborted")?
        .into_string()
        .context("Cannot read SHA256SUMS — update aborted")?;
    let expected_hash = parse_checksum_for_asset(&sums_text, &asset.name).ok_or_else(|| {
        anyhow!(
            "Missing, invalid or duplicate SHA256SUMS entry for {}",
            asset.name
        )
    })?;

    println!("Downloading {}...", asset.name);
    let response = ureq::get(&asset.browser_download_url)
        .set("User-Agent", &format!("hss/{current}"))
        .call()
        .context("Binary download failed — update aborted")?;

    let bytes = {
        let mut reader = response.into_reader();
        let mut tmp = std::fs::File::create(tmp_path).map_err(|e| {
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
        bail!("Downloaded file is empty — update aborted");
    }

    let actual_hash =
        compute_file_sha256(tmp_path).context("Cannot verify SHA256 checksum — update aborted")?;
    if !expected_hash.eq_ignore_ascii_case(&actual_hash) {
        bail!(
            "SHA256 checksum mismatch!\nExpected: {expected_hash}\nActual:   {actual_hash}\nUpdate aborted for security."
        );
    }
    println!("Checksum OK ({actual_hash})");
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
        bail!("Unsupported platform")
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
    let mut expected = None;
    for line in sums_text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(filename) = parts.next() else {
            continue;
        };
        let filename = filename.strip_prefix('*').unwrap_or(filename);
        let filename = filename.strip_prefix("./").unwrap_or(filename);
        if filename == asset_name {
            if expected.is_some()
                || parts.next().is_some()
                || hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return None;
            }
            expected = Some(hash.to_string());
        }
    }
    expected
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
    fn resolve_repo_fallback_or_custom() {
        assert!(!resolve_repo().is_empty());
    }

    #[test]
    fn update_url_github_and_custom() {
        assert_eq!(
            resolve_update_url("artp1ay/hss"),
            "https://api.github.com/repos/artp1ay/hss/releases/latest"
        );
        assert_eq!(
            resolve_update_url("https://git.freakware.ru/api/v1/repos/max/hss"),
            "https://git.freakware.ru/api/v1/repos/max/hss/releases/latest"
        );
        assert_eq!(
            resolve_update_url("https://git.freakware.ru/api/v1/repos/max/hss/releases/latest"),
            "https://git.freakware.ru/api/v1/repos/max/hss/releases/latest"
        );
    }

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

    fn serve_responses(responses: Vec<(u16, String)>) -> (String, std::thread::JoinHandle<()>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let thread = std::thread::spawn(move || {
            for (status, body) in responses {
                let request = server
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .expect("expected update HTTP request");
                request
                    .respond(tiny_http::Response::from_string(body).with_status_code(status))
                    .unwrap();
            }
        });
        (url, thread)
    }

    fn check_download(responses: Vec<(u16, String)>) -> Result<()> {
        let (url, server) = serve_responses(responses);
        let asset = Asset {
            name: "hss-test".into(),
            browser_download_url: format!("{url}/binary"),
        };
        let sums = Asset {
            name: "SHA256SUMS".into(),
            browser_download_url: format!("{url}/SHA256SUMS"),
        };
        let path = std::env::temp_dir().join(format!("hss-update-test-{}", uuid::Uuid::new_v4()));
        let result = download_and_verify(&asset, &sums, &path, "1.0.0");
        let _ = std::fs::remove_file(&path);
        server.join().unwrap();
        result
    }

    #[test]
    fn checksum_parser_rejects_invalid_or_ambiguous_entries() {
        let hash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        for name in ["hss-test", "*hss-test", "./hss-test", "*./hss-test"] {
            assert_eq!(
                parse_checksum_for_asset(
                    &format!("{}  {name}\r\n", hash.to_uppercase()),
                    "hss-test"
                ),
                Some(hash.to_uppercase())
            );
        }
        for text in [
            String::new(),
            format!("{hash}  other/hss-test"),
            format!("{hash}  ../hss-test"),
            format!("{hash}  **hss-test"),
            format!("{hash}  hss-test extra"),
            format!("{}  hss-test", "g".repeat(64)),
            format!("{}  hss-test", &hash[..63]),
            format!("{hash}0  hss-test"),
            format!("{hash}  hss-test\n{hash}  *./hss-test"),
            format!("invalid  hss-test\n{hash}  hss-test"),
        ] {
            assert_eq!(parse_checksum_for_asset(&text, "hss-test"), None, "{text}");
        }
    }

    #[test]
    fn release_api_errors_are_not_success() {
        for (status, body) in [(404, "Not Found"), (200, "not JSON")] {
            let (url, server) = serve_responses(vec![(status, body.into())]);
            assert!(fetch_release(&url, "1.0.0").is_err());
            server.join().unwrap();
        }
    }

    #[test]
    fn download_requires_valid_checksum_and_http_success() {
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let sums = format!("{hash}  hss-test");
        check_download(vec![(200, sums.clone()), (200, "abc".into())]).unwrap();

        for (responses, message) in [
            (
                vec![(404, "Not Found".into())],
                "Cannot download SHA256SUMS",
            ),
            (
                vec![(200, "invalid  hss-test".into())],
                "invalid or duplicate",
            ),
            (vec![(200, format!("{hash}  other"))], "Missing"),
            (
                vec![(200, sums.clone()), (404, "Not Found".into())],
                "Binary download failed",
            ),
            (
                vec![(200, sums.clone()), (200, String::new())],
                "Downloaded file is empty",
            ),
            (
                vec![(200, sums), (200, "tampered".into())],
                "SHA256 checksum mismatch",
            ),
        ] {
            let error = check_download(responses).unwrap_err();
            assert!(error.to_string().contains(message), "{error:#}");
        }
    }

    #[test]
    fn checksum_read_failure_is_an_error() {
        let path = std::env::temp_dir().join(format!("hss-missing-{}", uuid::Uuid::new_v4()));
        assert!(compute_file_sha256(&path).is_err());
    }

    #[test]
    fn newer_build_detected() {
        assert!(is_newer("1.0.0002", "1.0.0001"));
        assert!(is_newer("1.0.0010", "1.0.0009"));
        assert!(!is_newer("1.0.0001", "1.0.0001"));
        assert!(!is_newer("1.0.0001", "1.0.0002"));
    }
}
