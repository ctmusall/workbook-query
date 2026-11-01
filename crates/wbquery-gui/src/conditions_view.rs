//! The condition tree editor: AND / OR groups (with NOT, a name, On, a highlight color, and — while
//! collapsed — one operator / value for everything inside) and conditions (sheet, column, operator,
//! value with suggestions, color, On).

use egui::{Id, RichText, Ui};
use wbquery_core::conditions::{
    ANY_SOURCE, Condition, Group, LOGIC, NO_VALUE_OPERATORS, Node, OPERATORS, sort_columns,
};

use crate::state::AppState;
use crate::widgets::{color_button, combo, search_combo, suggest_field};

pub const ANY_LABEL: &str = "Any sheet";
const INDENT: f32 = 18.0;

/// What the editor asks the window to do after drawing.
#[derive(Default)]
pub struct Events {
    pub select: Option<Vec<usize>>,
    pub run: bool,
}

/// Draw the tree (`state.conditions`).
pub fn conditions_tree(ui: &mut Ui, state: &mut AppState) -> Events {
    let mut events = Events::default();
    let mut root = std::mem::take(&mut state.conditions);
    let focus = state.focus_value.take();
    draw_group(ui, state, &mut root, &[], true, true, &focus, &mut events);
    state.conditions = root;
    events
}

fn selected(state: &AppState, path: &[usize]) -> bool {
    state.selected.as_deref() == Some(path)
}

/// Every condition under `g`, mutably.
fn all_conditions(g: &mut Group) -> Vec<&mut Condition> {
    g.conditions_mut()
}

#[allow(clippy::too_many_arguments)]
fn draw_group(
    ui: &mut Ui,
    state: &mut AppState,
    g: &mut Group,
    path: &[usize],
    root: bool,
    parent_on: bool,
    focus: &Option<Vec<usize>>,
    events: &mut Events,
) {
    let on = parent_on && (root || g.enabled);
    let is_sel = selected(state, path);
    let frame = egui::Frame::new()
        .fill(if is_sel {
            ui.visuals().selection.bg_fill.gamma_multiply(0.35)
        } else {
            egui::Color32::TRANSPARENT
        })
        .inner_margin(egui::Margin::symmetric(2, 1))
        .corner_radius(3.0);
    let row = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(INDENT * path.len() as f32);
            if !root {
                let arrow = if g.collapsed { "▸" } else { "▾" };
                if ui.add(egui::Button::new(arrow).frame(false)).on_hover_text("Collapse / expand").clicked()
                {
                    g.collapsed = !g.collapsed;
                }
            }
            ui.add_enabled_ui(on, |ui| {
                ui.label("Match");
                combo(ui, Id::unique(("logic", path)), "Match", &mut g.logic, LOGIC, 54.0);
                ui.checkbox(&mut g.negate, "NOT");
                if root {
                    return;
                }
                ui.add(egui::TextEdit::singleline(&mut g.name).hint_text("group name").desired_width(110.0))
                    .on_hover_text("A label for this group (optional)");
                if g.collapsed {
                    bulk_editor(ui, state, g, path);
                }
                let colors: std::collections::BTreeSet<String> =
                    g.conditions(false).iter().map(|c| c.color.clone()).collect();
                let shared = if colors.len() == 1 {
                    colors.iter().next().cloned().unwrap_or_default()
                } else {
                    String::new()
                };
                if let Some(c) =
                    color_button(ui, Id::unique(("group-color", path)), &shared, colors.len() > 1)
                {
                    for cond in all_conditions(g) {
                        cond.color = c.clone();
                    }
                }
            });
            if !root {
                ui.add_enabled_ui(parent_on, |ui| {
                    ui.checkbox(&mut g.enabled, "On")
                        .on_hover_text("Untick to switch the whole group off without deleting it");
                });
            }
        });
    });
    if row.response.interact(egui::Sense::click()).clicked() {
        events.select = Some(path.to_vec());
    }
    if g.collapsed && !root {
        return;
    }
    for i in 0..g.items.len() {
        let mut child = path.to_vec();
        child.push(i);
        match &mut g.items[i] {
            Node::Group(sub) => draw_group(ui, state, sub, &child, false, on, focus, events),
            Node::Cond(c) => draw_condition(ui, state, c, &child, on, focus, events),
        }
    }
}

/// One operator / value for every condition in a collapsed group (when they all share them).
fn bulk_editor(ui: &mut Ui, state: &mut AppState, g: &mut Group, path: &[usize]) {
    let conds = g.conditions(false);
    if conds.is_empty() {
        return;
    }
    let shared: std::collections::BTreeSet<(String, String)> =
        conds.iter().map(|c| (c.op.clone(), c.value.clone())).collect();
    if shared.len() != 1 {
        ui.label(RichText::new("operators / values differ — expand to edit").italics());
        return;
    }
    let (mut op, mut value) = shared.into_iter().next().expect("one");
    let mut suggestions: Vec<String> = Vec::new();
    for c in &conds {
        let src = c.source.clone();
        suggestions.extend(state.values_for(&src, &c.column));
    }
    suggestions.sort();
    suggestions.dedup();
    let n = conds.len();
    let op_changed =
        combo(ui, Id::unique(("bulk-op", path)), "Operator for the group", &mut op, OPERATORS, 96.0);
    let value_changed = ui
        .add_enabled_ui(!NO_VALUE_OPERATORS.contains(&op.as_str()), |ui| {
            suggest_field(
                ui,
                Id::unique(("bulk-value", path)),
                &mut value,
                &suggestions,
                "value for every condition",
                130.0,
            )
            .changed
        })
        .inner;
    if op_changed || value_changed {
        for c in all_conditions(g) {
            c.op = op.clone();
            c.value = value.clone();
        }
    }
    let _ = n;
}

fn draw_condition(
    ui: &mut Ui,
    state: &mut AppState,
    c: &mut Condition,
    path: &[usize],
    on: bool,
    focus: &Option<Vec<usize>>,
    events: &mut Events,
) {
    let is_sel = selected(state, path);
    let frame = egui::Frame::new()
        .fill(if is_sel {
            ui.visuals().selection.bg_fill.gamma_multiply(0.35)
        } else {
            egui::Color32::TRANSPARENT
        })
        .inner_margin(egui::Margin::symmetric(2, 1))
        .corner_radius(3.0);
    let row = frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.add_space(INDENT * path.len() as f32 + 14.0);
            ui.add_enabled_ui(on, |ui| {
                let mut source_label =
                    if c.source == ANY_SOURCE { ANY_LABEL.to_string() } else { c.source.clone() };
                let mut labels: Vec<String> = vec![ANY_LABEL.into()];
                labels.extend(state.aliases());
                if !labels.contains(&source_label) {
                    labels.push(source_label.clone());
                }
                if search_combo(
                    ui,
                    Id::unique(("source", path)),
                    "Sheet",
                    &mut source_label,
                    &labels,
                    120.0,
                    &|_| None,
                ) {
                    c.source =
                        if source_label == ANY_LABEL { ANY_SOURCE.into() } else { source_label.clone() };
                }
                let mut columns = sort_columns(&state.columns_for(&c.source));
                if !columns.contains(&c.column) {
                    columns.push(c.column.clone());
                }
                let any = c.source == ANY_SOURCE;
                let tip = |col: &str| any.then(|| format!("In: {}", state.sheets_with(col).join(", ")));
                search_combo(
                    ui,
                    Id::unique(("column", path)),
                    "Column",
                    &mut c.column,
                    &columns,
                    140.0,
                    &tip,
                );
                if combo(ui, Id::unique(("op", path)), "Operator", &mut c.op, OPERATORS, 96.0)
                    && NO_VALUE_OPERATORS.contains(&c.op.as_str())
                {
                    c.value.clear();
                }
                let suggestions = state.values_for(&c.source.clone(), &c.column.clone());
                let value_id = Id::unique(("value", path));
                let ev = ui
                    .add_enabled_ui(c.enabled && !NO_VALUE_OPERATORS.contains(&c.op.as_str()), |ui| {
                        suggest_field(
                            ui,
                            value_id,
                            &mut c.value,
                            &suggestions,
                            "value (lists: a, b, c)",
                            150.0,
                        )
                    })
                    .inner;
                if let Some(r) = &ev.response {
                    if focus.as_deref() == Some(path) {
                        r.request_focus();
                    }
                    if r.gained_focus() {
                        events.select = Some(path.to_vec());
                    }
                }
                if ev.submitted {
                    events.run = true;
                }
                if let Some(col) = color_button(ui, Id::unique(("color", path)), &c.color, false) {
                    c.color = col;
                }
                ui.checkbox(&mut c.enabled, "On");
            });
        });
    });
    if row.response.interact(egui::Sense::click()).clicked() {
        events.select = Some(path.to_vec());
    }
}

/// Text for the live "Where:" line.
pub fn where_text(g: &Group) -> String {
    format!("Where: {}", g.describe())
}
