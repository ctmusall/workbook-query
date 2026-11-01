//! The app's behavior without a window, ported from the Python GUI smoke tests (which also called the
//! window's methods directly rather than clicking).

use std::path::{Path, PathBuf};

use wbquery_core::cell::Cell;
use wbquery_core::conditions::{Condition, Group, Node};
use wbquery_gui::app::shared_problem;
use wbquery_gui::chart_state::Message;
use wbquery_gui::state::{Added, AppState, Tick};

fn data(name: &str) -> PathBuf {
    std::fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/data").join(name),
    )
    .unwrap()
}

/// Lots (sheet Lots) and Results loaded, as in the Python fixtures.
fn loaded() -> AppState {
    let mut s = AppState::default();
    assert_eq!(s.add_workbook(&data("lots.xlsx"), Some(vec!["Lots".into()])), Added::Sheets(1));
    assert_eq!(s.add_workbook(&data("results.xlsx"), None), Added::Sheets(1));
    assert!(s.rename_source(0, "lots") && s.rename_source(1, "results"));
    s
}

fn cond(src: &str, col: &str, op: &str, value: &str) -> Node {
    Node::Cond(Condition::new(src, col, op, value).unwrap())
}

#[test]
fn starts_empty() {
    let s = AppState::default();
    assert_eq!(s.status.text, "Add workbooks to start.");
    assert!(s.title().starts_with("Workbook Query "));
}

#[test]
fn sheets_and_names() {
    let mut s = AppState::default();
    match s.add_workbook(&data("lots.xlsx"), None) {
        Added::Choose(_, names) => assert_eq!(names, ["Lots", "Owners"]),
        other => panic!("{other:?}"),
    }
    assert!(s.entries.is_empty());
    s.add_workbook(&data("lots.xlsx"), Some(vec!["Lots".into(), "Owners".into()]));
    s.add_workbook(&data("results.xlsx"), None);
    assert_eq!(s.aliases(), ["lots/Lots", "lots/Owners", "results/Results"]);
    assert_eq!(s.status.text, "3 sheets loaded. Build conditions, tick output columns, then Run query.");
    // The join key is a column another sheet also has.
    assert_eq!(s.entries[0].source.key, "Lot");
    assert_eq!(s.entries[2].source.key, "Lot ID");
    // Shared columns, and sheets with a column (ignoring case).
    assert_eq!(s.shared_columns(), [("Lot".to_string(), 2)]);
    assert_eq!(s.sheets_with("lot"), ["lots/Lots", "lots/Owners"]);
    // Rename: refused when blank or taken; conditions follow a rename.
    s.conditions = Group::and(vec![cond("lots/Lots", "Status", "=", "Pass")]);
    assert!(!s.rename_source(0, "lots/Owners"));
    assert!(s.errors.pop().unwrap().contains("already used"));
    assert!(!s.rename_source(0, "  "));
    s.errors.clear();
    assert!(s.rename_source(0, "L"));
    assert_eq!(s.conditions.conditions(false)[0].source, "L");
    assert_eq!(s.outputs[0].alias, "L");
    assert!(s.remove_source(1));
    assert_eq!(s.aliases(), ["L", "results/Results"]);
}

#[test]
fn unreadable_files_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("junk.xlsx");
    std::fs::write(&junk, b"nope").unwrap();
    let mut s = AppState::default();
    assert_eq!(s.add_workbook(&junk, None), Added::Sheets(0));
    assert!(s.errors[0].starts_with("Can't read"));
    assert!(!s.load_query(&dir.path().join("missing.json")));
    assert!(s.errors[1].contains("not found"));
    assert!(!s.run());
    assert_eq!(s.errors[2], "Add workbooks first.");
}

#[test]
fn independent_run_export_and_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = loaded();
    s.conditions =
        Group::or(vec![cond("lots", "Status", "=", "pass"), cond("results", "Value", ">=", "6e7")]);
    s.set_outputs("lots", &["Lot", "Status"]);
    s.set_outputs("results", &["Lot ID", "Value"]);
    assert!(s.run(), "{:?}", s.errors);
    let r = s.result.as_ref().unwrap();
    let names: Vec<&str> = r.tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["All results", "lots", "results"]);
    assert_eq!(s.status.text, "Matched — All results: 5 rows, lots: 3 rows, results: 2 rows");
    let xlsx = s.export("xlsx", &dir.path().join("out")).unwrap();
    assert!(xlsx.ends_with("out.xlsx") && xlsx.exists());
    assert!(s.status.text.starts_with("Excel workbook saved to ") && s.status.saved.is_some());
    assert_eq!(std::fs::canonicalize(s.save_dir()).unwrap(), std::fs::canonicalize(dir.path()).unwrap());
    let html = s.export("html", &dir.path().join("out")).unwrap();
    assert!(html.ends_with("out.html"));
    // A failing export says so.
    assert!(s.export("xlsx", &dir.path().join("no/such/dir/out.xlsx")).is_none());
    assert!(s.errors.pop().unwrap().starts_with("Couldn't export"));
}

#[test]
fn join_save_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = loaded();
    s.join = true;
    s.name = "Lots vs results".into();
    s.conditions =
        Group::and(vec![cond("lots", "Status", "=", "Pass"), cond("results", "Value", ">", "2e7")]);
    s.set_outputs("lots", &["Status"]);
    s.set_outputs("results", &["Value"]);
    assert!(s.run(), "{:?}", s.errors);
    assert_eq!(s.result.as_ref().unwrap().tables[0].name, "Joined");
    assert_eq!(s.query_file_name(), "Lots_vs_results.json");
    let path = s.save_query(&dir.path().join("q.json")).unwrap();
    let mut t = AppState::default();
    assert!(t.load_query(&path));
    assert_eq!(t.build_query(), s.build_query());
    assert_eq!(t.status.text, "Loaded Lots vs results with 2 sheets.");
    assert_eq!(t.title(), format!("Lots vs results — Workbook Query {}", wbquery_core::VERSION));
}

#[test]
fn condition_editing() {
    let mut s = loaded();
    // Several sheets: a new condition tests any sheet, on a column they share.
    let c = s.add_condition(None);
    assert_eq!((c.source.as_str(), c.column.as_str()), ("*", "Lot"));
    assert_eq!(s.selected, Some(vec![0]));
    // With a condition selected, the next copies its sheet and column.
    s.conditions = Group::and(vec![cond("results", "Value", ">", "1")]);
    s.selected = Some(vec![0]);
    let c = s.add_condition(None);
    assert_eq!((c.source.as_str(), c.column.as_str(), c.op.as_str()), ("results", "Value", "="));
    assert!(s.duplicate());
    assert_eq!(s.conditions.items.len(), 3);
    assert_eq!(s.selected, Some(vec![2]));
    assert!(s.move_selected(-1));
    assert_eq!(s.selected, Some(vec![1]));
    assert!(!s.move_selected(-5));
    s.add_group("OR");
    assert_eq!(s.selected, Some(vec![3]));
    s.add_condition(None); // into the selected group
    assert!(matches!(&s.conditions.items[3], Node::Group(g) if g.items.len() == 1));
    assert!(s.remove());
    assert!(s.remove());
    assert_eq!(s.conditions.items.len(), 3);
    s.selected = None;
    assert!(!s.remove());
}

#[test]
fn move_out_of_and_into_groups() {
    let mut s = loaded();
    // [A, (B, (C))]: a group nested in a group.
    s.conditions = Group::and(vec![
        cond("lots", "Status", "=", "A"),
        Node::Group(Group::or(vec![
            cond("lots", "Status", "=", "B"),
            Node::Group(Group::and(vec![cond("lots", "Status", "=", "C")])),
        ])),
    ]);
    let values =
        |s: &AppState| s.conditions.conditions(false).iter().map(|c| c.value.clone()).collect::<Vec<_>>();
    // The inner group out of its parent: now [A, (B), (C)].
    s.selected = Some(vec![1, 1]);
    assert!(s.can_move_out() && s.move_out());
    assert_eq!(s.selected, Some(vec![2]));
    assert!(matches!(&s.conditions.items[2], Node::Group(g) if g.items.len() == 1));
    assert!(!s.can_move_out()); // already at the top
    assert!(!s.move_out());
    // And back in, into the group above it: [A, (B, (C))].
    assert!(s.can_move_in() && s.move_in());
    assert_eq!(s.selected, Some(vec![1, 1]));
    assert_eq!(values(&s), ["A", "B", "C"]);
    // A condition into the group above it; nothing above A to move into.
    s.selected = Some(vec![0]);
    assert!(!s.can_move_in() && !s.move_in());
    s.selected = Some(vec![1, 1, 0]);
    assert!(s.move_out());
    assert_eq!(s.selected, Some(vec![1, 2]));
    assert_eq!(values(&s), ["A", "B", "C"]);
}

#[test]
fn shared_columns_and_add_for_column() {
    let mut s = loaded();
    let added = s.add_shared_many(&["Lot".into()], "in", "A1, A2", Some("OR"), "#FF0000");
    assert_eq!(added.len(), 1);
    let Node::Group(g) = &s.conditions.items[0] else { panic!() };
    assert_eq!(g.logic, "OR");
    assert_eq!(g.conditions(false)[0].color, "#ff0000");
    assert_eq!(s.selected, Some(vec![0, 0]));
    // The dialog's checks: a value that doesn't fit the operator keeps it open.
    assert_eq!(shared_problem(&[], "=", ""), "Tick at least one column.");
    assert_eq!(shared_problem(&["Lot".into()], ">", ""), "");
    assert!(shared_problem(&["Lot".into()], ">", "abc").contains("needs a number or date"));
    assert_eq!(shared_problem(&["Lot".into()], "is blank", "x"), "");
}

#[test]
fn suggestions_and_result_cells() {
    let mut s = loaded();
    assert_eq!(s.values_for("lots", "status"), ["Fail", "Pass", "pass"]);
    let values = s.values_for("*", "Lot");
    assert_eq!(values[0], "101"); // numbers first
    assert!(values.contains(&"A1".to_string()));
    s.set_outputs("lots", &["Lot", "Status"]);
    s.set_outputs("results", &[]);
    assert!(s.run());
    let conds = s.cell_conditions("Status", &Cell::Text("Pass".into()));
    assert_eq!(conds[0].describe(), "lots › Status = 'Pass'");
    assert_eq!(conds[1].op, "<>");
    let blank = s.cell_conditions("Lot", &Cell::Empty);
    assert_eq!(blank[0].describe(), "lots › Lot is blank"); // only lots has a Lot column
    assert!(s.cell_conditions("Excel row", &Cell::Int(2)).is_empty());
    assert_eq!(s.cell_conditions("Lot", &Cell::Float(5.0))[0].value, "5");
    s.add_condition_from_result(conds[0].clone());
    assert!(s.status.text.starts_with("Added condition: lots › Status = 'Pass'"));
    // Join mode: columns are alias.column.
    s.join = true;
    s.set_outputs("results", &["Value"]);
    assert!(s.run(), "{:?}", s.errors);
    let c = s.cell_conditions("results.Value", &Cell::Float(5e7));
    assert_eq!(c[0].describe(), "results › Value = '50000000'");
    assert!(s.cell_conditions("Key", &Cell::Text("A1".into())).is_empty());
}

#[test]
fn outputs_for_all_sheets() {
    let mut s = AppState::default();
    s.add_workbook(&data("lots.xlsx"), Some(vec!["Lots".into(), "Owners".into()]));
    s.set_outputs("lots/Owners", &["Owner"]);
    let states = s.column_states();
    let lot = states.iter().find(|r| r.0 == "Lot").unwrap();
    assert_eq!(lot.1, ["lots/Lots", "lots/Owners"]);
    assert_eq!(lot.2, Tick::Some);
    s.apply_column_states(&[("Lot".into(), Tick::None), ("Owner".into(), Tick::Some)]);
    assert_eq!(s.outputs_chosen()[1].1, ["Owner"]);
    assert!(!s.outputs_chosen()[0].1.contains(&"Lot".to_string()));
    s.move_output("lots/Lots", 3, 0);
    assert_eq!(s.outputs[0].columns[0].0, "Notes");
}

#[test]
fn table_sorting_puts_blanks_first() {
    let mut s = loaded();
    s.set_outputs("lots", &["Lot", "Notes"]);
    s.set_outputs("results", &[]);
    assert!(s.run());
    let notes = 4; // File, Sheet, Excel row, Lot, Notes
    s.toggle_sort(0, notes);
    let col = &s.result.as_ref().unwrap().tables[0].frame.columns[notes];
    let order = s.table_order(0);
    let shown: Vec<String> = order.iter().map(|&i| col.cells[i].display()).collect();
    assert_eq!(shown, ["", "", "ok", "orphan", "retest"]);
    s.toggle_sort(0, notes);
    assert_eq!(s.table_order(0)[0], order[4]);
    s.toggle_sort(0, notes);
    assert_eq!(s.table_order(0), [0, 1, 2, 3, 4]);
}

#[test]
fn charts_follow_results() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = loaded();
    s.set_outputs("lots", &[]);
    s.set_outputs("results", &["Lot ID", "Value", "Pct"]);
    assert!(s.run());
    let c = &mut s.chart;
    assert_eq!(c.tables, ["results"]);
    assert_eq!(c.y, "Value"); // the first numeric column
    for k in wbquery_charts::KINDS {
        c.set_kind(k.key);
        assert!(c.redraw(), "{}: {:?}", k.key, c.message);
    }
    c.set_kind("line");
    assert!(c.pick_y(&["Value".into(), "Pct".into()]));
    assert_eq!(c.spec().y_columns(), ["Value", "Pct"]);
    for ext in ["png", "svg", "pdf"] {
        let p = c.save(&dir.path().join(format!("c.{ext}"))).unwrap();
        assert!(p.exists());
    }
    assert!(c.save(&dir.path().join("plain")).unwrap().ends_with("plain.png"));
    assert!(matches!(c.message, Message::Saved(_)));
    // The kind and columns stay when the query is run again.
    c.set_kind("scatter");
    assert!(s.run());
    assert_eq!(s.chart.kind, "scatter");
    // An impossible chart says why.
    s.chart.set_kind("histogram");
    s.chart.y = String::new();
    assert!(!s.chart.redraw());
    assert!(matches!(&s.chart.message, Message::Error(e) if e == "Choose a numeric column for Y"));
    s.clear_results();
    assert!(s.chart.chart.is_none());
}

#[test]
fn icon_loads() {
    assert!(wbquery_gui::icon().is_some_and(|i| i.width >= 64));
    assert!(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/icon.ico").exists());
}
