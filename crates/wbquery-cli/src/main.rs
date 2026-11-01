//! Command line: run a saved query (JSON) and write the results.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use wbquery_core::cell::Frame;
use wbquery_core::export_html::write_html;
use wbquery_core::export_xlsx::write_xlsx;
use wbquery_core::{Error, load_query, loader, run_query};

const FORMATS: &[&str] = &["xlsx", "html"];

#[derive(Parser, Debug)]
#[command(
    name = "wbquery-cli",
    version,
    about = "Filter rows across Excel workbooks and sheets with a saved query, and export the matches."
)]
struct Args {
    /// Query saved from the app (or written by hand).
    #[arg(short, long, value_name = "FILE.json")]
    query: Option<PathBuf>,

    /// Read source ALIAS from PATH instead of the path saved in the query (repeatable).
    #[arg(long, value_name = "ALIAS=PATH")]
    source: Vec<String>,

    /// Output file name without extension, or an existing directory.
    #[arg(short, long, default_value = "query_results")]
    out: PathBuf,

    /// Comma-separated: xlsx, html.
    #[arg(long, default_value = "xlsx,html")]
    formats: String,

    /// Also print the result tables.
    #[arg(long)]
    print: bool,

    /// List each file's sheets and columns, then exit.
    #[arg(long, value_name = "FILE", num_args = 1..)]
    list: Option<Vec<PathBuf>>,
}

fn list(files: &[PathBuf]) -> Result<(), Error> {
    for f in files {
        println!("{}", f.display());
        for sheet in loader::list_sheets(f)? {
            let frame = loader::load_table(f, Some(&sheet))?;
            println!("  [{sheet}] {}", frame.names().join(", "));
        }
    }
    Ok(())
}

/// The table as aligned text: a header line, then one line per row, values right-aligned.
fn format_table(frame: &Frame) -> String {
    let cols: Vec<Vec<String>> = frame
        .columns
        .iter()
        .map(|c| std::iter::once(c.name.clone()).chain(c.cells.iter().map(|v| v.display())).collect())
        .collect();
    let widths: Vec<usize> =
        cols.iter().map(|c| c.iter().map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for i in 0..=frame.len() {
        let line: Vec<String> =
            cols.iter().zip(&widths).map(|(c, w)| format!("{:>w$}", c[i], w = *w)).collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
    }
    out
}

/// `out` with `ext` in place of its last suffix (Python's `Path.with_suffix`).
fn with_suffix(out: &Path, ext: &str) -> PathBuf {
    out.with_extension(ext)
}

enum Failure {
    Usage(String),
    Run(String),
}

fn run(args: Args) -> Result<(), Failure> {
    if let Some(files) = &args.list {
        return list(files).map_err(|e| Failure::Run(e.to_string()));
    }
    let Some(query_path) = &args.query else {
        return Err(Failure::Usage(
            "give --query FILE.json (or --list FILE to see sheets and columns)".into(),
        ));
    };
    let formats: Vec<String> =
        args.formats.split(',').map(|f| f.trim().to_lowercase()).filter(|f| !f.is_empty()).collect();
    if formats.is_empty() || formats.iter().any(|f| !FORMATS.contains(&f.as_str())) {
        return Err(Failure::Usage(format!("--formats takes {}", FORMATS.join(", "))));
    }
    let mut query = load_query(query_path).map_err(|e| Failure::Run(e.to_string()))?;
    for item in &args.source {
        let Some((alias, path)) = item.split_once('=') else {
            return Err(Failure::Usage(format!("--source needs ALIAS=PATH, not '{item}'")));
        };
        query.source_mut(alias.trim()).map_err(|e| Failure::Run(e.to_string()))?.path =
            path.trim().to_string();
    }
    let result = run_query(&query, None).map_err(|e| Failure::Run(e.to_string()))?;

    for w in &result.warnings {
        eprintln!("warning: {w}");
    }
    let mut out = args.out.clone();
    if out.is_dir() {
        out = out.join("query_results");
    }
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| Failure::Run(format!("{}: {e}", parent.display())))?;
    }
    let mut written = Vec::new();
    if formats.iter().any(|f| f == "xlsx") {
        written
            .push(write_xlsx(&result, with_suffix(&out, "xlsx")).map_err(|e| Failure::Run(e.to_string()))?);
    }
    if formats.iter().any(|f| f == "html") {
        written
            .push(write_html(&result, with_suffix(&out, "html")).map_err(|e| Failure::Run(e.to_string()))?);
    }
    for t in &result.tables {
        println!("{}: {} row(s)", t.name, t.frame.len());
        if args.print {
            print!("{}", format_table(&t.frame));
        }
    }
    for p in written {
        println!("wrote {}", p.display());
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
        Err(Failure::Run(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(1)
        }
    }
}
