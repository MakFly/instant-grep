use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const GITHUB_API_URL: &str =
    "https://api.github.com/repos/MakFly/instant-grep/releases/latest";
const CHECK_INTERVAL_SECS: u64 = 86400; // 24h
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Spawn a background thread that checks for updates (non-blocking).
/// All errors are silently ignored — this must never break the CLI.
pub fn check_update_background() {
    std::thread::spawn(|| {
        let _ = check_update();
    });
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
            && let Ok(last_check) = timestamp_str.parse::<u64>() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs();
                if now.saturating_sub(last_check) < CHECK_INTERVAL_SECS {
                    // Still fresh — but show cached version if newer
                    if let Some(cached_version) = lines.next()
                        && is_newer(cached_version) {
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
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs();
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
        "\x1b[33m\u{1f4a1} ig v{} available (current: v{}) \u{2192} https://github.com/MakFly/instant-grep/releases\x1b[0m",
        latest, CURRENT_VERSION,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_true() {
        // CURRENT_VERSION is determined at compile time from Cargo.toml
        // Use versions guaranteed to be newer than any realistic current version
        assert!(is_newer("99.0.0"));
        assert!(is_newer("99.99.99"));
    }

    #[test]
    fn test_is_newer_false() {
        assert!(!is_newer("0.0.1"));
        assert!(!is_newer("0.1.0"));
        assert!(!is_newer(CURRENT_VERSION)); // same version is not newer
    }

    #[test]
    fn test_is_newer_invalid() {
        assert!(!is_newer("not-a-version"));
        assert!(!is_newer(""));
        assert!(!is_newer("1.0")); // only two parts — not a valid semver triple
    }

    #[test]
    fn test_cache_path_exists() {
        if std::env::var("HOME").is_ok() {
            assert!(cache_path().is_some());
            let path = cache_path().unwrap();
            assert!(path.to_string_lossy().contains(".config/ig"));
        }
    }
}
