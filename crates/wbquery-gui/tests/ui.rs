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
