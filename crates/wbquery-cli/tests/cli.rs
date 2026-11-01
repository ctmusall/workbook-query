//! The built `wbquery-cli` binary: exit codes, messages and the files it writes.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use calamine::{Reader, open_workbook_auto};

fn data(name: &str) -> PathBuf {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/data").join(name);
    std::fs::canonicalize(p).unwrap()
}

/// A join query over the golden lots / results workbooks: R.Value >= 5e7, outputs L.Status and R.Value.
fn query_file(dir: &Path) -> PathBuf {
    let doc = serde_json_text(&data("lots.xlsx"), &data("results.xlsx"));
    let path = dir.join("q.json");
    std::fs::write(&path, doc).unwrap();
    path
}

fn serde_json_text(lots: &Path, results: &Path) -> String {
    let esc = |p: &Path| p.to_string_lossy().replace('\\', "\\\\");
    format!(
        r#"{{"version": 1, "mode": "join", "join_how": "inner",
  "sources": [{{"alias": "L", "path": "{}", "sheet": "Lots", "key": "Lot"}},
              {{"alias": "R", "path": "{}", "sheet": "Results", "key": "Lot ID"}}],
  "where": {{"logic": "AND", "items": [{{"source": "R", "column": "Value", "op": ">=", "value": "5e7"}}]}},
  "outputs": {{"L": ["Status"], "R": ["Value"]}}}}"#,
        esc(lots),
        esc(results)
    )
}

fn cli() -> Command {
    Command::cargo_bin("wbquery-cli").unwrap()
}

fn rows(path: &Path, sheet: &str) -> usize {
    let mut wb = open_workbook_auto(path).unwrap();
    wb.worksheet_range(sheet).unwrap().height()
}

#[test]
fn run() {
    let dir = tempfile::tempdir().unwrap();
    let q = query_file(dir.path());
    let out = cli()
        .args(["-q", q.to_str().unwrap(), "-o", dir.path().join("res").to_str().unwrap(), "--print"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("Joined: 3 row(s)"), "{stdout}");
    assert!(stdout.contains("L.Status"), "--print shows the table: {stdout}");
    assert!(stderr.contains("warning:"), "{stderr}");
    assert!(std::fs::metadata(dir.path().join("res.html")).unwrap().len() > 0);
    assert_eq!(rows(&dir.path().join("res.xlsx"), "Joined"), 4);
}

#[test]
fn out_dir_and_formats() {
    let dir = tempfile::tempdir().unwrap();
    let q = query_file(dir.path());
    cli()
        .args(["-q", q.to_str().unwrap(), "-o", dir.path().to_str().unwrap(), "--formats", "html"])
        .assert()
        .code(0);
    assert!(dir.path().join("query_results.html").exists());
    assert!(!dir.path().join("query_results.xlsx").exists());
    cli().args(["-q", q.to_str().unwrap(), "--formats", "pdf"]).assert().code(2);
}

#[test]
fn source_override() {
    let dir = tempfile::tempdir().unwrap();
    let q = query_file(dir.path());
    let x = dir.path().join("x");
    let out = cli()
        .args(["-q", q.to_str().unwrap(), "--source", "R=missing.xlsx", "-o", x.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("R:"));
    let results = format!("R={}", data("results.xlsx").display());
    cli().args(["-q", q.to_str().unwrap(), "--source", &results, "-o", x.to_str().unwrap()]).assert().code(0);
    cli().args(["-q", q.to_str().unwrap(), "--source", "Nope=x.xlsx"]).assert().code(1);
    cli().args(["-q", q.to_str().unwrap(), "--source", "bad"]).assert().code(2);
}

#[test]
fn list_and_errors() {
    let out = cli().args(["--list", data("lots.xlsx").to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("[Lots] Lot, Status, Made, Notes"));
    cli().assert().code(2);
    let out = cli().arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains(env!("CARGO_PKG_VERSION")));
    cli().args(["-q", "missing.json"]).assert().code(1);
}
