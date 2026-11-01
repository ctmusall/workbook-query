//! A result table: rows drawn only when visible, click a header to sort, highlight colors from the
//! conditions, and right-click a cell to add a condition on its value.

use egui::{Color32, RichText, Ui};
use egui_extras::{Column, TableBuilder};
use wbquery_core::conditions::{Condition, text_color};

use crate::state::AppState;
use crate::widgets::color32;

/// Draw result table `t`; returns a condition the user chose to add from a cell's menu.
pub fn result_table(ui: &mut Ui, state: &mut AppState, t: usize) -> Option<Condition> {
    let order = state.table_order(t);
    let table = state.result.as_ref().and_then(|r| r.tables.get(t))?;
    let frame = &table.frame;
    let sort = state.sort.get(t).copied().unwrap_or_default();
    let mut clicked_header = None;
    let mut add = None;
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    let ncols = frame.columns.len();
    let mut builder = TableBuilder::new(ui)
        .id_salt(("result", t, &table.name))
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .min_scrolled_height(0.0)
        .auto_shrink([false, false]);
    for _ in 0..ncols {
        builder = builder.column(Column::auto().at_least(50.0).clip(true));
    }
    builder
        .header(row_h + 2.0, |mut header| {
            for (j, c) in frame.columns.iter().enumerate() {
                header.col(|ui| {
                    let arrow = match (sort.column == Some(j), sort.descending) {
                        (true, false) => " ▲",
                        (true, true) => " ▼",
                        _ => "",
                    };
                    let r = ui
                        .add(
                            egui::Button::new(RichText::new(format!("{}{arrow}", c.name)).strong())
                                .frame(false),
                        )
                        .on_hover_text("Click to sort");
                    if r.clicked() {
                        clicked_header = Some(j);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h, order.len(), |mut row| {
                let i = order[row.index()];
                for (j, c) in frame.columns.iter().enumerate() {
                    row.col(|ui| {
                        let color = &table.colors[j][i];
                        let text = c.cells[i].display();
                        let cell = ui.max_rect().expand2(egui::vec2(4.0, 2.0));
                        if let Some(bg) = color32(color) {
                            ui.painter().rect_filled(cell, 0.0, bg);
                            let fg = color32(text_color(color)).unwrap_or(Color32::BLACK);
                            ui.add(egui::Label::new(RichText::new(text).color(fg)).selectable(false));
                        } else {
                            ui.add(egui::Label::new(text).selectable(false));
                        }
                        // The whole cell (blank ones too) opens the menu.
                        let r = ui.interact(cell, egui::Id::unique(("cell", t, i, j)), egui::Sense::click());
                        r.context_menu(|ui| {
                            let conds = state.cell_conditions(&c.name, &c.cells[i]);
                            if conds.is_empty() {
                                ui.label("No condition can test this column");
                            }
                            for cond in conds {
                                if ui.button(format!("Add condition: {}", cond.describe())).clicked() {
                                    add = Some(cond);
                                    ui.close();
                                }
                            }
                        });
                    });
                }
            });
        });
    if let Some(j) = clicked_header {
        state.toggle_sort(t, j);
    }
    add
}
