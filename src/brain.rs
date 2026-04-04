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
    let mut config: BrainConfig = serde_json::from_str(&content).ok()?;
    if let Ok(url) = std::env::var("BRAIN_API_URL") {
        config.api_url = url;
    }
    Some(config)
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

    let api_url = std::env::var("BRAIN_API_URL")
        .unwrap_or_else(|_| "https://brain.dev/api/v1".to_string());

    let config = BrainConfig {
        token,
        api_url: api_url.clone(),
        auto_sync: true,
    };
    save_config(&config)?;

    eprintln!();
    eprintln!("  \u{2713} Connected to {}", api_url);
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

    // Sync Claude Code project memories (~/.claude/projects/*/memory/*.md)
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let projects_dir = PathBuf::from(&home).join(".claude").join("projects");
    let mut neural_count = 0u32;

    if projects_dir.exists() {
        if let Ok(entries) = fs::read_dir(&projects_dir) {
            for entry in entries.flatten() {
                let memory_dir = entry.path().join("memory");
                if !memory_dir.exists() {
                    continue;
                }

                // Extract project name from dir name (e.g. -Users-kev-...-headless-kit → headless-kit)
                let dir_name = entry.file_name().to_string_lossy().to_string();
                let project_name = extract_project_name(&dir_name);

                if let Ok(files) = fs::read_dir(&memory_dir) {
                    for file in files.flatten() {
                        let path = file.path();
                        if path.extension().and_then(|e| e.to_str()) != Some("md") {
                            continue;
                        }
                        if path.file_name().and_then(|n| n.to_str()) == Some("MEMORY.md") {
                            continue; // Skip index file
                        }

                        let raw = match fs::read_to_string(&path) {
                            Ok(c) => c,
                            Err(_) => continue,
                        };

                        // Parse frontmatter
                        let (name, mem_type, description, content) = parse_frontmatter(&raw, &path);

                        let payload = serde_json::json!({
                            "name": name,
                            "type": mem_type,
                            "description": description,
                            "content": content,
                            "project": project_name,
                        });

                        let _ = ureq::post(&format!("{}/brain/memories/sync", config.api_url))
                            .header("Authorization", &format!("Bearer {}", config.token))
                            .header("Content-Type", "application/json")
                            .send_json(&payload);

                        neural_count += 1;
                    }
                }
            }
        }
    }

    eprintln!("    Memories: {} synced ({} from MEMORY.md, {} neural)",
        memory_count + neural_count, memory_count, neural_count);
    eprintln!(
        "    Stats: {} commands, {:.1}MB saved",
        format_number(total_commands),
        total_saved as f64 / 1_048_576.0
    );
    eprintln!("  \u{2713} Sync complete");
    Ok(())
}

fn extract_project_name(dir_name: &str) -> String {
    if dir_name.starts_with('-') {
        let segments: Vec<&str> = dir_name.split('-').filter(|s| !s.is_empty()).collect();
        let skip = ["Users", "kev", "Documents", "lab", "sandbox", "perso"];
        let meaningful: Vec<&&str> = segments.iter().filter(|s| !skip.contains(s)).collect();
        if meaningful.is_empty() {
            dir_name.to_string()
        } else {
            meaningful.iter().copied().copied().collect::<Vec<&str>>().join("-")
        }
    } else {
        dir_name.to_string()
    }
}

fn parse_frontmatter(raw: &str, path: &PathBuf) -> (String, String, String, String) {
    let file_stem = path
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut name = file_stem;
    let mut mem_type = "project".to_string();
    let mut description = String::new();
    let mut content = raw.to_string();

    if raw.starts_with("---") {
        if let Some(end) = raw[3..].find("\n---") {
            let fm = &raw[3..3 + end];
            content = raw[3 + end + 4..].trim().to_string();
            for line in fm.lines() {
                let line = line.trim();
                if let Some(val) = line.strip_prefix("name:") {
                    name = val.trim().to_string();
                } else if let Some(val) = line.strip_prefix("type:") {
                    mem_type = val.trim().to_string();
                } else if let Some(val) = line.strip_prefix("description:") {
                    description = val.trim().to_string();
                }
            }
        }
    }

    (name, mem_type, description, content)
}

pub fn brain_pull(quiet: bool) -> Result<()> {
    let config = load_config().context("not logged in — run `ig brain login` first")?;
    let auth = format!("Bearer {}", config.token);

    // --- Fetch skills ---
    let skills = match ureq::get(&format!("{}/brain/skills", config.api_url))
        .header("Authorization", &auth)
        .call()
    {
        Ok(mut resp) => {
            let json: serde_json::Value = resp.body_mut().read_json().unwrap_or_default();
            json.get("data")
                .and_then(|d| d.get("skills"))
                .and_then(|s| s.as_array())
                .cloned()
                .unwrap_or_default()
        }
        Err(_) => {
            if !quiet {
                eprintln!("  ⚠ Could not fetch skills (API unreachable)");
            }
            vec![]
        }
    };

    let skills_dir = PathBuf::from(".brain").join("skills");
    fs::create_dir_all(&skills_dir).ok();
    for skill in &skills {
        let name = skill.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
        let content = skill.get("content").and_then(|c| c.as_str()).unwrap_or("");
        let filename = format!("{}.md", name);
        fs::write(skills_dir.join(&filename), content).ok();
    }

    // --- Fetch rules ---
    let rules = match ureq::get(&format!("{}/brain/rules", config.api_url))
        .header("Authorization", &auth)
        .call()
    {
        Ok(mut resp) => {
            let json: serde_json::Value = resp.body_mut().read_json().unwrap_or_default();
            json.get("data")
                .and_then(|d| d.get("rules"))
                .and_then(|s| s.as_array())
                .cloned()
                .unwrap_or_default()
        }
        Err(_) => vec![],
    };

    let rules_dir = PathBuf::from(".claude").join("rules");
    fs::create_dir_all(&rules_dir).ok();
    for rule in &rules {
        let filename = rule.get("filename").and_then(|n| n.as_str()).unwrap_or("unknown");
        let content = rule.get("content").and_then(|c| c.as_str()).unwrap_or("");
        let safe_name = format!("brain-{}", filename.replace('/', "-"));
        let path = rules_dir.join(&safe_name);
        // Skip write if unchanged (avoid triggering re-reads)
        let existing = fs::read_to_string(&path).unwrap_or_default();
        if existing != content {
            fs::write(&path, content).ok();
        }
    }

    // --- Fetch memories context for current project ---
    let cwd_name = std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default();

    let (project_memories, cross_project) = match ureq::get(
        &format!("{}/brain/memories/context?project={}", config.api_url, cwd_name),
    )
    .header("Authorization", &auth)
    .call()
    {
        Ok(mut resp) => {
            let json: serde_json::Value = resp.body_mut().read_json().unwrap_or_default();
            let data = json.get("data").cloned().unwrap_or_default();
            let mems = data.get("memories").and_then(|m| m.as_array()).cloned().unwrap_or_default();
            let cross = data.get("cross_project").and_then(|c| c.as_array()).cloned().unwrap_or_default();
            (mems, cross)
        }
        Err(_) => (vec![], vec![]),
    };

    // --- Generate brain-context.md ---
    let mut context = String::from("# brain.dev — Project Context\n\n");

    // Project memories (most valuable — Claude's own understanding)
    if !project_memories.is_empty() {
        context.push_str(&format!("## This Project ({})\n", cwd_name));
        for mem in &project_memories {
            let name = mem.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let content = mem.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let truncated = if first_line.len() > 120 { &first_line[..120] } else { first_line };
            context.push_str(&format!("- {}: {}\n", name, truncated));
        }
        context.push('\n');
    }

    // Cross-project memories
    if !cross_project.is_empty() {
        context.push_str("## Other Projects\n");
        for mem in &cross_project {
            let name = mem.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let project = mem.get("project").and_then(|p| p.as_str()).unwrap_or("?");
            let content = mem.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let truncated = if first_line.len() > 100 { &first_line[..100] } else { first_line };
            context.push_str(&format!("- {} ({}): {}\n", name, project, truncated));
        }
        context.push('\n');
    }

    if !rules.is_empty() {
        context.push_str("## Active Rules\n");
        for rule in &rules {
            let filename = rule.get("filename").and_then(|n| n.as_str()).unwrap_or("?");
            let content = rule.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let truncated = if first_line.len() > 80 { &first_line[..80] } else { first_line };
            context.push_str(&format!("- {}: {}\n", filename, truncated));
        }
        context.push('\n');
    }

    if !skills.is_empty() {
        context.push_str("## Available Skills\n");
        for skill in &skills {
            let name = skill.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let content = skill.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let first_line = content.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            let truncated = if first_line.len() > 80 { &first_line[..80] } else { first_line };
            context.push_str(&format!("- {}: {}\n", name, truncated));
        }
        context.push('\n');
    }

    // Write brain-context.md only if changed
    let context_path = rules_dir.join("brain-context.md");
    let existing = fs::read_to_string(&context_path).unwrap_or_default();
    if existing != context {
        fs::write(&context_path, &context).ok();
    }

    let mem_count = project_memories.len() + cross_project.len();
    if !quiet {
        eprintln!("  \u{2713} Pulled {} skills, {} rules, {} memories", skills.len(), rules.len(), mem_count);
    }
    Ok(())
}

pub fn brain_status() -> Result<()> {
    let config = load_config().context("not logged in — run `ig brain login` first")?;

    let response: serde_json::Value = ureq::get(&format!("{}/brain/status", config.api_url))
        .header("Authorization", &format!("Bearer {}", config.token))
        .call()
        .context("failed to reach brain.dev — check your connection")?
        .body_mut()
        .read_json()
        .context("failed to parse response")?;

    let data = response.get("data").unwrap_or(&response);
    let email = data.get("email").and_then(|e| e.as_str()).unwrap_or("unknown");
    let name = data.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let plan = data.get("plan").and_then(|p| p.as_str()).unwrap_or("free");
    let memories = data.get("memories_count").and_then(|m| m.as_u64()).unwrap_or(0);
    let orgs = data.get("orgs").and_then(|o| o.as_array()).cloned().unwrap_or_default();

    let org_names: Vec<&str> = orgs
        .iter()
        .filter_map(|o| o.get("name").and_then(|n| n.as_str()))
        .collect();

    // Check hooks in ~/.claude/settings.json
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let settings_path = PathBuf::from(&home).join(".claude").join("settings.json");
    let settings_content = fs::read_to_string(&settings_path).unwrap_or_default();
    let has_inject = settings_content.contains("brain-inject");
    let has_capture = settings_content.contains("brain-capture");
    let has_session = settings_content.contains("brain-session");

    let hook_status = |installed: bool| if installed { "\u{2713}" } else { "\u{2717}" };

    eprintln!();
    if !name.is_empty() {
        eprintln!("  brain.dev — Connected ({})", name);
    } else {
        eprintln!("  brain.dev — Connected");
    }
    eprintln!("    User: {}", email);
    eprintln!("    Plan: {}", capitalize(plan));
    if !org_names.is_empty() {
        eprintln!("    Orgs: {}", org_names.join(", "));
    }
    eprintln!("    Memories: {}", memories);
    eprintln!("    Hooks: inject {}  capture {}  session {}",
        hook_status(has_inject), hook_status(has_capture), hook_status(has_session));
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
