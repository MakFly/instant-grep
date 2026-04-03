use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct BrainConfig {
    pub token: String,
    pub api_url: String,
    pub auto_sync: bool,
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("brain")
        .join("config.json")
}

pub fn load_config() -> Option<BrainConfig> {
    let path = config_path();
    let content = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

fn save_config(config: &BrainConfig) -> Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("creating config directory")?;
    }
    let json = serde_json::to_string_pretty(config)?;
    fs::write(&path, format!("{}\n", json)).context("writing config")?;
    Ok(())
}

pub fn brain_login() -> Result<()> {
    eprintln!();
    eprintln!("  brain.dev — API Token Setup");
    eprintln!();
    eprintln!("  1. Go to https://brain.dev/settings/tokens");
    eprintln!("  2. Create a new token");
    eprintln!("  3. Paste it here:");
    eprintln!();
    eprint!("  Token: ");
    io::stderr().flush().ok();

    let mut token = String::new();
    io::stdin()
        .lock()
        .read_line(&mut token)
        .context("reading token from stdin")?;
    let token = token.trim().to_string();

    if token.is_empty() {
        anyhow::bail!("no token provided");
    }

    if !token.starts_with("brn_") {
        eprintln!("  Warning: token does not start with 'brn_' — are you sure it's correct?");
    }

    let config = BrainConfig {
        token,
        api_url: "https://brain.dev/api/v1".to_string(),
        auto_sync: true,
    };
    save_config(&config)?;

    eprintln!();
    eprintln!("  \u{2713} Connected to brain.dev");
    eprintln!();
    Ok(())
}

pub fn brain_sync() -> Result<()> {
    let config = load_config().context("not logged in — run `ig brain login` first")?;

    eprintln!("  Syncing to brain.dev...");

    // Sync memories from MEMORY.md
    let memory_path = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".claude")
        .join("MEMORY.md");

    let mut memory_count = 0u32;
    if memory_path.exists() {
        let content = fs::read_to_string(&memory_path).unwrap_or_default();
        let memories: Vec<&str> = content
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect();
        memory_count = memories.len() as u32;

        if !memories.is_empty() {
            let payload = serde_json::json!({
                "memories": memories.iter().map(|m| {
                    serde_json::json!({"content": m, "source": "claude-memory"})
                }).collect::<Vec<_>>(),
            });

            let _response = ureq::post(&format!("{}/brain/memories/bulk", config.api_url))
                .header("Authorization", &format!("Bearer {}", config.token))
                .header("Content-Type", "application/json")
                .send_json(&payload);
        }
    }

    // Sync tracking stats from history.jsonl
    let history_path = tracking_history_path();
    let mut total_commands = 0u64;
    let mut total_saved: u64 = 0;

    if history_path.exists() {
        if let Ok(content) = fs::read_to_string(&history_path) {
            for line in content.lines() {
                if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                    total_commands += 1;
                    let original = entry
                        .get("original_bytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let output = entry
                        .get("output_bytes")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    total_saved += original.saturating_sub(output);
                }
            }
        }

        if total_commands > 0 {
            let payload = serde_json::json!({
                "commands": total_commands,
                "bytes_saved": total_saved,
                "source": "ig-tracking",
            });

            let _response = ureq::post(&format!("{}/brain/stats", config.api_url))
                .header("Authorization", &format!("Bearer {}", config.token))
                .header("Content-Type", "application/json")
                .send_json(&payload);
        }
    }

    eprintln!("    Memories: {} synced", memory_count);
    eprintln!(
        "    Stats: {} commands, {:.1}MB saved",
        format_number(total_commands),
        total_saved as f64 / 1_048_576.0
    );
    eprintln!("  \u{2713} Sync complete");
    Ok(())
}

pub fn brain_pull() -> Result<()> {
    let config = load_config().context("not logged in — run `ig brain login` first")?;

    let response: serde_json::Value = ureq::get(&format!("{}/brain/skills", config.api_url))
        .header("Authorization", &format!("Bearer {}", config.token))
        .call()
        .context("failed to fetch skills")?
        .body_mut()
        .read_json()
        .context("failed to parse skills response")?;

    let skills = response
        .get("data")
        .and_then(|d| d.get("skills"))
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();

    let skills_dir = PathBuf::from(".brain").join("skills");
    fs::create_dir_all(&skills_dir).context("creating .brain/skills directory")?;

    for skill in &skills {
        let name = skill
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("unknown");
        let content = skill.get("content").and_then(|c| c.as_str()).unwrap_or("");
        let filename = format!("{}.md", name);
        fs::write(skills_dir.join(&filename), content)
            .with_context(|| format!("writing skill {}", filename))?;
    }

    eprintln!("  \u{2713} Pulled {} skills", skills.len());
    Ok(())
}

pub fn brain_status() -> Result<()> {
    let config = load_config().context("not logged in — run `ig brain login` first")?;

    let response: serde_json::Value = ureq::get(&format!("{}/auth/me", config.api_url))
        .header("Authorization", &format!("Bearer {}", config.token))
        .call()
        .context("failed to reach brain.dev — check your connection")?
        .body_mut()
        .read_json()
        .context("failed to parse response")?;

    let data = response.get("data").unwrap_or(&response);
    let email = data
        .get("email")
        .and_then(|e| e.as_str())
        .unwrap_or("unknown");
    let plan = data.get("plan").and_then(|p| p.as_str()).unwrap_or("free");
    let last_sync = data
        .get("last_sync")
        .and_then(|l| l.as_str())
        .unwrap_or("never");

    eprintln!();
    eprintln!("  brain.dev — Connected");
    eprintln!("    User: {}", email);
    eprintln!("    Plan: {}", capitalize(plan));
    eprintln!("    Last sync: {}", last_sync);
    eprintln!();
    Ok(())
}

fn tracking_history_path() -> PathBuf {
    if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("ig")
            .join("history.jsonl")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("ig")
            .join("history.jsonl")
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        n.to_string()
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}
