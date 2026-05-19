//! Shared "print output + tee fallback + track savings" logic for the
//! per-tool parsers.

use crate::filter::{FilterEngine, apply_filter};
use crate::tee;
use crate::tracking;

use super::ParseOutcome;
use super::spawn::CapturedRun;

/// Write `final_output` to stdout, save raw bytes to the tee store if the
/// filter removed a lot of content on a failing run, and record a tracking
/// row for the SQLite analytics DB.
pub fn emit(
    command_label: &str,
    raw: &CapturedRun,
    final_output: &str,
    parse_outcome: ParseOutcome,
) {
    let mut out = final_output.to_string();

    if tee::should_save(raw.merged.len(), out.len(), raw.exit_code)
        && let Some(id) = tee::save(raw.merged.as_bytes(), command_label)
    {
        out.push_str(&format!(
            "\n[ig: full output saved — run `ig tee show {}` to read it]\n",
            id
        ));
    }

    print!("{}", out);

    tracking::log_savings(&tracking::TrackEntry {
        command: command_label.to_string(),
        original_bytes: raw.merged.len() as u64,
        output_bytes: out.len() as u64,
        project: tracking::current_project(),
        exec_time_ms: None,
        exit_code: Some(raw.exit_code),
        parse_outcome: Some(parse_outcome.as_str().to_string()),
    });
}

/// Try the TOML filter pipeline for `argv` (joined as a command line) and
/// return the filtered output if a filter matched. Returns `None` when no
/// TOML filter matched — the caller should fall back to raw passthrough.
pub fn try_toml_filter(argv: &[String], raw: &str) -> Option<String> {
    let engine = FilterEngine::new();
    let cmd_str = argv.join(" ");
    if let Some(f) = engine.find(&cmd_str) {
        return Some(apply_filter(f, raw));
    }
    let basename = std::path::Path::new(&argv[0])
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(&argv[0]);
    if basename != argv[0] {
        let normalized = std::iter::once(basename.to_string())
            .chain(argv.iter().skip(1).cloned())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(f) = engine.find(&normalized) {
            return Some(apply_filter(f, raw));
        }
    }
    None
}
