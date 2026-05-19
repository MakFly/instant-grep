#![allow(clippy::collapsible_if, clippy::collapsible_match)]
//! Per-agent installers registered with the `ig setup` / `ig init` entry
//! point.
//!
//! Each agent owns one file under `agents/` and implements the
//! [`AgentInstaller`](super::AgentInstaller) trait. The dispatcher
//! [`super::run_per_agent`] iterates `all()` to install / uninstall / show
//! integrations.

use super::AgentInstaller;

pub mod antigravity;
pub mod claude;
pub mod cline;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod hermes;
pub mod kilocode;
pub mod opencode;
pub mod windsurf;

/// The full registered installer set. Order matters only for printed output —
/// Claude first because it's the most invasive (hooks + settings.json),
/// stub-only / experimental agents last.
pub fn all() -> Vec<Box<dyn AgentInstaller>> {
    vec![
        Box::new(claude::Claude),
        Box::new(codex::Codex),
        Box::new(cursor::Cursor),
        Box::new(copilot::Copilot),
        Box::new(gemini::Gemini),
        Box::new(opencode::OpenCode),
        Box::new(windsurf::Windsurf),
        Box::new(cline::Cline),
        Box::new(hermes::Hermes),
        Box::new(kilocode::Kilocode),
        Box::new(antigravity::Antigravity),
    ]
}
