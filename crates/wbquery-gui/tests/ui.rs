//! The real window, driven headless: click buttons and check what happens. Set WBQ_UI_DIR to also
//! render screenshots there (needs a GPU or a software Vulkan/GL driver).

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use wbquery_core::conditions::{Condition, Group, Node};
use wbquery_gui::{AppState, WbqApp};

fn data(name: &str) -> PathBuf {
    std::fs::canonicalize(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/data").join(name),
    )
    .unwrap()
}

fn app() -> WbqApp {
    let mut app = WbqApp::new(AppState::default());
    app.open_paths(&[data("em.xlsx"), data("results.xlsx")]);
    let s = &mut app.state;
    s.conditions = Group::or(vec![Node::Cond(
        Condition::new("*", "Hand 1", "<>", "0").unwrap().with_color("#ffd54f").unwrap(),
    )]);
    app
}

fn harness(app: WbqApp) -> Harness<'static, WbqApp> {
    Harness::builder()
        .with_size([1500.0, 900.0])
        .with_context_setup(wbquery_gui::setup_fonts)
        .build_ui_state(|ui, app: &mut WbqApp| app.draw(ui), app)
}

#[test]
fn run_button_shows_results() {
    let mut h = harness(app());
    h.run();
    h.get_by_label("Run query").click();
    h.run();
    let s = &h.state().state;
    assert!(s.errors.is_empty(), "{:?}", s.errors);
    let names: Vec<String> = s.result.as_ref().unwrap().tables.iter().map(|t| t.name.clone()).collect();
    assert_eq!(names, ["All results", "em/Sheet", "results/Results"]);
    h.get_by_label("em/Sheet (1)");
}

#[test]
fn condition_buttons_edit_the_tree() {
    let mut h = harness(app());
    h.run();
    h.get_by_label("Add condition").click();
    h.run();
    assert_eq!(h.state().state.conditions.items.len(), 2);
    h.get_by_label("Add group").click();
    h.run();
    h.get_by_label("Duplicate").click();
    h.run();
    assert_eq!(h.state().state.conditions.items.len(), 4);
    h.get_by_label("Remove").click();
    h.run();
    assert_eq!(h.state().state.conditions.items.len(), 3);
}

fn first_condition(h: &Harness<'_, WbqApp>) -> Condition {
    h.state().state.conditions.conditions(false)[0].clone()
}

/// The widgets on a condition row and a group row respond (the row's click-to-select area used to sit on
/// top of them and swallow every click).
#[test]
fn condition_row_widgets_respond() {
    let mut h = harness(app());
    h.run();
    // Operator dropdown: open it and pick "=".
    h.get_by_label("Operator: <>").click();
    h.run();
    h.get_all_by_label("=").last().unwrap().click();
    h.run();
    assert_eq!(first_condition(&h).op, "=");
    // Value box: type into it.
    let value = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.value().as_deref() == Some("0"))
        .unwrap();
    value.focus();
    h.run();
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.value().as_deref() == Some("0"))
        .unwrap()
        .type_text("5");
    h.run();
    assert!(first_condition(&h).value.contains('5'), "{:?}", first_condition(&h).value);
    // On checkbox.
    h.get_by_label("On").click();
    h.run();
    assert!(!first_condition(&h).enabled);
    // Column dropdown: open it and pick another column.
    h.get_by_label("Column: Hand 1").click();
    h.run();
    h.get_by_label("Person 2").click();
    h.run();
    assert_eq!(first_condition(&h).column, "Person 2");
    // The group's Match dropdown and NOT box.
    h.get_by_label("Match: OR").click();
    h.run();
    h.get_by_label("AND").click();
    h.run();
    assert_eq!(h.state().state.conditions.logic, "AND");
    h.get_by_label("NOT").click();
    h.run();
    assert!(h.state().state.conditions.negate);
    // Clicking a row's background still selects it.
    h.state_mut().state.selected = None;
    let row = h.get_by_label("Sheet: Any sheet").rect();
    h.hover_at(egui::pos2(row.left() - 6.0, row.center().y));
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(row.left() - 6.0, row.center().y),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.event(egui::Event::PointerButton {
        pos: egui::pos2(row.left() - 6.0, row.center().y),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run();
    assert_eq!(h.state().state.selected, Some(vec![0]));
}

#[test]
fn right_click_a_result_cell_adds_a_condition() {
    let mut h = harness(app());
    h.run();
    h.get_by_label("Run query").click();
    h.run();
    h.get_by_label("KSG").click_secondary();
    h.run();
    h.get_by_label("Add condition: em/Sheet › Person 1 = 'KSG'").click();
    h.run();
    let s = &h.state().state;
    assert_eq!(s.conditions.conditions(false).last().unwrap().describe(), "em/Sheet › Person 1 = 'KSG'");
    assert!(s.status.text.starts_with("Added condition"));
}

#[test]
fn outputs_and_chart_controls_respond() {
    let mut h = harness(app());
    h.run();
    // Open a sheet's output list and untick a column.
    h.get_by_label_contains("em/Sheet  (7 of 7)").click();
    h.run();
    h.get_by_label("Person 2").click();
    h.run();
    assert!(!h.state().state.outputs_chosen()[0].1.contains(&"Person 2".to_string()));
    h.get_by_label("None").click();
    h.run();
    assert!(h.state().state.outputs_chosen()[0].1.is_empty());
    h.get_by_label("All").click();
    h.run();
    // Charts: switch type and Y from the controls.
    h.get_by_label("Run query").click();
    h.run();
    h.get_by_label("Visualize").click();
    h.run();
    h.get_by_label("Chart: Line / run chart").click();
    h.run();
    h.get_by_label("Bar").click();
    h.run();
    assert_eq!(h.state().state.chart.kind, "bar");
    h.get_by_label_contains("Table: ").click();
    h.run();
    h.get_by_label("em/Sheet").click();
    h.run();
    assert_eq!(h.state().state.chart.table, "em/Sheet");
}

#[test]
fn color_picker_opens_and_sets_a_color() {
    let mut h = harness(app());
    h.run();
    h.get_by_label("Highlight color").click();
    h.run();
    h.get_by_label("No color").click();
    h.run();
    assert_eq!(first_condition(&h).color, "");
    h.get_by_label("Highlight color").click();
    h.run();
    h.get_by_label("Highlight #ff8a80").click();
    h.run();
    assert_eq!(first_condition(&h).color, "#ff8a80");
}

#[test]
fn pressing_a_rows_widget_selects_the_row() {
    let mut a = app();
    a.state.conditions.items.push(Node::Cond(Condition::new("*", "Person 1", "=", "KSG").unwrap()));
    let mut h = harness(a);
    h.run();
    assert_eq!(h.state().state.selected, None);
    h.get_by_label("Column: Person 1").click();
    h.run();
    assert_eq!(h.state().state.selected, Some(vec![1]));
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_label("Operator: <>").click();
    h.run();
    assert_eq!(h.state().state.selected, Some(vec![0]));
    // Choosing from the open list (drawn over the row below) keeps this row selected.
    h.get_all_by_label("contains").last().unwrap().click();
    h.run();
    assert_eq!(h.state().state.selected, Some(vec![0]));
    assert_eq!(first_condition(&h).op, "contains");
}

#[test]
fn sections_and_left_pane_collapse() {
    let mut h = harness(app());
    h.run();
    // Each section folds away and opens again.
    h.get_by_label("Add condition");
    h.get_by_label("Conditions").click();
    h.run();
    assert!(h.query_by_label("Add condition").is_none());
    h.get_by_label("Conditions").click();
    h.run();
    h.get_by_label("Add condition");
    h.get_by_label("Workbook sheets").click();
    h.run();
    assert!(h.query_by_label("Add workbooks…").is_none());
    // The whole pane hides, leaving the results the window.
    h.get_by_label("◀").click();
    h.run();
    assert!(h.query_by_label("Add condition").is_none());
    h.get_by_label("Run query");
    h.get_by_label("▶").click();
    h.run();
    h.get_by_label("Add condition");
}

#[test]
fn errors_show_in_a_dialog() {
    let mut a = WbqApp::new(AppState::default());
    a.state.error("Something odd <> '0'");
    let mut h = harness(a);
    h.run();
    h.get_by_label("Something odd <> '0'");
    h.get_by_label("OK").click();
    h.run();
    assert!(h.state().state.errors.is_empty());
}

#[test]
fn screenshots() {
    let Some(dir) = std::env::var_os("WBQ_UI_DIR").map(PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    let mut h = Harness::builder()
        .with_size([1500.0, 900.0])
        .with_context_setup(wbquery_gui::setup_fonts)
        .wgpu()
        .build_ui_state(|ui, app: &mut WbqApp| app.draw(ui), app());
    h.run();
    h.get_by_label("Highlight color").click();
    h.run();
    h.render().unwrap().save(dir.join("picker.png")).unwrap();
    h.key_press(egui::Key::Escape);
    h.run();
    h.get_by_label("Run query").click();
    h.run();
    h.render().unwrap().save(dir.join("tables.png")).unwrap();
    {
        let c = &mut h.state_mut().state.chart;
        c.set_table("em/Sheet");
        c.set_kind("scatter");
        c.x = "Monitor Date".into();
        c.pick_y(&["Chest 1".into(), "Hand 1".into(), "Hand 1 (2)".into()]);
    }
    h.get_by_label("Visualize").click();
    h.run();
    h.render().unwrap().save(dir.join("chart.png")).unwrap();
    {
        let s = &mut h.state_mut().state;
        let person = |n: &str, color: &str| {
            let mut g = Group::and(vec![
                Node::Cond(Condition::new("*", &format!("Person {n}"), "=", "KSG").unwrap()),
                Node::Group(Group::or(vec![
                    Node::Cond(Condition::new("*", "Chest 1", "<>", "0").unwrap().with_color(color).unwrap()),
                    Node::Cond(Condition::new("*", "Hand 1", "<>", "0").unwrap().with_color(color).unwrap()),
                ])),
            ]);
            g.name = format!("Person {n}");
            g
        };
        let mut second = person("2", "#80d8ff");
        second.collapsed = true;
        s.conditions = Group::or(vec![Node::Group(person("1", "#ffd54f")), Node::Group(second)]);
        s.selected = Some(vec![0, 1, 0]);
        s.run();
    }
    h.state_mut().state.chart.set_table("em/Sheet");
    h.state_mut().state.chart.set_kind("imr");
    h.state_mut().state.chart.y = "Hand 1".into();
    h.state_mut().state.chart.redraw();
    h.run();
    h.render().unwrap().save(dir.join("imr.png")).unwrap();
}
