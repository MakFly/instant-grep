//! `ig curl [args]` — curl wrapper. Auto-injects `-sS -i` (silent + include
//! response headers) when ig owns args, captures headers + body. Bodies
//! larger than 1 MiB are truncated in the visible output and the full body
//! is saved to the tee store.

use std::io::IsTerminal;

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{ParseOutcome, finish::try_toml_filter, spawn::capture};
use crate::tee;
use crate::tracking;

const BODY_PREVIEW_BYTES: usize = 64 * 1024;
const TEE_THRESHOLD_BYTES: usize = 1024 * 1024;

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    // Opt-in safety per spec: only auto-tee when stdin is not a TTY AND
    // stdout is captured.
    let auto_safe = !std::io::stdin().is_terminal() && !std::io::stdout().is_terminal();

    let owns = !args.iter().any(|a| a == "-o" || a == "--output");
    let mut argv = vec!["curl".to_string()];
    if owns {
        argv.push("-sS".to_string());
        argv.push("-i".to_string());
    }
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig curl {}", args.join(" "));

    let raw = &run.stdout;
    let final_out = if raw.len() > BODY_PREVIEW_BYTES {
        let cut = (0..BODY_PREVIEW_BYTES)
            .rev()
            .find(|i| raw.is_char_boundary(*i))
            .unwrap_or(0);
        let mut s = raw[..cut].to_string();
        s.push_str(&format!("\n(truncated; {} bytes total)\n", raw.len()));
        if auto_safe
            && raw.len() > TEE_THRESHOLD_BYTES
            && let Some(id) = tee::save(raw.as_bytes(), &label)
        {
            s.push_str(&format!(
                "(full body at tee:{} — run `ig tee show {}`)\n",
                id, id
            ));
        }
        s
    } else if let Some(f) = try_toml_filter(&argv, &run.merged) {
        f
    } else {
        run.merged.clone()
    };

    print!("{}", final_out);
    tracking::log_savings(&tracking::TrackEntry {
        command: label,
        original_bytes: run.merged.len() as u64,
        output_bytes: final_out.len() as u64,
        project: tracking::current_project(),
        exit_code: Some(run.exit_code),
        parse_outcome: Some(ParseOutcome::Partial.as_str().to_string()),
        ..Default::default()
    });
    Ok(run.exit_code)
}

#[cfg(test)]
mod tests {
    #[test]
    fn body_preview_constant_is_64k() {
        assert_eq!(super::BODY_PREVIEW_BYTES, 64 * 1024);
    }
}
