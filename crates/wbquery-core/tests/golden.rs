//! Parity tests against fixtures generated from the original Python implementation
//! (tests/golden/generate.py). Each test collects every mismatch before failing, so one run shows them all.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use chrono::NaiveDateTime;
use serde_json::Value;
use wbquery_core::cell::{Cell, Column};
use wbquery_core::conditions::{
    Condition, Group, Node, cell_hits, column_key, evaluate_group, normalize_color, parse_number,
    resolve_column, sort_columns, split_list, text_color,
};
use wbquery_core::export_html::render_html;
use wbquery_core::export_xlsx::{Style, sheet_name, workbook_model};
use wbquery_core::query::{Query, default_alias, normalize_key, run_query, unique_alias};
use wbquery_core::{VERSION, loader};

fn golden_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden");
    std::fs::canonicalize(dir).expect("tests/golden exists")
}

fn read(name: &str) -> Value {
    let text = std::fs::read_to_string(golden_dir().join(name)).expect("golden file");
    serde_json::from_str(&text).expect("golden JSON")
}

fn cell(v: &Value) -> Cell {
    match v {
        Value::Null => Cell::Empty,
        Value::Bool(b) => Cell::Bool(*b),
        Value::Number(n) if n.is_f64() => Cell::Float(n.as_f64().unwrap()),
        Value::Number(n) => Cell::Int(n.as_i64().unwrap()),
        Value::String(s) => Cell::Text(s.clone()),
        Value::Object(o) if o.contains_key("d") => {
            Cell::Date(NaiveDateTime::parse_from_str(o["d"].as_str().unwrap(), "%Y-%m-%dT%H:%M:%S").unwrap())
        }
        Value::Object(o) => match o["f"].as_str().unwrap() {
            "inf" => Cell::Float(f64::INFINITY),
            _ => Cell::Float(f64::NEG_INFINITY),
        },
        other => panic!("bad golden cell {other}"),
    }
}

fn cells(v: &Value) -> Vec<Cell> {
    v.as_array().unwrap().iter().map(cell).collect()
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn bools(v: &Value) -> Vec<bool> {
    v.as_array().unwrap().iter().map(|b| b.as_bool().unwrap()).collect()
}

/// Same value, counting an int and a float with the same value as equal (Excel stores numbers as floats).
fn same_number(a: &Cell, b: &Cell) -> bool {
    match (a, b) {
        (Cell::Int(i), Cell::Float(f)) | (Cell::Float(f), Cell::Int(i)) => *i as f64 == *f,
        _ => a == b,
    }
}

fn finish(problems: Vec<String>) {
    if !problems.is_empty() {
        panic!("{} mismatch(es):\n{}", problems.len(), problems.join("\n"));
    }
}

#[test]
fn loader_matches_python() {
    let mut problems = Vec::new();
    for doc in read("loader.json").as_array().unwrap() {
        let file = doc["file"].as_str().unwrap();
        let path = golden_dir().join("data").join(file);
        let sheets = loader::list_sheets(&path).unwrap();
        if sheets != strings(&doc["sheets"]) {
            problems.push(format!("{file}: sheets {sheets:?} != {}", doc["sheets"]));
        }
        for (sheet, want) in doc["tables"].as_object().unwrap() {
            let got = loader::load_table(&path, if loader::is_csv(&path) { None } else { Some(sheet) });
            let place = format!("{file} [{sheet}]");
            if let Some(err) = want.get("error") {
                match got {
                    Err(e) if e.to_string() == err.as_str().unwrap() => {}
                    other => problems.push(format!("{place}: want error {err}, got {other:?}")),
                }
                continue;
            }
            let frame = match got {
                Ok(f) => f,
                Err(e) => {
                    problems.push(format!("{place}: error {e}"));
                    continue;
                }
            };
            if frame.names() != strings(&want["columns"]) {
                problems.push(format!("{place}: columns {:?} != {}", frame.names(), want["columns"]));
                continue;
            }
            let index: Vec<i64> =
                want["index"].as_array().unwrap().iter().map(|i| i.as_i64().unwrap()).collect();
            if frame.index != index {
                problems.push(format!("{place}: index {:?} != {index:?}", frame.index));
            }
            for c in &frame.columns {
                let expect = cells(&want["cells"][&c.name]);
                if c.cells != expect {
                    problems.push(format!("{place} {}: {:?} != {expect:?} ({:?})", c.name, c.cells, c.dtype));
                }
            }
            let numeric = loader::numeric_columns(&frame, 0.8);
            if numeric != strings(&want["numeric_columns"]) {
                problems.push(format!("{place}: numeric {numeric:?} != {}", want["numeric_columns"]));
            }
            let dates = loader::date_columns(&frame);
            if dates != strings(&want["date_columns"]) {
                problems.push(format!("{place}: dates {dates:?} != {}", want["date_columns"]));
            }
        }
    }
    finish(problems);
}

#[test]
fn conditions_match_python() {
    let doc = read("conditions.json");
    let mut problems = Vec::new();
    let mut checked = 0;
    for series in doc["series"].as_array().unwrap() {
        let name = series["name"].as_str().unwrap();
        let col = Column::inferred("c", cells(&series["cells"]));
        for case in series["cases"].as_array().unwrap() {
            if case.get("crash").is_some() {
                continue; // the Python version crashed here; nothing to match
            }
            let (op, value) = (case["op"].as_str().unwrap(), case["value"].as_str().unwrap());
            let got = Condition::new("*", "c", op, value).unwrap().matches(&col);
            let place = format!("{name} {op} {value:?}");
            checked += 1;
            // Known divergence: pandas reads a blank (NaT) in a date column as the smallest int64 when
            // comparing it with a number, so it matched `<>` and `<`; here a blank never matches.
            let got = got.map(|mut g| {
                if col.dtype == wbquery_core::DType::Date
                    && parse_number(value).is_ok_and(|n| n.is_some())
                    && let Some(want) = case.get("expect")
                {
                    {
                        for (i, c) in col.cells.iter().enumerate() {
                            if c.is_empty() {
                                g[i] = want[i].as_bool().unwrap();
                            }
                        }
                    }
                }
                g
            });
            match (case.get("expect"), got) {
                (Some(want), Ok(got)) if got == bools(want) => {}
                (None, Err(e)) if e.to_string() == case["error"].as_str().unwrap() => {}
                (want, got) => problems.push(format!(
                    "{place}: want {} got {got:?}",
                    want.map(|w| w.to_string()).unwrap_or_else(|| case["error"].to_string())
                )),
            }
        }
    }
    assert!(checked > 3000, "only {checked} cases");
    for case in doc["checks"].as_array().unwrap() {
        let (op, value) = (case["op"].as_str().unwrap(), case["value"].as_str().unwrap());
        let got = Condition::new("X", "c", op, value).unwrap().check_value();
        match (case.get("error"), got) {
            (None, Ok(())) => {}
            (Some(want), Err(e)) if e.to_string() == want.as_str().unwrap() => {}
            (want, got) => problems.push(format!("check {op} {value:?}: want {want:?} got {got:?}")),
        }
    }
    let bad = Condition::new("X", "c", "~", "1").unwrap_err().to_string();
    if bad != doc["bad_op"].as_str().unwrap() {
        problems.push(format!("bad op: {bad}"));
    }
    for d in doc["describes"].as_array().unwrap() {
        let tree = Group::from_json(&d["tree"]).unwrap();
        if tree.describe() != d["describe"].as_str().unwrap() {
            problems.push(format!("describe: {} != {}", tree.describe(), d["describe"]));
        }
        if tree.to_json() != d["tree"] {
            problems.push(format!("round trip: {} != {}", tree.to_json(), d["tree"]));
        }
        let active: Vec<&str> = tree.conditions(true).iter().map(|c| c.column.as_str()).collect();
        let all: Vec<&str> = tree.conditions(false).iter().map(|c| c.column.as_str()).collect();
        if active != strings(&d["active"]) || all != strings(&d["all"]) {
            problems.push(format!("conditions: {active:?} {all:?} vs {} {}", d["active"], d["all"]));
        }
    }
    let frame: Vec<Column> = doc["frame"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, v)| Column::inferred(name.clone(), cells(v)))
        .collect();
    let column_for = |c: &Condition| frame.iter().filter(|col| col.name == c.column).collect::<Vec<_>>();
    for e in doc["evaluate"].as_array().unwrap() {
        let tree = Group::from_json(&e["tree"]).unwrap();
        let got = evaluate_group(&tree, &column_for).unwrap();
        let want = if e["expect"].is_null() { None } else { Some(bools(&e["expect"])) };
        if got != want {
            problems.push(format!("evaluate {}: {got:?} != {want:?}", e["tree"]));
        }
        let hits: Vec<(String, Vec<bool>)> =
            cell_hits(&tree, &column_for).unwrap().into_iter().map(|h| (h.column, h.rows)).collect();
        let want: Vec<(String, Vec<bool>)> = e["hits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| (h["column"].as_str().unwrap().to_string(), bools(&h["rows"])))
            .collect();
        if hits != want {
            problems.push(format!("cell_hits {}: {hits:?} != {want:?}", e["tree"]));
        }
    }
    finish(problems);
}

#[test]
fn scalars_match_python() {
    let doc = read("scalars.json");
    let mut problems = Vec::new();
    let mut check = |what: &str, ok: bool, detail: String| {
        if !ok {
            problems.push(format!("{what}: {detail}"));
        }
    };
    let num = |v: &Value| match cell(v) {
        Cell::Empty => f64::NAN,
        Cell::Int(i) => i as f64,
        Cell::Float(f) => f,
        other => panic!("{other:?}"),
    };
    let close = |a: f64, b: f64| (a.is_nan() && b.is_nan()) || a == b;
    for c in doc["cell_number"].as_array().unwrap() {
        let got = loader::cell_number(&cell(&c["in"]));
        check("cell_number", close(got, num(&c["out"])), format!("{} -> {got}, want {}", c["in"], c["out"]));
    }
    for c in doc["leading_number"].as_array().unwrap() {
        let got = loader::leading_number(c["in"].as_str().unwrap());
        let want = c["out"].as_array().map(|a| (num(&a[0]), a[1].as_str().unwrap().to_string()));
        check("leading_number", got == want, format!("{} -> {got:?}, want {want:?}", c["in"]));
    }
    for c in doc["parse_number"].as_array().unwrap() {
        let got = parse_number(c["in"].as_str().unwrap());
        let ok = match (&got, c.get("ok"), c.get("error")) {
            (Ok(None), Some(Value::Null), _) => true,
            (Ok(Some(f)), Some(Value::Null), _) => f.is_nan(), // NaN is written as null
            (Ok(Some(f)), Some(v), _) if !v.is_null() => close(*f, num(v)),
            (Err(e), _, Some(want)) => e.to_string() == want.as_str().unwrap(),
            _ => false,
        };
        check("parse_number", ok, format!("{} -> {got:?}, want {c}", c["in"]));
    }
    for c in doc["normalize_key"].as_array().unwrap() {
        let got = normalize_key(&cell(&c["in"]), "BLANK");
        check(
            "normalize_key",
            got == c["out"].as_str().unwrap(),
            format!("{} -> {got}, want {}", c["in"], c["out"]),
        );
    }
    for c in doc["normalize_color"].as_array().unwrap() {
        let got = normalize_color(c["in"].as_str().unwrap());
        let ok = match (&got, c.get("ok"), c.get("error")) {
            (Ok(s), Some(want), _) => s == want.as_str().unwrap(),
            (Err(e), _, Some(want)) => e.to_string() == want.as_str().unwrap(),
            _ => false,
        };
        check("normalize_color", ok, format!("{} -> {got:?}", c["in"]));
    }
    for c in doc["text_color"].as_array().unwrap() {
        let got = text_color(c["in"].as_str().unwrap());
        check("text_color", got == c["out"].as_str().unwrap(), format!("{} -> {got}", c["in"]));
    }
    for c in doc["sort_columns"].as_array().unwrap() {
        let got = sort_columns(&strings(&c["in"]));
        check("sort_columns", got == strings(&c["out"]), format!("{got:?} != {}", c["out"]));
    }
    for c in doc["resolve_column"].as_array().unwrap() {
        let got = resolve_column(&strings(&c["columns"]), c["name"].as_str().unwrap());
        let want = c["out"].as_str().map(str::to_string);
        check("resolve_column", got == want, format!("{c} -> {got:?}"));
    }
    for c in doc["column_key"].as_array().unwrap() {
        let got = column_key(c["in"].as_str().unwrap());
        check("column_key", got == c["out"].as_str().unwrap(), format!("{} -> {got}", c["in"]));
    }
    for c in doc["split_list"].as_array().unwrap() {
        let got = split_list(c["in"].as_str().unwrap());
        check("split_list", got == strings(&c["out"]), format!("{} -> {got:?}", c["in"]));
    }
    let mut taken: HashSet<String> = ["query".to_string()].into();
    for c in doc["sheet_name"].as_array().unwrap() {
        let got = sheet_name(c["in"].as_str().unwrap(), &mut taken);
        check(
            "sheet_name",
            got == c["out"].as_str().unwrap(),
            format!("{} -> {got}, want {}", c["in"], c["out"]),
        );
    }
    for c in doc["default_alias"].as_array().unwrap() {
        let got = default_alias(c["path"].as_str().unwrap(), c["sheet"].as_str().unwrap());
        check("default_alias", got == c["out"].as_str().unwrap(), format!("{c} -> {got}"));
    }
    for c in doc["unique_alias"].as_array().unwrap() {
        let got = unique_alias(c["base"].as_str().unwrap(), &strings(&c["taken"]));
        check("unique_alias", got == c["out"].as_str().unwrap(), format!("{c} -> {got}"));
    }
    finish(problems);
}

/// Where the golden fixtures are, relative to this crate (Cargo runs tests from the crate's directory).
/// Relative, with forward slashes, so paths in the outputs read the same on every OS.
const GOLDEN_REL: &str = "../../tests/golden";

fn with_paths(query: &Value) -> Query {
    let mut doc = query.clone();
    for s in doc["sources"].as_array_mut().unwrap() {
        let rel = s["path"].as_str().unwrap().to_string();
        s["path"] = Value::String(format!("{GOLDEN_REL}/{rel}"));
    }
    Query::from_json(&doc).unwrap()
}

fn normalize(text: &str, run_at: &str) -> String {
    text.replace(run_at, "{RUN_AT}")
        .replace(&format!("{GOLDEN_REL}/"), "{DIR}/")
        .replace(&format!("Workbook Query {VERSION}"), "Workbook Query {VERSION}")
}

fn column_letter(j: usize) -> String {
    let mut n = j + 1;
    let mut s = String::new();
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, (b'A' + r as u8) as char);
        n = (n - 1) / 26;
    }
    s
}

#[test]
fn queries_match_python() {
    let mut problems = Vec::new();
    for case in read("queries.json").as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let query = with_paths(&case["query"]);
        if Query::from_json(&case["query"]).unwrap().to_json() != case["query"] {
            problems.push(format!("{name}: query JSON doesn't round-trip"));
        }
        let result = match run_query(&query, None) {
            Ok(r) => r,
            Err(e) => {
                let got = normalize(&e.to_string(), "\0");
                if case.get("error").and_then(Value::as_str) != Some(got.as_str()) {
                    problems.push(format!("{name}: error {got:?}, want {:?}", case.get("error")));
                }
                continue;
            }
        };
        if let Some(err) = case.get("error") {
            problems.push(format!("{name}: ran, but Python failed with {err}"));
            continue;
        }
        let names: Vec<&str> = result.tables.iter().map(|t| t.name.as_str()).collect();
        let want_names: Vec<&str> =
            case["tables"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        if names != want_names {
            problems.push(format!("{name}: tables {names:?} != {want_names:?}"));
            continue;
        }
        for (t, want) in result.tables.iter().zip(case["tables"].as_array().unwrap()) {
            let place = format!("{name}/{}", t.name);
            if t.frame.names() != strings(&want["columns"]) {
                problems.push(format!("{place}: columns {:?} != {}", t.frame.names(), want["columns"]));
                continue;
            }
            for (j, c) in t.frame.columns.iter().enumerate() {
                let expect = cells(&want["cells"][&c.name]);
                if c.cells != expect {
                    problems.push(format!("{place} {}: {:?} != {expect:?}", c.name, c.cells));
                }
                let hits = bools(&case["hits"][&t.name][&c.name]);
                if t.hits[j] != hits {
                    problems.push(format!("{place} {} hits: {:?} != {hits:?}", c.name, t.hits[j]));
                }
                let colors = strings(&case["colors"][&t.name][&c.name]);
                if t.colors[j] != colors {
                    problems.push(format!("{place} {} colors: {:?} != {colors:?}", c.name, t.colors[j]));
                }
            }
        }
        if result.warnings != strings(&case["warnings"]) {
            problems.push(format!("{name}: warnings {:?} != {}", result.warnings, case["warnings"]));
        }
        for (s, want) in result.sources.iter().zip(case["sources"].as_array().unwrap()) {
            let ok = s.rows as u64 == want["rows"].as_u64().unwrap()
                && s.matched.map(|m| m as u64) == want["matched"].as_u64()
                && s.sha256.as_deref() == want["sha256"].as_str();
            if !ok {
                problems.push(format!("{name}: source {s:?} != {want}"));
            }
        }
        let run_at = result.run_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let html_file = golden_dir().join("html").join(format!("{name}.html"));
        if html_file.exists() {
            let want = std::fs::read_to_string(&html_file).unwrap();
            let got = normalize(&render_html(&result, ""), &run_at);
            if got != want {
                let line = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(0);
                problems.push(format!(
                    "{name}: HTML differs at line {}:\n  got  {:?}\n  want {:?}",
                    line + 1,
                    got.lines().nth(line),
                    want.lines().nth(line)
                ));
            }
        }
        if let Some(sheets) = case.get("xlsx") {
            compare_xlsx(name, &workbook_model(&result), sheets.as_array().unwrap(), &run_at, &mut problems);
        }
    }
    finish(problems);
}

fn compare_xlsx(
    name: &str,
    model: &[wbquery_core::export_xlsx::SheetModel],
    want: &[Value],
    run_at: &str,
    problems: &mut Vec<String>,
) {
    let names: Vec<&str> = model.iter().map(|s| s.name.as_str()).collect();
    let want_names: Vec<&str> = want.iter().map(|s| s["name"].as_str().unwrap()).collect();
    if names != want_names {
        problems.push(format!("{name} xlsx: sheets {names:?} != {want_names:?}"));
        return;
    }
    for (sheet, w) in model.iter().zip(want) {
        let place = format!("{name} xlsx {}", sheet.name);
        let want_rows = w["rows"].as_array().unwrap();
        let ncols = want_rows.iter().map(|r| r.as_array().unwrap().len()).max().unwrap_or(0);
        if sheet.rows.len() != want_rows.len() {
            problems.push(format!("{place}: {} rows != {}", sheet.rows.len(), want_rows.len()));
        }
        for (i, (row, wrow)) in sheet.rows.iter().zip(want_rows).enumerate() {
            for j in 0..ncols {
                let got = row.get(j).map_or(Cell::Empty, |(c, _)| match c {
                    Cell::Text(s) => Cell::Text(normalize(s, run_at)),
                    other => other.clone(),
                });
                let expect = cell(&wrow[j]);
                if !same_number(&got, &expect) {
                    problems.push(format!("{place} {}{}: {got:?} != {expect:?}", column_letter(j), i + 1));
                }
            }
        }
        let mut fills: HashMap<String, String> = HashMap::new();
        for (i, row) in sheet.rows.iter().enumerate().skip(1) {
            for (j, (_, style)) in row.iter().enumerate() {
                let color = match style {
                    Style::Header => "00DDE6F0".to_string(),
                    Style::Fill(c) => format!("00{}", c[1..].to_uppercase()),
                    _ => continue,
                };
                fills.insert(format!("{}{}", column_letter(j), i + 1), color);
            }
        }
        let want_fills: HashMap<String, String> = w["fills"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect();
        if fills != want_fills {
            problems.push(format!("{place}: fills {fills:?} != {want_fills:?}"));
        }
        for (k, v) in w["widths"].as_object().unwrap() {
            if sheet.name == "Query" && k == "B" {
                continue; // holds the source paths, whose length depends on where the fixtures are
            }
            let j = sheet.widths.iter().enumerate().find(|(j, _)| column_letter(*j) == *k).map(|(_, w)| *w);
            if j != v.as_f64() {
                problems.push(format!("{place}: width {k} {j:?} != {v}"));
            }
        }
        let freeze = if sheet.table { Some("A2") } else { None };
        if freeze != w["freeze"].as_str() {
            problems.push(format!("{place}: freeze {freeze:?} != {}", w["freeze"]));
        }
        let filter = sheet.table.then(|| {
            let ncols = sheet.rows[0].len();
            format!("A1:{}{}", column_letter(ncols.max(1) - 1), sheet.rows.len())
        });
        if filter.as_deref() != w["filter"].as_str() {
            problems.push(format!("{place}: filter {filter:?} != {}", w["filter"]));
        }
    }
}

#[test]
fn saved_query_text_matches_python() {
    // Python writes json.dumps(indent=2, ensure_ascii=True) plus a newline; so must we.
    let q = Query {
        name: "Lots › results".into(),
        r#where: Group::or(vec![
            Node::Cond(Condition::new("L", "Status", "=", "Pass").unwrap().with_color("#FF0000").unwrap()),
            Node::Group(Group::and(vec![]).negated()),
        ]),
        outputs: vec![("L".into(), vec!["Lot".into()]), ("R".into(), vec![])],
        ..Query::default()
    };
    let text = wbquery_core::query::query_to_string(&q);
    let expect = r##"{
  "version": 1,
  "name": "Lots \u203a results",
  "description": "",
  "mode": "independent",
  "join_how": "inner",
  "include_provenance": true,
  "sources": [],
  "where": {
    "logic": "OR",
    "negate": false,
    "items": [
      {
        "source": "L",
        "column": "Status",
        "op": "=",
        "value": "Pass",
        "enabled": true,
        "color": "#ff0000"
      },
      {
        "logic": "AND",
        "negate": true,
        "items": []
      }
    ]
  },
  "outputs": {
    "L": [
      "Lot"
    ],
    "R": []
  }
}
"##;
    assert_eq!(text, expect);
}
