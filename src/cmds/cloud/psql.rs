//! `ig psql [args]` — psql wrapper. For SELECT statements, surface a row
//! count + the first 10 rows. For DML statements, surface only the command
//! tag (`INSERT 0 N`, `UPDATE N`, …). Everything else passes through.

use anyhow::Result;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], _opts: RunOptions) -> Result<i32> {
    let mut argv = vec!["psql".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig psql {}", args.join(" "));
    let out = if let Some(f) = try_toml_filter(&argv, &run.merged) {
        f
    } else {
        compact_psql(&run.stdout)
    };
    emit(&label, &run, &out, ParseOutcome::Partial);
    Ok(run.exit_code)
}

/// Look for the SELECT row-count footer (`(N rows)`) and the command tag
/// trailers. Surface first 10 rows + total when a SELECT result is detected.
pub fn compact_psql(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    // Find a "(N rows)" or "(N row)" line.
    let rows_re = regex::Regex::new(r"^\((\d+)\s+rows?\)\s*$").unwrap();
    if let Some((idx, total)) = lines.iter().enumerate().find_map(|(i, l)| {
        rows_re
            .captures(l.trim())
            .and_then(|c| c[1].parse::<usize>().ok().map(|n| (i, n)))
    }) {
        // psql table layout:
        //   col | col
        //  -----+-----
        //   v   | v
        //  ...
        //  (N rows)
        // The separator line is the `-----+-----` line. Header is one line above.
        let sep_idx = lines
            .iter()
            .enumerate()
            .take(idx)
            .position(|(_, l)| l.trim_start().starts_with('-') && l.contains('+'));
        let header_idx = sep_idx.and_then(|i| i.checked_sub(1));
        let body_start = sep_idx.map(|i| (i + 1).min(idx)).unwrap_or(0);
        let body_end = idx;
        let body_len = body_end.saturating_sub(body_start);
        let take = body_len.min(10);
        let mut out = String::new();
        if let Some(h) = header_idx {
            out.push_str(lines[h]);
            out.push('\n');
            if h + 1 < idx {
                out.push_str(lines[h + 1]);
                out.push('\n');
            }
        }
        for l in &lines[body_start..body_start + take] {
            out.push_str(l);
            out.push('\n');
        }
        if body_len > take {
            out.push_str(&format!("... (+{} more rows)\n", body_len - take));
        }
        out.push_str(&format!("({} rows)\n", total));
        return out;
    }
    // Look for command tags
    let tag_re =
        regex::Regex::new(r"^(INSERT|UPDATE|DELETE|SELECT|COPY)\s+\d+(\s+\d+)?\s*$").unwrap();
    let tags: Vec<&&str> = lines.iter().filter(|l| tag_re.is_match(l.trim())).collect();
    if !tags.is_empty() {
        let mut out = String::new();
        for t in tags {
            out.push_str(t);
            out.push('\n');
        }
        return out;
    }
    // Default: passthrough.
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_select_truncates_to_ten() {
        let mut s = String::from(" id | name\n----+------\n");
        for i in 0..25 {
            s.push_str(&format!("  {} | x{}\n", i, i));
        }
        s.push_str("(25 rows)\n");
        let out = compact_psql(&s);
        assert!(out.contains("(25 rows)"));
        assert!(out.contains("+15 more"));
    }

    #[test]
    fn compact_insert_surfaces_tag() {
        let s = "INSERT 0 3\n";
        let out = compact_psql(s);
        assert!(out.contains("INSERT 0 3"));
    }

    #[test]
    fn compact_update_surfaces_tag() {
        let s = "UPDATE 17\n";
        let out = compact_psql(s);
        assert!(out.contains("UPDATE 17"));
    }
}
