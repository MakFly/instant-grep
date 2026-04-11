#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::Deserialize;

/// Global ig configuration, loaded from ~/.config/ig/config.toml.
#[derive(Deserialize, Default)]
pub struct IgConfig {
    #[serde(default)]
    pub tracking: TrackingConfig,
    #[serde(default)]
    pub filters: FilterConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
}

#[derive(Deserialize)]
pub struct TrackingConfig {
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

impl Default for TrackingConfig {
    fn default() -> Self {
        Self {
            retention_days: default_retention_days(),
        }
    }
}

fn default_retention_days() -> u32 {
    90
}

#[derive(Deserialize, Default)]
pub struct FilterConfig {
    /// Override the user filter directory (default: ~/.config/ig/filters/)
    pub user_dir: Option<PathBuf>,
}

#[derive(Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_grep_max")]
    pub grep_max_results: usize,
    #[serde(default = "default_head")]
    pub head_default: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            grep_max_results: default_grep_max(),
            head_default: default_head(),
        }
    }
}

fn default_grep_max() -> usize {
    1000
}

fn default_head() -> usize {
    250
}

static CONFIG: OnceLock<IgConfig> = OnceLock::new();

/// Get the global config singleton. Loads from disk on first call.
pub fn config() -> &'static IgConfig {
    CONFIG.get_or_init(load_config)
}

/// Load config from ~/.config/ig/config.toml, falling back to defaults.
fn load_config() -> IgConfig {
    let Some(config_dir) = dirs::config_dir() else {
        return IgConfig::default();
    };

    let path = config_dir.join("ig").join("config.toml");
    if !path.exists() {
        return IgConfig::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("ig: warn: failed to parse {}: {}", path.display(), e);
                IgConfig::default()
            }
        },
        Err(e) => {
            eprintln!("ig: warn: failed to read {}: {}", path.display(), e);
            IgConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults() {
        let cfg = IgConfig::default();
        assert_eq!(cfg.tracking.retention_days, 90);
        assert_eq!(cfg.limits.grep_max_results, 1000);
        assert_eq!(cfg.limits.head_default, 250);
        assert!(cfg.filters.user_dir.is_none());
    }

    #[test]
    fn test_config_accessor_does_not_panic() {
        let cfg = config();
        assert!(cfg.tracking.retention_days > 0);
    }

    #[test]
    fn test_deserialize_partial_config() {
        let toml = r#"
[tracking]
retention_days = 30
"#;
        let cfg: IgConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tracking.retention_days, 30);
        // Other fields use defaults
        assert_eq!(cfg.limits.grep_max_results, 1000);
    }

    #[test]
    fn test_deserialize_full_config() {
        let toml = r#"
[tracking]
retention_days = 60

[filters]
user_dir = "/custom/filters"

[limits]
grep_max_results = 500
head_default = 100
"#;
        let cfg: IgConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.tracking.retention_days, 60);
        assert_eq!(
            cfg.filters.user_dir.as_deref(),
            Some(std::path::Path::new("/custom/filters"))
        );
        assert_eq!(cfg.limits.grep_max_results, 500);
        assert_eq!(cfg.limits.head_default, 100);
    }
}
