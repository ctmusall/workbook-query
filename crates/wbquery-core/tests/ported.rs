//! Checks ported by hand from the Python test suite (the ones the golden fixtures don't cover):
//! saving and loading queries, and the files the exports write.

use std::path::{Path, PathBuf};

use calamine::{Data, Reader, open_workbook_auto};
use wbquery_core::conditions::{Condition, Group, Node};
use wbquery_core::export_html::{render_html, write_html};
use wbquery_core::export_xlsx::write_xlsx;
use wbquery_core::query::{Query, Source, load_query, run_query, save_query};

fn data(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/data").join(name);
    std::fs::canonicalize(p).unwrap().to_string_lossy().into_owned()
}

fn two_sources() -> Vec<Source> {
    vec![
        Source::new("L", &data("lots.xlsx"), "Lots", "Lot"),
        Source::new("R", &data("results.xlsx"), "Results", "Lot ID"),
    ]
}

fn cond(source: &str, column: &str, op: &str, value: &str) -> Node {
    Node::Cond(Condition::new(source, column, op, value).unwrap())
}

#[test]
fn round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let q = Query {
        sources: two_sources(),
        mode: "join".into(),
        join_how: "left".into(),
        r#where: Group::or(vec![cond("L", "Status", "=", "Pass"), Node::Group(Group::and(vec![]).negated())]),
        outputs: vec![("L".into(), vec!["Lot".into()]), ("R".into(), vec!["Value".into(), "Pct".into()])],
        include_provenance: false,
        name: "Lots vs results".into(),
        description: "Line one\nline two".into(),
    };
    let path = save_query(&q, dir.path().join("q.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc["version"], 1);
    assert_eq!(load_query(&path).unwrap(), q);
}

#[test]
fn unknown_keys_ignored_and_errors() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("q.json");
    std::fs::write(
        &p,
        r#"{"extra": 1, "sources": [{"alias": "A", "path": "a.xlsx", "sheet": "S", "x": 2}]}"#,
    )
    .unwrap();
    let q = load_query(&p).unwrap();
    assert_eq!(q.sources, vec![Source::new("A", "a.xlsx", "S", "")]);
    assert_eq!((q.name.as_str(), q.description.as_str()), ("", ""));
    std::fs::write(&p, "{nope").unwrap();
    assert!(load_query(&p).unwrap_err().to_string().contains("not valid JSON"));
    std::fs::write(&p, r#"{"sources": [{"alias": "A"}]}"#).unwrap();
    assert!(load_query(&p).unwrap_err().to_string().contains("needs"));
    let missing = load_query(dir.path().join("missing.json")).unwrap_err().to_string();
    assert!(missing.contains("not found"), "{missing}");
}

#[test]
fn bad_condition_values() {
    assert!(Condition::new("X", "c", "~", "1").unwrap_err().to_string().contains("unknown operator"));
    let check =
        |op: &str, v: &str| Condition::new("X", "c", op, v).unwrap().check_value().unwrap_err().to_string();
    assert!(check(">", "abc").contains("needs a number or date"));
    assert!(check("contains", "").contains("needs a value"));
    assert!(check("in", " , ").contains("separated by commas"));
    assert!(Condition::new("X", "c", "is blank", "").unwrap().check_value().is_ok());
    assert!(Condition::new("*", "x", "=", "1").unwrap().with_color("red").is_err());
}

fn result(mode: &str) -> wbquery_core::QueryResult {
    let q = Query {
        sources: two_sources(),
        mode: mode.into(),
        r#where: Group::and(vec![cond("L", "Status", "=", "Pass")]),
        outputs: vec![("L".into(), vec!["Lot".into(), "Made".into()]), ("R".into(), vec!["Value".into()])],
        ..Query::default()
    };
    run_query(&q, None).unwrap()
}

fn sheet_rows(path: &Path, sheet: &str) -> Vec<Vec<Data>> {
    let mut wb = open_workbook_auto(path).unwrap();
    let range = wb.worksheet_range(sheet).unwrap();
    range.rows().map(|r| r.to_vec()).collect()
}

#[test]
fn xlsx_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_xlsx(&result("independent"), dir.path().join("out")).unwrap();
    assert_eq!(path.extension().unwrap(), "xlsx");
    let wb = open_workbook_auto(&path).unwrap();
    assert_eq!(wb.sheet_names(), ["Query", "All results", "L", "R"]);
    let lots = sheet_rows(&path, "L");
    let header: Vec<String> = lots[0].iter().map(|d| d.to_string()).collect();
    assert_eq!(header, ["File", "Sheet", "Excel row", "Lot", "Made"]);
    assert_eq!(lots.len(), 4);
    let query = sheet_rows(&path, "Query");
    assert!(query.iter().any(|r| r[0] == "Conditions" && r[1] == "L › Status = 'Pass'"));
    assert!(query.iter().any(|r| r[0] == "Rows out: L" && r[1] == Data::Float(3.0)));
    assert!(query.iter().any(|r| r[0] == "L" && r[5].to_string().len() == 64));
}

#[test]
fn xlsx_join_and_warnings() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_xlsx(&result("join"), dir.path().join("join.xlsx")).unwrap();
    let wb = open_workbook_auto(&path).unwrap();
    assert_eq!(wb.sheet_names(), ["Query", "Joined"]);
    assert!(sheet_rows(&path, "Query").iter().any(|r| r[0] == "Warning"));
}

#[test]
fn html_file() {
    let dir = tempfile::tempdir().unwrap();
    let res = result("independent");
    let text = render_html(&res, "");
    assert!(text.contains("L › Status = &#x27;Pass&#x27;"));
    assert!(text.contains("2024-01-05") && text.contains("<h2>L") && text.contains("(3 rows)"));
    let path = write_html(&res, dir.path().join("r")).unwrap();
    assert_eq!(path.extension().unwrap(), "html");
    assert!(std::fs::metadata(&path).unwrap().len() > 1000);
}

#[test]
fn name_and_description_in_exports() {
    let dir = tempfile::tempdir().unwrap();
    let mut res = result("independent");
    res.query.name = "Passing lots".into();
    res.query.description = "Why: <monthly> review".into();
    let text = render_html(&res, "");
    assert!(text.contains("<title>Passing lots</title>") && text.contains("Why: &lt;monthly&gt; review"));
    let path = write_xlsx(&res, dir.path().join("out.xlsx")).unwrap();
    let rows = sheet_rows(&path, "Query");
    let first: Vec<(String, String)> =
        rows.iter().take(2).map(|r| (r[0].to_string(), r[1].to_string())).collect();
    assert_eq!(
        first,
        [("Query".into(), "Passing lots".into()), ("Description".into(), "Why: <monthly> review".into())]
    );
    res.query.name.clear();
    res.query.description.clear();
    assert!(render_html(&res, "").contains("<title>Workbook Query results</title>"));
    let plain = write_xlsx(&res, dir.path().join("b.xlsx")).unwrap();
    assert_eq!(sheet_rows(&plain, "Query")[0][0].to_string(), "Mode");
}

#[test]
fn unreadable_and_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.xlsx");
    std::fs::write(&junk, b"not a workbook").unwrap();
    let q = Query {
        sources: vec![Source::new("J", &junk.to_string_lossy(), "S", "")],
        outputs: vec![("J".into(), vec!["a".into()])],
        ..Query::default()
    };
    assert!(run_query(&q, None).unwrap_err().to_string().starts_with("J: "));
}
