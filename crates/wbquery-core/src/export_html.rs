//! Write query results to a single self-contained HTML report.

use std::path::{Path, PathBuf};

use crate::VERSION;
use crate::conditions::text_color;
use crate::error::{Error, Result};
use crate::pyfmt::html_escape;
use crate::query::{QueryResult, ResultTable};
use crate::report::{SOURCE_HEADERS, source_rows, summary_rows};

const CSS: &str = r#"
:root { --bg:#fff; --fg:#1d232b; --muted:#5b6672; --line:#d6dce3; --head:#eef2f6; --stripe:#f7f9fb; --warn:#8a5a00; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#16191d; --fg:#e4e8ec; --muted:#9aa5b1; --line:#323940; --head:#222830; --stripe:#1b1f24; --warn:#e0b050; }
}
body { background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif;
       margin:0 auto; padding:24px 16px; max-width:1400px; }
h1 { font-size:22px; margin:0 0 4px; } h2 { font-size:17px; margin:28px 0 8px; }
.muted { color:var(--muted); } .warn { color:var(--warn); }
.scroll { overflow-x:auto; border:1px solid var(--line); border-radius:6px; }
table { border-collapse:collapse; width:100%; font-variant-numeric:tabular-nums; }
th, td { padding:5px 10px; border-bottom:1px solid var(--line); text-align:left; white-space:nowrap; }
th { background:var(--head); position:sticky; top:0; }
tbody tr:nth-child(even) td { background:var(--stripe); }
table.kv th { width:12em; white-space:normal; } table.kv td { white-space:pre-line; }
code { font-size:12px; }
"#;

fn table(t: &ResultTable) -> String {
    let f = &t.frame;
    let head: String = f.columns.iter().map(|c| format!("<th>{}</th>", html_escape(&c.name))).collect();
    let body: Vec<String> = (0..f.len())
        .map(|i| {
            let tds: String = f
                .columns
                .iter()
                .enumerate()
                .map(|(j, c)| {
                    let color = &t.colors[j][i];
                    let style = if color.is_empty() {
                        String::new()
                    } else {
                        format!(r#" style="background:{color};color:{}""#, text_color(color))
                    };
                    format!("<td{style}>{}</td>", html_escape(&c.cells[i].display()))
                })
                .collect();
            format!("<tr>{tds}</tr>")
        })
        .collect();
    format!(
        "<div class=\"scroll\"><table><thead><tr>{head}</tr></thead><tbody>\n{}\n</tbody></table></div>",
        body.join("\n")
    )
}

/// The report as HTML text. `title` defaults to the query's name.
pub fn render_html(result: &QueryResult, title: &str) -> String {
    let name = result.query.name.trim();
    let title = if !title.is_empty() {
        title
    } else if !name.is_empty() {
        name
    } else {
        "Workbook Query results"
    };
    let kv: String = summary_rows(result)
        .iter()
        .map(|(k, v)| format!("<tr><th>{}</th><td>{}</td></tr>", html_escape(k), html_escape(&v.to_string())))
        .collect();
    let warn: String =
        result.warnings.iter().map(|w| format!("<p class=\"warn\">⚠ {}</p>", html_escape(w))).collect();
    let src_head: String = SOURCE_HEADERS.iter().map(|h| format!("<th>{h}</th>")).collect();
    let src_body: String = source_rows(result)
        .iter()
        .map(|r| {
            let tds: String = r
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let (open, close) = if i == 5 { ("<code>", "</code>") } else { ("", "") };
                    format!("<td>{open}{}{close}</td>", html_escape(&v.to_string()))
                })
                .collect();
            format!("<tr>{tds}</tr>")
        })
        .collect();
    let sections: String = result
        .tables
        .iter()
        .map(|t| {
            let n = t.frame.len();
            format!(
                "<h2>{} <span class=\"muted\">({n} row{})</span></h2>{}",
                html_escape(&t.name),
                if n == 1 { "" } else { "s" },
                table(t)
            )
        })
        .collect();
    let title = html_escape(title);
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title><style>{CSS}</style></head>
<body>
<h1>{title}</h1>
<p class="muted">Workbook Query {VERSION}</p>
<div class="scroll"><table class="kv"><tbody>{kv}</tbody></table></div>
{warn}
{sections}
<h2>Sources</h2>
<div class="scroll"><table><thead><tr>{src_head}</tr></thead><tbody>{src_body}</tbody></table></div>
</body></html>
"#
    )
}

/// `path` with an `.html` suffix (kept when it is already `.html` or `.htm`).
pub fn html_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    match ext.as_deref() {
        Some("html") | Some("htm") => path.to_path_buf(),
        _ => path.with_extension("html"),
    }
}

/// Write the report; returns the path written (always ending in `.html` / `.htm`).
pub fn write_html(result: &QueryResult, path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = html_path(path);
    std::fs::write(&path, render_html(result, ""))
        .map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
    Ok(path)
}
