use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

const GITHUB_API_URL: &str = "https://api.github.com/repos/MakFly/instant-grep/releases/latest";
const CHECK_INTERVAL_SECS: u64 = 86400; // 24h
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Spawn a background thread that checks for updates (non-blocking).
/// All errors are silently ignored — this must never break the CLI.
pub fn check_update_background() {
    std::thread::spawn(|| {
        let _ = check_update();
    });
}

/// Interactive self-update with progress bar.
pub fn run_update() -> Result<()> {
    eprint!("  Checking latest version... ");
    let response: serde_json::Value = ureq::get(GITHUB_API_URL)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .context("failed to reach GitHub API")?
        .body_mut()
        .read_json()
        .context("failed to parse release info")?;

    let tag = response
        .get("tag_name")
        .and_then(|t| t.as_str())
        .context("no tag_name in release")?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    if !is_newer(latest) {
        eprintln!("✓");
        eprintln!("\n  Already up to date (v{}).", CURRENT_VERSION);
        return Ok(());
    }

    eprintln!("v{} → v{}", CURRENT_VERSION, latest);

    // Detect platform
    let artifact = detect_artifact()?;
    let url = format!(
        "https://github.com/MakFly/instant-grep/releases/download/{}/{}",
        tag, artifact
    );

    // Find current binary path
    let bin_path = std::env::current_exe().context("cannot determine binary path")?;
    let bin_path = bin_path.canonicalize().unwrap_or_else(|_| bin_path.clone());

    // Download binary
    let tmp_path = bin_path.with_extension("tmp");

    eprint!("  Downloading {}...", artifact);
    io::stderr().flush().ok();

    let bytes = ureq::get(&url)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .context("download failed")?
        .body_mut()
        .read_to_vec()
        .context("failed to read response body")?;

    let size_mb = bytes.len() as f64 / 1_048_576.0;
    eprint!(
        "\r  Downloading {}... [{}] {:.1} MB ✓\n",
        artifact,
        "█".repeat(20),
        size_mb
    );

    fs::write(&tmp_path, &bytes).context("cannot write temp file")?;

    // Set executable permission
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o755))?;
    }

    // Atomic replace: rename tmp over current binary
    eprint!("  Installing... ");
    fs::rename(&tmp_path, &bin_path)
        .or_else(|_| {
            // rename fails across filesystems — fallback to copy
            fs::copy(&tmp_path, &bin_path)?;
            fs::remove_file(&tmp_path)?;
            Ok::<_, io::Error>(())
        })
        .context("failed to replace binary")?;

    eprintln!("✓");
    eprintln!("\n  Updated: v{} → v{}", CURRENT_VERSION, latest);
    eprintln!("  Path: {}", bin_path.display());

    // Update cache so background check doesn't re-notify
    if let Some(cache) = cache_path() {
        if let Some(dir) = cache.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(mut f) = fs::File::create(&cache) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let _ = writeln!(f, "{}", now);
            let _ = writeln!(f, "{}", latest);
        }
    }

    Ok(())
}

fn detect_artifact() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let name = match (os, arch) {
        ("macos", "aarch64") => "ig-macos-aarch64",
        ("macos", "x86_64") => "ig-macos-x86_64",
        ("linux", "x86_64") => "ig-linux-x86_64",
        ("linux", "aarch64") => "ig-linux-aarch64",
        _ => anyhow::bail!("unsupported platform: {}-{}", os, arch),
    };
    Ok(name.to_string())
}

fn cache_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("ig"));
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config").join("ig"))
}

fn cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("last_update_check"))
}

fn check_update() -> Option<()> {
    let cache = cache_path()?;

    // Check if we already checked recently
    if let Ok(contents) = fs::read_to_string(&cache) {
        let mut lines = contents.lines();
        if let Some(timestamp_str) = lines.next()
            && let Ok(last_check) = timestamp_str.parse::<u64>()
        {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
            if now.saturating_sub(last_check) < CHECK_INTERVAL_SECS {
                // Still fresh — but show cached version if newer
                if let Some(cached_version) = lines.next()
                    && is_newer(cached_version)
                {
                    print_update_message(cached_version);
                }
                return Some(());
            }
        }
    }

    // Fetch latest release from GitHub
    let response: serde_json::Value = ureq::get(GITHUB_API_URL)
        .header("User-Agent", &format!("ig/{}", CURRENT_VERSION))
        .call()
        .ok()?
        .body_mut()
        .read_json()
        .ok()?;

    let tag = response.get("tag_name")?.as_str()?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    // Update cache
    if let Some(dir) = cache_dir() {
        let _ = fs::create_dir_all(&dir);
        if let Ok(mut f) = fs::File::create(&cache) {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
            let _ = writeln!(f, "{}", now);
            let _ = writeln!(f, "{}", latest);
        }
    }

    if is_newer(latest) {
        print_update_message(latest);
    }

    Some(())
}

fn is_newer(latest: &str) -> bool {
    let parse = |v: &str| -> Option<(u64, u64, u64)> {
        let mut parts = v.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        Some((major, minor, patch))
    };

    match (parse(CURRENT_VERSION), parse(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

fn print_update_message(latest: &str) {
    eprintln!(
        "\x1b[33mig v{} available (current: v{}). Run `ig update` to upgrade.\x1b[0m",
        latest, CURRENT_VERSION,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_true() {
        assert!(is_newer("99.0.0"));
        assert!(is_newer("99.99.99"));
    }

    #[test]
    fn test_is_newer_false() {
        assert!(!is_newer("0.0.1"));
        assert!(!is_newer("0.1.0"));
        assert!(!is_newer(CURRENT_VERSION));
    }

    #[test]
    fn test_is_newer_invalid() {
        assert!(!is_newer("not-a-version"));
        assert!(!is_newer(""));
        assert!(!is_newer("1.0"));
    }

    #[test]
    fn test_cache_path_exists() {
        if std::env::var("HOME").is_ok() {
            assert!(cache_path().is_some());
            let path = cache_path().unwrap();
            assert!(path.to_string_lossy().contains(".config/ig"));
        }
    }

    #[test]
    fn test_detect_artifact() {
        let artifact = detect_artifact().unwrap();
        assert!(artifact.starts_with("ig-"));
    }
}
