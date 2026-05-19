//! `ig aws <service> <verb> [args]` — AWS CLI wrapper with per-service
//! compaction. Auto-injects `--output json` when ig owns args; parses JSON
//! with `serde_json::Value` and renders a compact table or summary.
//! Unknown service/verb combinations fall through to passthrough.

use anyhow::Result;
use serde_json::Value;

use crate::RunOptions;
use crate::cmds::util::{
    ParseOutcome,
    finish::{emit, try_toml_filter},
    spawn::capture,
};

pub fn run(args: &[String], opts: RunOptions) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig aws <service> <verb> [args...]");
    }
    let service = args[0].as_str();
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let rest = if args.len() >= 2 {
        &args[2..]
    } else {
        &args[1..]
    };

    // Pure passthrough: s3 ls etc. are already terse.
    if service == "s3" {
        return passthrough(args, opts, "(s3: passthrough)");
    }

    let renderer: Option<fn(&Value) -> String> = match (service, verb) {
        ("sts", "get-caller-identity") => Some(render_sts),
        ("ec2", "describe-instances") => Some(render_ec2),
        ("lambda", "list-functions") => Some(render_lambda),
        ("dynamodb", "scan") | ("dynamodb", "query") => Some(render_ddb_rows),
        ("iam", "list-roles") => Some(render_iam_roles),
        ("iam", "list-users") => Some(render_iam_users),
        _ => None,
    };

    let Some(render) = renderer else {
        return passthrough(
            args,
            opts,
            &format!("(passthrough: no compact view for {} {})", service, verb),
        );
    };

    let owns = !args.iter().any(|a| a == "--output");
    let mut argv = vec!["aws".to_string(), service.to_string()];
    if !verb.is_empty() {
        argv.push(verb.to_string());
    }
    argv.extend(rest.iter().cloned());
    if owns {
        argv.push("--output".to_string());
        argv.push("json".to_string());
    }
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig aws {} {}", service, verb);
    match serde_json::from_str::<Value>(run.stdout.trim()) {
        Ok(v) => emit(&label, &run, &render(&v), ParseOutcome::Full),
        Err(_) => match try_toml_filter(&argv, &run.merged) {
            Some(f) => emit(&label, &run, &f, ParseOutcome::Passthrough),
            None => emit(&label, &run, &run.merged, ParseOutcome::Passthrough),
        },
    }
    Ok(run.exit_code)
}

fn passthrough(args: &[String], opts: RunOptions, note: &str) -> Result<i32> {
    let mut argv = vec!["aws".to_string()];
    argv.extend(args.iter().cloned());
    let Some(run) = capture(&argv)? else {
        return Ok(127);
    };
    let label = format!("ig aws {}", args.join(" "));
    let mut out = try_toml_filter(&argv, &run.merged).unwrap_or_else(|| run.merged.clone());
    if opts.verbose > 0 {
        eprintln!("{}", note);
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    emit(&label, &run, &out, ParseOutcome::Passthrough);
    Ok(run.exit_code)
}

// ---------- Per-service renderers ----------

pub fn render_sts(v: &Value) -> String {
    let acct = v.get("Account").and_then(|x| x.as_str()).unwrap_or("?");
    let arn = v.get("Arn").and_then(|x| x.as_str()).unwrap_or("?");
    let uid = v.get("UserId").and_then(|x| x.as_str()).unwrap_or("?");
    format!("Account={} Arn={} UserId={}\n", acct, arn, uid)
}

pub fn render_ec2(v: &Value) -> String {
    let mut out = String::from("id\tstate\ttype\tip\tprivate-ip\tname\n");
    let Some(reservations) = v.get("Reservations").and_then(|x| x.as_array()) else {
        return out;
    };
    for r in reservations {
        let Some(insts) = r.get("Instances").and_then(|x| x.as_array()) else {
            continue;
        };
        for i in insts {
            let id = i.get("InstanceId").and_then(|x| x.as_str()).unwrap_or("?");
            let state = i
                .get("State")
                .and_then(|s| s.get("Name"))
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            let typ = i
                .get("InstanceType")
                .and_then(|x| x.as_str())
                .unwrap_or("?");
            let ip = i
                .get("PublicIpAddress")
                .and_then(|x| x.as_str())
                .unwrap_or("-");
            let pip = i
                .get("PrivateIpAddress")
                .and_then(|x| x.as_str())
                .unwrap_or("-");
            let name = i
                .get("Tags")
                .and_then(|t| t.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|tg| tg.get("Key").and_then(|k| k.as_str()) == Some("Name"))
                })
                .and_then(|tg| tg.get("Value"))
                .and_then(|x| x.as_str())
                .unwrap_or("-");
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                id, state, typ, ip, pip, name
            ));
        }
    }
    out
}

pub fn render_lambda(v: &Value) -> String {
    let mut out = String::from("name\truntime\tmem\tsize\tlastModified\n");
    let Some(arr) = v.get("Functions").and_then(|x| x.as_array()) else {
        return out;
    };
    for f in arr {
        let name = f
            .get("FunctionName")
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        let rt = f.get("Runtime").and_then(|x| x.as_str()).unwrap_or("?");
        let mem = f.get("MemorySize").and_then(|x| x.as_i64()).unwrap_or(0);
        let size = f.get("CodeSize").and_then(|x| x.as_i64()).unwrap_or(0);
        let modified = f
            .get("LastModified")
            .and_then(|x| x.as_str())
            .unwrap_or("?");
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            name, rt, mem, size, modified
        ));
    }
    out
}

pub fn render_ddb_rows(v: &Value) -> String {
    let count = v.get("Count").and_then(|x| x.as_i64()).unwrap_or(0);
    let scanned = v
        .get("ScannedCount")
        .and_then(|x| x.as_i64())
        .unwrap_or(count);
    let mut out = format!("count={} scanned={}\n", count, scanned);
    if let Some(items) = v.get("Items").and_then(|x| x.as_array()) {
        for (i, it) in items.iter().take(5).enumerate() {
            out.push_str(&format!(
                "[{}] {}\n",
                i,
                serde_json::to_string(it).unwrap_or_default()
            ));
        }
        if items.len() > 5 {
            out.push_str(&format!("(+{} more rows)\n", items.len() - 5));
        }
    }
    out
}

pub fn render_iam_roles(v: &Value) -> String {
    let mut out = String::from("name\tcreated\n");
    let Some(arr) = v.get("Roles").and_then(|x| x.as_array()) else {
        return out;
    };
    for r in arr {
        let name = r.get("RoleName").and_then(|x| x.as_str()).unwrap_or("?");
        let date = r.get("CreateDate").and_then(|x| x.as_str()).unwrap_or("?");
        out.push_str(&format!("{}\t{}\n", name, date));
    }
    out
}

pub fn render_iam_users(v: &Value) -> String {
    let mut out = String::from("name\tcreated\n");
    let Some(arr) = v.get("Users").and_then(|x| x.as_array()) else {
        return out;
    };
    for u in arr {
        let name = u.get("UserName").and_then(|x| x.as_str()).unwrap_or("?");
        let date = u.get("CreateDate").and_then(|x| x.as_str()).unwrap_or("?");
        out.push_str(&format!("{}\t{}\n", name, date));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sts_compact() {
        let v = json!({"Account": "1234", "Arn": "arn:aws:iam::1234:user/x", "UserId": "AIDA"});
        let out = render_sts(&v);
        assert!(out.contains("Account=1234"));
        assert!(out.contains("Arn=arn:aws:iam::1234:user/x"));
    }

    #[test]
    fn ec2_table() {
        let v = json!({"Reservations": [{"Instances": [{"InstanceId":"i-1","State":{"Name":"running"},"InstanceType":"t3.micro","PublicIpAddress":"1.2.3.4","PrivateIpAddress":"10.0.0.1","Tags":[{"Key":"Name","Value":"app"}]}]}]});
        let out = render_ec2(&v);
        assert!(out.contains("i-1"));
        assert!(out.contains("running"));
        assert!(out.contains("app"));
    }

    #[test]
    fn lambda_table() {
        let v = json!({"Functions": [{"FunctionName":"f","Runtime":"python3.12","MemorySize":256,"CodeSize":1024,"LastModified":"2026-01-01"}]});
        let out = render_lambda(&v);
        assert!(out.contains("f\tpython3.12"));
    }

    #[test]
    fn ddb_rows_truncates() {
        let v = json!({"Count": 7, "ScannedCount": 7, "Items": [{"k":1},{"k":2},{"k":3},{"k":4},{"k":5},{"k":6},{"k":7}]});
        let out = render_ddb_rows(&v);
        assert!(out.contains("count=7"));
        assert!(out.contains("+2 more rows"));
    }

    #[test]
    fn iam_roles_table() {
        let v = json!({"Roles":[{"RoleName":"r1","CreateDate":"2026"}]});
        assert!(render_iam_roles(&v).contains("r1\t2026"));
    }
}
