//! Lookup order: $OPENAI_API_KEY env var > .env at project root > ~/.config/ig/config.toml.

use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::Path;

pub struct PocConfig {
    pub openai_api_key: String,
    pub embed_model: String,
}

const DEFAULT_MODEL: &str = "text-embedding-3-small";

pub fn load() -> Result<PocConfig> {
    if let Ok(key) = std::env::var("OPENAI_API_KEY")
        && !key.is_empty()
        && key != "sk-proj-REPLACE_ME"
    {
        let model = std::env::var("OPENAI_EMBED_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        return Ok(PocConfig {
            openai_api_key: key,
            embed_model: model,
        });
    }

    if let Some(cfg) = load_from_dotenv(".env")? {
        return Ok(cfg);
    }

    if let Some(home) = dirs::home_dir() {
        let path = home.join(".config/ig/config.toml");
        if let Some(cfg) = load_from_toml(&path)? {
            return Ok(cfg);
        }
    }

    Err(anyhow!(
        "no OpenAI API key found. Set OPENAI_API_KEY, or fill .env at project root, or ~/.config/ig/config.toml"
    ))
}

fn load_from_dotenv(path: &str) -> Result<Option<PocConfig>> {
    let p = Path::new(path);
    if !p.exists() {
        return Ok(None);
    }
    let mut key: Option<String> = None;
    let mut model: Option<String> = None;
    let content = fs::read_to_string(p).with_context(|| format!("read {}", path))?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        match k.trim() {
            "OPENAI_API_KEY" => {
                if !v.is_empty() && v != "sk-proj-REPLACE_ME" {
                    key = Some(v.to_string());
                }
            }
            "OPENAI_EMBED_MODEL" => {
                if !v.is_empty() {
                    model = Some(v.to_string());
                }
            }
            _ => {}
        }
    }
    match key {
        Some(k) => Ok(Some(PocConfig {
            openai_api_key: k,
            embed_model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        })),
        None => Ok(None),
    }
}

fn load_from_toml(path: &Path) -> Result<Option<PocConfig>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let value: toml::Value = toml::from_str(&content).with_context(|| "parse config.toml")?;
    let providers = value.get("providers").and_then(|v| v.get("openai"));
    let key = providers
        .and_then(|v| v.get("api_key"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "sk-proj-REPLACE_ME")
        .map(|s| s.to_string());
    let model = providers
        .and_then(|v| v.get("default_model"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    match key {
        Some(k) => Ok(Some(PocConfig {
            openai_api_key: k,
            embed_model: model,
        })),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn dotenv_parses_key_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f, "OPENAI_API_KEY=sk-proj-realkey").unwrap();
        writeln!(f, "OPENAI_EMBED_MODEL=\"text-embedding-3-large\"").unwrap();
        let cfg = load_from_dotenv(path.to_str().unwrap())
            .unwrap()
            .expect("must parse");
        assert_eq!(cfg.openai_api_key, "sk-proj-realkey");
        assert_eq!(cfg.embed_model, "text-embedding-3-large");
    }

    #[test]
    fn dotenv_rejects_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        fs::write(&path, "OPENAI_API_KEY=sk-proj-REPLACE_ME").unwrap();
        let cfg = load_from_dotenv(path.to_str().unwrap()).unwrap();
        assert!(cfg.is_none());
    }
}
