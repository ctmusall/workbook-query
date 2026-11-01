//! Shared pieces for the exports: a plain summary of a query run.

use crate::query::QueryResult;

/// A summary value: text, or a count (written to Excel as a number).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Text(s) => f.write_str(s),
            Value::Int(i) => write!(f, "{i}"),
        }
    }
}

fn text(s: impl Into<String>) -> Value {
    Value::Text(s.into())
}

/// Label / value rows describing the run: name, mode, conditions, when, rows out per table.
pub fn summary_rows(result: &QueryResult) -> Vec<(String, Value)> {
    let q = &result.query;
    let mut rows: Vec<(String, Value)> = [("Query", &q.name), ("Description", &q.description)]
        .into_iter()
        .filter(|(_, t)| !t.trim().is_empty())
        .map(|(l, t)| (l.to_string(), text(t.as_str())))
        .collect();
    let mode = if q.mode == "join" { "Join on key columns" } else { "Each sheet filtered on its own" };
    rows.push(("Mode".into(), text(mode)));
    if q.mode == "join" {
        rows.push(("Join type".into(), text(q.join_how.as_str())));
        let keys: Vec<String> = q.sources.iter().map(|s| format!("{} › {}", s.alias, s.key)).collect();
        rows.push(("Keys".into(), text(keys.join(", "))));
    }
    rows.push(("Conditions".into(), text(q.r#where.describe())));
    rows.push(("Run at".into(), text(result.run_at.format("%Y-%m-%d %H:%M:%S").to_string())));
    for t in &result.tables {
        rows.push((format!("Rows out: {}", t.name), Value::Int(t.frame.len() as i64)));
    }
    rows
}

pub const SOURCE_HEADERS: [&str; 6] = ["Source", "File", "Sheet", "Rows", "Matched", "SHA-256"];

/// One row per source: alias, path, sheet, rows read, rows matched, SHA-256.
pub fn source_rows(result: &QueryResult) -> Vec<[Value; 6]> {
    result
        .sources
        .iter()
        .map(|s| {
            [
                text(s.alias.as_str()),
                text(s.path.as_str()),
                text(s.sheet.as_str()),
                Value::Int(s.rows as i64),
                s.matched.map_or_else(|| text(""), |m| Value::Int(m as i64)),
                text(s.sha256.clone().unwrap_or_default()),
            ]
        })
        .collect()
}
