//! The main window: sheets, how they combine, conditions and output columns on the left; buttons,
//! the status line, and the result tables or chart on the right. Dialogs are drawn as modals.

use std::path::PathBuf;

use egui::{Id, RichText, Ui};
use wbquery_charts::Dataset;
use wbquery_charts::spec::{AGGREGATES, KINDS, MULTI_Y, Need};
use wbquery_core::conditions::{ANY_SOURCE, Condition, NO_VALUE_OPERATORS, OPERATORS};
use wbquery_core::pyfmt::casefold;
use wbquery_core::query::JOIN_HOW;

use crate::chart_state::Message;
use crate::conditions_view::{conditions_tree, where_text};
use crate::state::{Added, AppState, Tick};
use crate::widgets::{color_button, combo, search_combo, suggest_field};

pub const WORKBOOK_EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "csv", "tsv", "txt"];

/// A checklist row of a column picker.
#[derive(Clone)]
struct PickRow {
    column: String,
    sheets: Vec<String>,
    tick: Tick,
    /// Started half-ticked, so "leave as is" is a choice.
    tristate: bool,
}

/// Where "Add for column…" puts its conditions.
#[derive(Clone, Copy, PartialEq)]
enum Place {
    Selected,
    NewOr,
    NewAnd,
}

/// What a column picker is for.
enum PickerUse {
    /// Output columns for every sheet at once.
    Outputs,
    /// "Add for column…": one any-sheet condition per ticked column.
    Shared { op: String, value: String, color: String, place: Place, problem: String },
    /// The chart's Y columns.
    ChartY,
    /// The Y columns of one overlay dataset (index into the datasets dialog).
    DatasetY(usize),
}

struct Picker {
    title: String,
    prompt: String,
    rows: Vec<PickRow>,
    filter: String,
    show_sheets: bool,
    purpose: PickerUse,
}

enum Dialog {
    Sheets { path: PathBuf, sheets: Vec<(String, bool)> },
    Columns(Picker),
    Datasets { rows: Vec<Dataset> },
}

#[derive(PartialEq, Clone, Copy)]
enum View {
    Tables,
    Visualize,
}

pub struct WbqApp {
    pub state: AppState,
    dialogs: Vec<Dialog>,
    view: View,
    tab: usize,
    alias_edits: Vec<String>,
    source_row: Option<usize>,
    last_title: String,
    /// Point labels hidden in the chart's last drawing.
    hidden_labels: usize,
    /// A picker to open over the current dialog (the datasets dialog's Y columns).
    pending_child: Option<Picker>,
    /// The left pane (sheets, conditions, outputs) is shown.
    pub left_open: bool,
}

impl WbqApp {
    pub fn new(state: AppState) -> WbqApp {
        WbqApp {
            state,
            dialogs: Vec::new(),
            view: View::Tables,
            tab: 0,
            alias_edits: Vec::new(),
            source_row: None,
            last_title: String::new(),
            hidden_labels: 0,
            pending_child: None,
            left_open: true,
        }
    }

    /// Open files given on the command line or dropped on the window: queries load, anything else
    /// is added as a workbook.
    pub fn open_paths(&mut self, paths: &[PathBuf]) {
        for p in paths {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) {
                self.state.load_query(p);
            } else {
                self.add_workbook(p.clone());
            }
        }
    }

    fn add_workbook(&mut self, path: PathBuf) {
        if let Added::Choose(path, names) = self.state.add_workbook(&path, None) {
            let sheets = names.into_iter().enumerate().map(|(i, s)| (s, i == 0)).collect();
            self.dialogs.push(Dialog::Sheets { path, sheets });
        }
    }

    fn browse_workbooks(&mut self) {
        let files = rfd::FileDialog::new()
            .set_title("Add workbooks")
            .set_directory(self.state.save_dir())
            .add_filter("Workbooks", WORKBOOK_EXTENSIONS)
            .add_filter("All files", &["*"])
            .pick_files();
        for f in files.unwrap_or_default() {
            self.state.remember_dir(&f);
            self.add_workbook(f);
        }
    }

    fn save_query_dialog(&mut self) {
        let path = rfd::FileDialog::new()
            .set_title("Save query")
            .set_directory(self.state.save_dir())
            .set_file_name(self.state.query_file_name())
            .add_filter("Query", &["json"])
            .save_file();
        if let Some(p) = path {
            self.state.save_query(&p);
        }
    }

    fn load_query_dialog(&mut self) {
        let path = rfd::FileDialog::new()
            .set_title("Load query")
            .set_directory(self.state.save_dir())
            .add_filter("Query", &["json"])
            .pick_file();
        if let Some(p) = path {
            self.state.load_query(&p);
            self.after_reset();
        }
    }

    fn export_dialog(&mut self, fmt: &str) {
        let (label, ext) = if fmt == "xlsx" { ("Excel workbook", "xlsx") } else { ("HTML report", "html") };
        let path = rfd::FileDialog::new()
            .set_title("Export results")
            .set_directory(self.state.save_dir())
            .set_file_name(self.state.export_file_name(fmt))
            .add_filter(label, &[ext])
            .save_file();
        if let Some(p) = path {
            self.state.export(fmt, &p);
        }
    }

    fn save_chart_dialog(&mut self) {
        let path = rfd::FileDialog::new()
            .set_title("Save chart")
            .set_directory(self.state.save_dir())
            .set_file_name("chart.png")
            .add_filter("PNG image", &["png"])
            .add_filter("SVG image", &["svg"])
            .add_filter("PDF document", &["pdf"])
            .save_file();
        if let Some(p) = path {
            match self.state.chart.save(&p) {
                Ok(saved) => self.state.remember_dir(&saved),
                Err(e) => self.state.error(format!("Couldn't save the chart to {}:\n{e}", p.display())),
            }
        }
    }

    fn after_reset(&mut self) {
        self.tab = 0;
        self.source_row = None;
    }

    fn run(&mut self) {
        if self.state.run() {
            self.tab =
                self.tab.min(self.state.result.as_ref().map_or(0, |r| r.tables.len().saturating_sub(1)));
        }
    }

    // --- layout ----------------------------------------------------------------------------

    pub fn draw(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // Files dropped on the window.
        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !dropped.is_empty() {
            for p in dropped {
                self.add_workbook(p);
            }
        }
        let title = self.state.title();
        if title != self.last_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }
        self.shortcuts(&ctx);

        if self.left_open {
            egui::Panel::left("left").resizable(true).default_size(640.0).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("◀").on_hover_text("Hide this pane").clicked() {
                            self.left_open = false;
                        }
                    });
                });
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.left(ui));
            });
        } else {
            egui::Panel::left("left-hidden").resizable(false).exact_size(30.0).show(ui, |ui| {
                if ui.button("▶").on_hover_text("Show sheets, conditions and output columns").clicked() {
                    self.left_open = true;
                }
            });
        }
        egui::CentralPanel::default().show(ui, |ui| self.right(ui));
        self.dialogs(&ctx);
        self.error_dialog(&ctx);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some()) && ctx.egui_wants_keyboard_input();
        if ctx.input_mut(|i| {
            i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::D))
        }) {
            self.state.duplicate();
        }
        if !typing && self.dialogs.is_empty() && ctx.input(|i| i.key_pressed(egui::Key::Delete)) {
            self.state.remove();
        }
    }

    fn left(&mut self, ui: &mut Ui) {
        section(ui, "Query", |ui| self.about(ui));
        section(ui, "Workbook sheets", |ui| self.sources(ui));
        section(ui, "How sheets combine", |ui| self.combine(ui));
        section(ui, "Conditions", |ui| self.conditions(ui));
        section(ui, "Output columns", |ui| self.outputs(ui));
    }

    fn about(&mut self, ui: &mut Ui) {
        egui::Grid::new("about").num_columns(2).show(ui, |ui| {
            ui.label("Name");
            ui.add(
                egui::TextEdit::singleline(&mut self.state.name)
                    .hint_text("Name it, e.g. EOJ personnel hits, full year")
                    .desired_width(f32::INFINITY),
            );
            ui.end_row();
            ui.label("Description");
            ui.add(
                egui::TextEdit::multiline(&mut self.state.description)
                    .hint_text("What it finds and why, so you remember later (saved with the query and shown in exports)")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            ui.end_row();
        });
    }

    fn conditions(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.button("Add condition").on_hover_text("Add a condition to the selected group, on the selected condition's sheet and column").clicked() {
                self.state.add_condition(None);
            }
            let shared = self.state.shared_columns();
            if ui
                .add_enabled(!shared.is_empty(), egui::Button::new("Add for column…"))
                .on_hover_text("Pick columns that several sheets share; each gets one condition that tests every sheet with that column")
                .clicked()
            {
                self.open_shared_picker();
            }
            if ui.button("Add group").clicked() {
                self.state.add_group("OR");
            }
            if ui.button("Duplicate").on_hover_text("Copy the selected condition or group (Ctrl+D)").clicked() {
                self.state.duplicate();
            }
            if ui.button("Remove").on_hover_text("Remove the selected condition or group (Delete)").clicked() {
                self.state.remove();
            }
            if ui.button("Up").clicked() {
                self.state.move_selected(-1);
            }
            if ui.button("Down").clicked() {
                self.state.move_selected(1);
            }
            if ui
                .add_enabled(self.state.can_move_out(), egui::Button::new("Out"))
                .on_hover_text("Move the selected condition or group out of its group")
                .clicked()
            {
                self.state.move_out();
            }
            if ui
                .add_enabled(self.state.can_move_in(), egui::Button::new("In"))
                .on_hover_text("Move the selected condition or group into the group just above it")
                .clicked()
            {
                self.state.move_in();
            }
        });
        let ev = conditions_tree(ui, &mut self.state);
        if let Some(sel) = ev.select {
            self.state.selected = Some(sel);
        }
        if ev.run {
            self.run();
        }
        ui.label(RichText::new(where_text(&self.state.conditions)).weak());
    }

    fn sources(&mut self, ui: &mut Ui) {
        let n = self.state.entries.len();
        self.alias_edits.resize(n, String::new());
        let join = self.state.join;
        egui::Grid::new("sources").num_columns(if join { 4 } else { 3 }).striped(true).show(ui, |ui| {
            ui.label(RichText::new("Name").strong());
            ui.label(RichText::new("File").strong());
            ui.label(RichText::new("Sheet").strong());
            if join {
                ui.label(RichText::new("Join key").strong());
            }
            ui.end_row();
            for i in 0..n {
                let alias = self.state.entries[i].source.alias.clone();
                let id = Id::unique(("alias", i));
                let editing = ui.memory(|m| m.has_focus(id));
                if !editing {
                    self.alias_edits[i] = alias.clone();
                }
                let r = ui
                    .add_sized([150.0, 20.0], egui::TextEdit::singleline(&mut self.alias_edits[i]).id(id))
                    .on_hover_text("The name conditions and outputs use; edit to rename");
                if r.gained_focus() {
                    self.source_row = Some(i);
                }
                if r.lost_focus()
                    && self.alias_edits[i].trim() != alias
                    && !self.state.rename_source(i, &self.alias_edits[i].clone())
                {
                    self.alias_edits[i] = alias.clone();
                }
                let e = &self.state.entries[i];
                let file = std::path::Path::new(&e.source.path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let selected = self.source_row == Some(i);
                if ui.selectable_label(selected, file).on_hover_text(&e.source.path).clicked() {
                    self.source_row = Some(i);
                }
                ui.label(&e.source.sheet);
                if join {
                    let mut key = e.source.key.clone();
                    let cols = wbquery_core::conditions::sort_columns(&e.columns());
                    if search_combo(ui, Id::unique(("key", i)), "Join key", &mut key, &cols, 130.0, &|_| None)
                    {
                        self.state.entries[i].source.key = key;
                    }
                }
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Add workbooks…").clicked() {
                self.browse_workbooks();
            }
            let can_remove = self.source_row.is_some_and(|r| r < n);
            if ui
                .add_enabled(can_remove, egui::Button::new("Remove sheet"))
                .on_hover_text("Remove the selected sheet")
                .clicked()
                && let Some(r) = self.source_row
            {
                self.state.remove_source(r);
                self.source_row = None;
            }
        });
        if n == 0 {
            ui.label(RichText::new("Drop workbooks on the window, or use Add workbooks…").weak());
        }
    }

    fn combine(&mut self, ui: &mut Ui) {
        ui.radio_value(&mut self.state.join, false, "Filter each sheet on its own");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.state.join, true, "Join sheets on a key column");
            ui.add_enabled_ui(self.state.join, |ui| {
                combo(ui, Id::unique("join-how"), "Join type", &mut self.state.join_how, JOIN_HOW, 80.0);
            });
        });
        ui.checkbox(&mut self.state.provenance, "Add file / sheet / row columns");
    }

    fn outputs(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.state.outputs.is_empty(), egui::Button::new("Choose for all sheets…"))
                .on_hover_text("Tick or untick a column in every sheet that has it, all at once")
                .clicked()
            {
                let rows = self
                    .state
                    .column_states()
                    .into_iter()
                    .map(|(column, sheets, tick)| PickRow { column, sheets, tristate: tick == Tick::Some, tick })
                    .collect();
                self.dialogs.push(Dialog::Columns(Picker {
                    title: "Output columns for all sheets".into(),
                    prompt: "Tick the columns to output. Each applies to every sheet that has it; a half-ticked column is left as it is.".into(),
                    rows,
                    filter: String::new(),
                    show_sheets: true,
                    purpose: PickerUse::Outputs,
                }));
            }
        });
        let mut moves: Vec<(String, usize, usize)> = Vec::new();
        let mut all: Vec<(String, bool)> = Vec::new();
        for o in &mut self.state.outputs {
            let ticked = o.columns.iter().filter(|c| c.1).count();
            egui::CollapsingHeader::new(format!("{}  ({ticked} of {})", o.alias, o.columns.len()))
                .id_salt(("outputs", &o.alias))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("All").clicked() {
                            all.push((o.alias.clone(), true));
                        }
                        if ui.small_button("None").clicked() {
                            all.push((o.alias.clone(), false));
                        }
                        ui.label(RichText::new("drag ≡ to reorder").weak());
                    });
                    for (k, (col, on)) in o.columns.iter_mut().enumerate() {
                        let drop = ui.dnd_drop_zone::<(String, usize), ()>(egui::Frame::new(), |ui| {
                            ui.horizontal(|ui| {
                                let handle = Id::unique(("drag", &o.alias, k));
                                ui.dnd_drag_source(handle, (o.alias.clone(), k), |ui| {
                                    ui.label("≡");
                                });
                                ui.checkbox(on, col.as_str());
                            });
                        });
                        if let Some(p) = drop.1
                            && p.0 == o.alias
                        {
                            moves.push((o.alias.clone(), p.1, k));
                        }
                    }
                });
        }
        for (alias, on) in all {
            self.state.set_all_outputs(&alias, on);
        }
        for (alias, from, to) in moves {
            self.state.move_output(&alias, from, to);
        }
        ui.label(RichText::new("Tick the columns to output for each sheet.").weak());
    }

    fn right(&mut self, ui: &mut Ui) {
        let has_entries = !self.state.entries.is_empty();
        let has_result = self.state.result.is_some();
        ui.horizontal(|ui| {
            if ui.add_enabled(has_entries, egui::Button::new(RichText::new("Run query").strong())).clicked() {
                self.run();
            }
            if ui.add_enabled(has_result, egui::Button::new("Export Excel…")).clicked() {
                self.export_dialog("xlsx");
            }
            if ui.add_enabled(has_result, egui::Button::new("Export HTML…")).clicked() {
                self.export_dialog("html");
            }
            ui.separator();
            ui.selectable_value(&mut self.view, View::Tables, "Tables");
            ui.add_enabled_ui(has_result, |ui| {
                ui.selectable_value(&mut self.view, View::Visualize, "Visualize")
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Load query…").clicked() {
                    self.load_query_dialog();
                }
                if ui.add_enabled(has_entries, egui::Button::new("Save query…")).clicked() {
                    self.save_query_dialog();
                }
            });
        });
        self.status_line(ui);
        ui.separator();
        if !has_result {
            self.view = View::Tables;
        }
        match self.view {
            View::Tables => self.tables(ui),
            View::Visualize => self.visualize(ui),
        }
    }

    fn status_line(&mut self, ui: &mut Ui) {
        let status = self.state.status.clone();
        ui.horizontal_wrapped(|ui| {
            let mut lines = status.text.lines();
            if let Some(first) = lines.next() {
                match first.split_once(" — ") {
                    Some((head, rest)) if head == "Matched" => {
                        ui.label(RichText::new(head).strong());
                        ui.label(format!("— {rest}"));
                    }
                    _ => {
                        ui.label(first);
                    }
                }
            }
            if let Some(path) = &status.saved
                && ui.link("Open folder").clicked()
                && let Some(folder) = path.parent()
            {
                ui.ctx().open_url(egui::OpenUrl::new_tab(format!("file://{}", folder.display())));
            }
            for w in lines {
                ui.end_row();
                ui.label(RichText::new(w).color(ui.visuals().warn_fg_color));
            }
        });
    }

    fn tables(&mut self, ui: &mut Ui) {
        let Some(result) = &self.state.result else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Run the query to see the matching rows here.").weak())
            });
            return;
        };
        let names: Vec<String> =
            result.tables.iter().map(|t| format!("{} ({})", t.name, t.frame.len())).collect();
        if names.is_empty() {
            ui.label("No result tables: tick some output columns.");
            return;
        }
        self.tab = self.tab.min(names.len() - 1);
        ui.horizontal_wrapped(|ui| {
            for (i, n) in names.iter().enumerate() {
                ui.selectable_value(&mut self.tab, i, n);
            }
        });
        ui.separator();
        ui.label(RichText::new("Right-click a cell to add a condition on its value").weak().small());
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            if let Some(cond) = crate::table_view::result_table(ui, &mut self.state, self.tab) {
                self.state.add_condition_from_result(cond);
            }
        });
    }

    fn visualize(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let mut kind_changed = None;
        let mut table_changed = None;
        let mut want_y = false;
        let mut want_datasets = false;
        let mut save = false;
        {
            let c = &mut self.state.chart;
            let info = *wbquery_charts::kind_info(&c.kind).unwrap_or(&KINDS[0]);
            let multi = MULTI_Y.contains(&c.kind.as_str());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                ui.label("Table");
                let mut table = c.table.clone();
                if search_combo(ui, Id::unique("chart-table"), "Table", &mut table, &c.tables, 150.0, &|_| None) {
                    table_changed = Some(table);
                }
                ui.label("Chart");
                let mut kind = c.kind.clone();
                let r = egui::ComboBox::from_id_salt("chart-kind").selected_text(info.label).width(170.0).show_ui(ui, |ui| {
                    for k in KINDS {
                        ui.selectable_value(&mut kind, k.key.to_string(), k.label);
                    }
                });
                let label = format!("Chart: {}", info.label);
                r.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, &label));
                if kind != c.kind {
                    kind_changed = Some(kind);
                }
                ui.label("Title");
                changed |= ui.add(egui::TextEdit::singleline(&mut c.title).hint_text("optional").desired_width(150.0)).changed();
                ui.end_row();

                let opt = |items: &[String], optional: bool| -> Vec<String> {
                    let mut v = if optional { vec!["(none)".to_string()] } else { Vec::new() };
                    v.extend(items.iter().cloned());
                    v
                };
                let pick = |ui: &mut Ui, id: &str, value: &mut String, items: Vec<String>| -> bool {
                    let mut shown = if value.is_empty() { "(none)".to_string() } else { value.clone() };
                    let ch = search_combo(ui, Id::unique(id), id, &mut shown, &items, 150.0, &|_| None);
                    if ch {
                        *value = if shown == "(none)" { String::new() } else { shown };
                    }
                    ch
                };
                if info.x != Need::None {
                    ui.label(info.x_label);
                    changed |= pick(ui, "chart-x", &mut c.x, opt(&c.x_options, true));
                }
                ui.label(if info.y == Need::Optional { "Y (blank: count rows)" } else { "Y" });
                changed |= pick(ui, "chart-y", &mut c.y, opt(&c.y_options, info.y == Need::Optional));
                if info.group != Need::None {
                    ui.label(info.group_label);
                    changed |= pick(ui, "chart-group", &mut c.group, opt(&c.group_options, true));
                }
                ui.end_row();

                if info.agg {
                    ui.label("Aggregate");
                    changed |= combo(ui, Id::unique("chart-agg"), "Aggregate", &mut c.agg, AGGREGATES, 90.0);
                }
                if info.bins {
                    ui.label("Bins");
                    changed |= ui.add(egui::DragValue::new(&mut c.bins).range(1..=500)).changed();
                }
                if info.stacked {
                    changed |= ui.checkbox(&mut c.stacked, "Stacked").changed();
                }
                if multi {
                    let n = c.extra_y.len();
                    let label = if n > 0 { format!("More Y… (+{n})") } else { "More Y…".into() };
                    let tip = if n > 0 {
                        format!("Also plotting: {}", c.extra_y.join(", "))
                    } else {
                        "Plot several Y columns (e.g. every Chest, Sleeve and Hand column)".into()
                    };
                    want_y = ui.button(label).on_hover_text(tip).clicked();
                    let n = c.datasets.len();
                    let label = if n > 0 { format!("More datasets… (+{n})") } else { "More datasets…".into() };
                    want_datasets = ui
                        .button(label)
                        .on_hover_text("Overlay more series, each with its own X column (e.g. Sample Date with the Hand columns)")
                        .clicked();
                    changed |= ui
                        .checkbox(&mut c.merge_numbered, "Combine numbered columns")
                        .on_hover_text("Chest 1, Chest 2 and Chest 1 (2) become one 'Chest' series")
                        .changed();
                }
                ui.end_row();
                if multi {
                    ui.add_enabled_ui(c.hits_enabled, |ui| {
                        changed |= ui
                            .checkbox(&mut c.only_hits, "Only cells that met a condition")
                            .on_hover_text("Plot only the cells a condition counted for (the ones highlighted in the results)")
                            .changed();
                    });
                    changed |= ui
                        .checkbox(&mut c.point_labels, "Label points with their text")
                        .on_hover_text("Write the text after each number next to its point (1 Micrococcus luteus -> 'Micrococcus luteus')")
                        .changed();
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Drag to pan, scroll to zoom, double-click to reset.").weak().small());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = ui.add_enabled(c.chart.is_some(), egui::Button::new("Save chart…")).clicked();
                });
            });
        }
        if let Some(t) = table_changed {
            self.state.chart.set_table(&t);
        }
        if let Some(k) = kind_changed {
            self.state.chart.set_kind(&k);
        }
        if changed {
            let extra = self.state.chart.extra_y.clone();
            self.state.chart.set_extra_y(extra);
            self.state.chart.redraw();
        }
        if want_y {
            let c = &self.state.chart;
            let current = c.spec().y_columns();
            let rows = c
                .kinds
                .numeric
                .iter()
                .map(|col| PickRow {
                    column: col.clone(),
                    sheets: Vec::new(),
                    tick: if current.contains(col) { Tick::All } else { Tick::None },
                    tristate: false,
                })
                .collect();
            self.dialogs.push(Dialog::Columns(Picker {
                title: "Y columns".into(),
                prompt: "Tick the columns to plot. With 'Combine numbered columns', Chest 1, Chest 2, Chest 1 (2)… make one 'Chest' line.".into(),
                rows,
                filter: String::new(),
                show_sheets: false,
                purpose: PickerUse::ChartY,
            }));
        }
        if want_datasets {
            let mut rows = self.state.chart.datasets.clone();
            if rows.is_empty() {
                rows.push(Dataset::default());
            }
            self.dialogs.push(Dialog::Datasets { rows });
        }
        if save {
            self.save_chart_dialog();
        }
        let message_h = 40.0;
        match self.state.chart.chart.clone() {
            Some(chart) => {
                let size = egui::vec2(ui.available_width(), (ui.available_height() - message_h).max(150.0));
                ui.allocate_ui(size, |ui| {
                    self.hidden_labels = crate::plot_view::chart_view(ui, &chart, Id::unique("chart"));
                });
            }
            None => {
                ui.add_space(20.0);
            }
        }
        match &self.state.chart.message {
            Message::Notes(n) => {
                let mut text = n.clone();
                if let Some(h) = wbquery_charts::render::hidden_note(self.hidden_labels) {
                    if !text.is_empty() {
                        text.push_str(" · ");
                    }
                    text.push_str(&h);
                }
                ui.label(text);
            }
            Message::Error(e) => {
                ui.label(RichText::new(format!("⚠ {e}")).color(egui::Color32::from_rgb(0xb0, 0x50, 0x00)));
            }
            Message::Saved(p) => {
                ui.horizontal(|ui| {
                    ui.label(format!("Chart saved to {}", p.display()));
                    if ui.link("Open folder").clicked()
                        && let Some(folder) = p.parent()
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(format!("file://{}", folder.display())));
                    }
                });
            }
        }
    }

    // --- dialogs ------------------------------------------------------------------------------

    fn open_shared_picker(&mut self) {
        let rows = self
            .state
            .shared_columns()
            .into_iter()
            .map(|(column, _)| {
                let sheets = self.state.sheets_with(&column);
                PickRow { column, sheets, tick: Tick::None, tristate: false }
            })
            .collect();
        self.dialogs.push(Dialog::Columns(Picker {
            title: "Add for column".into(),
            prompt: "Tick the columns to filter on. Each gets one condition that tests every sheet with that column, using the operator and value below.".into(),
            rows,
            filter: String::new(),
            show_sheets: true,
            purpose: PickerUse::Shared {
                op: "=".into(),
                value: String::new(),
                color: String::new(),
                place: Place::Selected,
                problem: String::new(),
            },
        }));
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialogs.pop() else { return };
        let mut done: Option<bool> = None; // Some(true): OK, Some(false): Cancel
        let modal = egui::Modal::new(Id::unique(("dialog", self.dialogs.len()))).show(ctx, |ui| {
            ui.set_width(460.0);
            match &mut dialog {
                Dialog::Sheets { path, sheets } => {
                    let name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                    ui.heading(format!("Sheets in {name}"));
                    ui.label("Choose the sheets to query:");
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (s, on) in sheets.iter_mut() {
                            ui.checkbox(on, s.as_str());
                        }
                    });
                }
                Dialog::Columns(p) => self.picker_ui(ui, p),
                Dialog::Datasets { rows } => self.datasets_ui(ui, rows),
            }
            ui.separator();
            if let Dialog::Columns(Picker { purpose: PickerUse::Shared { problem, .. }, .. }) = &dialog
                && !problem.is_empty()
            {
                ui.label(RichText::new(problem.as_str()).color(egui::Color32::from_rgb(0xc0, 0x39, 0x2b)));
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
        if modal.should_close() && done.is_none() {
            done = Some(false);
        }
        match done {
            None => {
                self.dialogs.push(dialog);
                if let Some(child) = self.pending_child.take() {
                    self.dialogs.push(Dialog::Columns(child));
                }
            }
            Some(false) => {}
            Some(true) => {
                if let Some(back) = self.accept(dialog) {
                    self.dialogs.push(back); // stays open (e.g. a value that doesn't fit the operator)
                }
            }
        }
    }

    fn picker_ui(&mut self, ui: &mut Ui, p: &mut Picker) {
        ui.heading(&p.title);
        ui.label(&p.prompt);
        ui.add(
            egui::TextEdit::singleline(&mut p.filter)
                .hint_text("Search columns")
                .desired_width(f32::INFINITY),
        );
        let needle = casefold(p.filter.trim());
        let shown = |r: &PickRow| needle.is_empty() || casefold(&r.column).contains(&needle);
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for r in p.rows.iter_mut().filter(|r| shown(r)) {
                let n = r.sheets.len();
                let label = if p.show_sheets {
                    format!("{}   ({n} sheet{})", r.column, if n == 1 { "" } else { "s" })
                } else {
                    r.column.clone()
                };
                let mut on = r.tick == Tick::All;
                let mut resp =
                    ui.add(egui::Checkbox::new(&mut on, label).indeterminate(r.tick == Tick::Some));
                if p.show_sheets {
                    let mut tip = format!("In: {}", r.sheets.join(", "));
                    if r.tristate {
                        tip.push_str("\nHalf-ticked: ticked in some of these sheets; left as is.");
                    }
                    resp = resp.on_hover_text(tip);
                }
                if resp.clicked() {
                    r.tick = match (r.tick, r.tristate) {
                        (Tick::None, _) => Tick::All,
                        (Tick::All, true) => Tick::Some,
                        (Tick::All, false) => Tick::None,
                        (Tick::Some, _) => Tick::None,
                    };
                    if let PickerUse::Shared { problem, .. } = &mut p.purpose {
                        problem.clear();
                    }
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("All").on_hover_text("Tick every column shown").clicked() {
                for r in p.rows.iter_mut().filter(|r| shown(r)) {
                    r.tick = Tick::All;
                }
            }
            if ui.button("None").on_hover_text("Untick every column shown").clicked() {
                for r in p.rows.iter_mut().filter(|r| shown(r)) {
                    r.tick = Tick::None;
                }
            }
        });
        let checked: Vec<String> =
            p.rows.iter().filter(|r| r.tick == Tick::All).map(|r| r.column.clone()).collect();
        if let PickerUse::Shared { op, value, color, place, problem } = &mut p.purpose {
            ui.separator();
            ui.label(RichText::new("Condition for every ticked column").strong());
            ui.horizontal(|ui| {
                if combo(ui, Id::unique("shared-op"), "Operator", op, OPERATORS, 100.0) {
                    problem.clear();
                }
                let suggestions =
                    if checked.is_empty() { Vec::new() } else { self.state.values_of(&checked) };
                ui.add_enabled_ui(!NO_VALUE_OPERATORS.contains(&op.as_str()), |ui| {
                    if suggest_field(
                        ui,
                        Id::unique("shared-value"),
                        value,
                        &suggestions,
                        "value (lists: a, b, c) — can be blank",
                        200.0,
                    )
                    .changed
                    {
                        problem.clear();
                    }
                });
                ui.label("Highlight");
                if let Some(c) = color_button(ui, Id::unique("shared-color"), color, false) {
                    *color = c;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Put them in");
                ui.radio_value(place, Place::Selected, "the selected group");
            });
            ui.radio_value(place, Place::NewOr, "a new OR group (any column matches)");
            ui.radio_value(place, Place::NewAnd, "a new AND group (every column matches)");
        }
    }

    fn datasets_ui(&mut self, ui: &mut Ui, rows: &mut Vec<Dataset>) {
        ui.heading("More datasets");
        ui.label("Each dataset is drawn over the chart's own X and Y with its own X column, e.g. Sample Date with the Hand columns.");
        let k = self.state.chart.kinds.clone();
        let mut xs: Vec<String> = wbquery_core::conditions::sort_columns(
            &[k.date.clone(), k.numeric.clone(), k.category.clone()].concat(),
        );
        xs.extend(k.extra.iter().cloned());
        let mut x_items = vec!["(none)".to_string()];
        x_items.extend(xs);
        let mut remove = None;
        let mut pick = None;
        for (i, d) in rows.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label("X");
                let mut shown = if d.x.is_empty() { "(none)".to_string() } else { d.x.clone() };
                if search_combo(ui, Id::unique(("ds-x", i)), "X", &mut shown, &x_items, 150.0, &|_| None) {
                    d.x = if shown == "(none)" { String::new() } else { shown };
                }
                let label = match d.ys.len() {
                    0 => "Y columns…".to_string(),
                    1 | 2 => format!("Y: {}", d.ys.join(", ")),
                    n => format!("Y: {} +{}", d.ys[0], n - 1),
                };
                if ui
                    .button(label)
                    .on_hover_text(if d.ys.is_empty() {
                        "Choose the Y columns of this dataset".into()
                    } else {
                        d.ys.join(", ")
                    })
                    .clicked()
                {
                    pick = Some(i);
                }
                if ui.button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
        if ui.button("Add dataset").clicked() {
            rows.push(Dataset::default());
        }
        if let Some(i) = remove {
            rows.remove(i);
        }
        if let Some(i) = pick {
            let current = rows[i].ys.clone();
            let picker = Picker {
                title: "Y columns".into(),
                prompt: "Tick the columns to plot against this dataset's X.".into(),
                rows: k
                    .numeric
                    .iter()
                    .map(|c| PickRow {
                        column: c.clone(),
                        sheets: Vec::new(),
                        tick: if current.contains(c) { Tick::All } else { Tick::None },
                        tristate: false,
                    })
                    .collect(),
                filter: String::new(),
                show_sheets: false,
                purpose: PickerUse::DatasetY(i),
            };
            // The Y picker opens over this dialog; this dialog comes back when it closes.
            self.pending_child = Some(picker);
        }
    }

    /// Apply a dialog's OK. Returns the dialog when it must stay open.
    fn accept(&mut self, dialog: Dialog) -> Option<Dialog> {
        match dialog {
            Dialog::Sheets { path, sheets } => {
                let chosen: Vec<String> = sheets.into_iter().filter(|(_, on)| *on).map(|(s, _)| s).collect();
                self.state.add_workbook(&path, Some(chosen));
                None
            }
            Dialog::Datasets { rows } => {
                self.state.chart.set_datasets(rows);
                self.state.chart.redraw();
                None
            }
            Dialog::Columns(mut p) => {
                let checked: Vec<String> =
                    p.rows.iter().filter(|r| r.tick == Tick::All).map(|r| r.column.clone()).collect();
                match &mut p.purpose {
                    PickerUse::Outputs => {
                        let states: Vec<(String, Tick)> =
                            p.rows.iter().map(|r| (r.column.clone(), r.tick)).collect();
                        self.state.apply_column_states(&states);
                        None
                    }
                    PickerUse::ChartY => {
                        self.state.chart.pick_y(&checked);
                        None
                    }
                    PickerUse::DatasetY(i) => {
                        if let Some(Dialog::Datasets { rows }) = self.dialogs.last_mut()
                            && let Some(d) = rows.get_mut(*i)
                        {
                            d.ys = checked;
                        }
                        None
                    }
                    PickerUse::Shared { op, value, color, place, problem } => {
                        let msg = shared_problem(&checked, op, value);
                        if !msg.is_empty() {
                            *problem = msg;
                            return Some(Dialog::Columns(p));
                        }
                        let value = if NO_VALUE_OPERATORS.contains(&op.as_str()) {
                            String::new()
                        } else {
                            value.clone()
                        };
                        let logic = match place {
                            Place::Selected => None,
                            Place::NewOr => Some("OR"),
                            Place::NewAnd => Some("AND"),
                        };
                        self.state.add_shared_many(&checked, op, &value, logic, color);
                        None
                    }
                }
            }
        }
    }

    fn error_dialog(&mut self, ctx: &egui::Context) {
        if self.state.errors.is_empty() {
            return;
        }
        let mut close = false;
        egui::Modal::new(Id::unique("error")).show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.heading(crate::state::APP);
            ui.label(&self.state.errors[0]);
            ui.separator();
            if ui.button("OK").clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
            {
                close = true;
            }
        });
        if close {
            self.state.errors.remove(0);
        }
    }
}

/// A collapsible section of the left pane, open at first.
fn section(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .id_salt(("section", title))
        .default_open(true)
        .show(ui, add);
}

/// Why "Add for column…" can't go ahead, or "". A blank value is allowed (filled in later).
pub fn shared_problem(checked: &[String], op: &str, value: &str) -> String {
    let Some(first) = checked.first() else { return "Tick at least one column.".into() };
    let value = if NO_VALUE_OPERATORS.contains(&op) { "" } else { value };
    if value.trim().is_empty() && !NO_VALUE_OPERATORS.contains(&op) {
        return String::new();
    }
    match Condition::new(ANY_SOURCE, first, op, value).and_then(|c| c.check_value()) {
        Ok(()) => String::new(),
        Err(e) => {
            let s = e.to_string();
            s.split_once(": ").map(|(_, rest)| rest.to_string()).unwrap_or(s)
        }
    }
}

impl eframe::App for WbqApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
