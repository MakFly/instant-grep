//! Runtime on/off toggle for the embed-poc subcommand.
//!
//! Independent of the `embed-poc` cargo feature (which controls *compile-time*
//! inclusion). This toggle controls *runtime* execution: even when the feature
//! is built in, the user can `ig emb off` to refuse to call OpenAI from this
//! binary. Default: disabled.
//!
//! State lives in `~/.config/ig/embed.toml`:
//!
//! ```toml
//! enabled = true
//! ```

use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::PathBuf;

const FILENAME: &str = "embed.toml";

fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow!("cannot resolve $HOME"))?;
    Ok(home.join(".config/ig").join(FILENAME))
}

/// Read the current toggle. Defaults to `false` when the file is absent or
/// malformed (fail-closed: if config is broken, embeddings stay off).
pub fn is_enabled() -> bool {
    let Ok(path) = config_path() else {
        return false;
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return false;
    };
    let Ok(value) = content.parse::<toml::Value>() else {
        return false;
    };
    value
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn set(enabled: bool) -> Result<PathBuf> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let body = format!(
        "# Runtime toggle for `ig emb` — overridable with `ig emb on/off`.\n\
         enabled = {}\n",
        enabled
    );
    fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Handle `ig emb [on|off|status]`.
pub fn run(state: Option<String>) -> Result<()> {
    let arg = state.as_deref().unwrap_or("status").trim().to_lowercase();
    match arg.as_str() {
        "on" | "true" | "1" | "yes" | "y" | "enable" | "enabled" => {
            let path = set(true)?;
            println!("embed: enabled (config: {})", path.display());
            #[cfg(not(feature = "embed-poc"))]
            println!(
                "note: this binary was built without the `embed-poc` cargo feature.\n      \
                 The toggle is set, but the subcommand will only appear after a rebuild:\n      \
                   cargo build --release --features embed-poc"
            );
            Ok(())
        }
        "off" | "false" | "0" | "no" | "n" | "disable" | "disabled" => {
            let path = set(false)?;
            println!("embed: disabled (config: {})", path.display());
            Ok(())
        }
        "status" | "" => {
            let on = is_enabled();
            let path = config_path()?;
            println!(
                "embed: {} (config: {})",
                if on { "enabled" } else { "disabled" },
                path.display()
            );
            #[cfg(not(feature = "embed-poc"))]
            println!(
                "note: built without `embed-poc` cargo feature — subcommand absent regardless of this flag."
            );
            Ok(())
        }
        other => Err(anyhow!(
            "expected on/off/status, got '{}'.\n\
             Usage: ig emb [on|off|status]",
            other
        )),
    }
}

#[cfg(test)]
fn read_at(path: &std::path::Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = content.parse::<toml::Value>() else {
        return false;
    };
    value
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
fn set_at(path: &std::path::Path, enabled: bool) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = format!("enabled = {}\n", enabled);
    fs::write(path, body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_for_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("embed.toml");
        assert!(!read_at(&p));
    }

    #[test]
    fn set_on_then_off_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("embed.toml");
        set_at(&p, true).unwrap();
        assert!(read_at(&p));
        set_at(&p, false).unwrap();
        assert!(!read_at(&p));
    }

    #[test]
    fn malformed_config_falls_back_to_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("embed.toml");
        fs::write(&p, "not valid toml = = =").unwrap();
        assert!(!read_at(&p));
    }
}
