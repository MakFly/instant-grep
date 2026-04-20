//! `ig json <file> [--schema]` — compact JSON viewer with optional schema mode.
//!
//! Normal mode: re-serializes JSON in compact form.
//! Schema mode (`--schema`): replaces all values with type placeholders and
//! collapses arrays to show element type + count.

use anyhow::Result;
use std::path::Path;

/// Run the json command.
pub fn run(args: &[String]) -> Result<i32> {
    if args.is_empty() {
        anyhow::bail!("Usage: ig json <file> [--schema]");
    }

    let file = &args[0];
    let schema_mode = args.iter().any(|a| a == "--schema");

    let content = std::fs::read_to_string(Path::new(file))?;
    let value: serde_json::Value = serde_json::from_str(&content)?;

    let output = if schema_mode {
        let schema = to_schema(&value);
        serde_json::to_string_pretty(&schema)?
    } else {
        serde_json::to_string(&value)?
    };

    println!("{}", output);

    // Track savings
    let project = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    crate::tracking::log_savings(&crate::tracking::TrackEntry {
        command: format!("ig json {}", args.join(" ")),
        original_bytes: content.len() as u64,
        output_bytes: output.len() as u64,
        project,
    });

    Ok(0)
}

/// Convert a JSON value into a schema representation.
///
/// - Strings → `"string"`
/// - Integers → `"int"`
/// - Floats → `"float"`
/// - Booleans → `"bool"`
/// - Null → `"null"`
/// - Objects → recurse into each key
/// - Arrays → show first element type + `"(N)"` suffix
fn to_schema(value: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;

    match value {
        Value::Null => Value::String("null".into()),
        Value::Bool(_) => Value::String("bool".into()),
        Value::Number(n) => {
            if n.is_f64() && n.as_i64().is_none() {
                Value::String("float".into())
            } else {
                Value::String("int".into())
            }
        }
        Value::String(_) => Value::String("string".into()),
        Value::Array(arr) => {
            if arr.is_empty() {
                Value::String("[] (0)".into())
            } else {
                let first_schema = to_schema(&arr[0]);
                let count = arr.len();
                Value::Array(vec![first_schema, Value::String(format!("({count})"))])
            }
        }
        Value::Object(map) => {
            let mut schema_map = serde_json::Map::new();
            for (key, val) in map {
                schema_map.insert(key.clone(), to_schema(val));
            }
            Value::Object(schema_map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_scalars() {
        assert_eq!(to_schema(&json!("hello")), json!("string"));
        assert_eq!(to_schema(&json!(42)), json!("int"));
        assert_eq!(to_schema(&json!(2.5)), json!("float"));
        assert_eq!(to_schema(&json!(true)), json!("bool"));
        assert_eq!(to_schema(&json!(null)), json!("null"));
    }

    #[test]
    fn schema_object() {
        let input = json!({"name": "test", "count": 5});
        let schema = to_schema(&input);
        assert_eq!(schema["name"], json!("string"));
        assert_eq!(schema["count"], json!("int"));
    }

    #[test]
    fn schema_array() {
        let input = json!([1, 2, 3]);
        let schema = to_schema(&input);
        assert_eq!(schema, json!(["int", "(3)"]));
    }

    #[test]
    fn schema_empty_array() {
        let input = json!([]);
        let schema = to_schema(&input);
        assert_eq!(schema, json!("[] (0)"));
    }
}
