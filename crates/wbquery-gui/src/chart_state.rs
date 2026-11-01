//! The Visualize tab's state, without UI: which table and chart type, the column choices (with the
//! defaults and automatic picks the Python app made), and the chart they draw.

use std::path::{Path, PathBuf};

use wbquery_charts::render::{self, DEFAULT_SIZE};
use wbquery_charts::spec::{Dataset, MULTI_Y, Need, kind_info};
use wbquery_charts::values::{is_date, numeric_columns};
use wbquery_charts::{Chart, ChartSpec};
use wbquery_core::conditions::sort_columns;
use wbquery_core::query::{QueryResult, ResultTable, is_provenance};

/// The table's columns sorted by what they hold.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ColumnKinds {
    pub numeric: Vec<String>,
    pub date: Vec<String>,
    pub category: Vec<String>,
    /// File / sheet / row columns, kept last in every list.
    pub extra: Vec<String>,
}

/// What shows under the chart.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// Notes about skipped data (may be empty).
    Notes(String),
    /// Why the chart can't be drawn.
    Error(String),
    /// Where a chart was saved.
    Saved(PathBuf),
}

#[derive(Debug, Clone)]
pub struct ChartState {
    /// The result tables available, and the one charted.
    pub tables: Vec<String>,
    pub table: String,
    pub kind: String,
    pub x: String,
    pub y: String,
    pub group: String,
    pub agg: String,
    pub bins: usize,
    pub stacked: bool,
    pub title: String,
    /// Line / scatter: Y columns after `y`.
    pub extra_y: Vec<String>,
    /// Line / scatter: overlays, each with its own X.
    pub datasets: Vec<Dataset>,
    pub merge_numbered: bool,
    pub only_hits: bool,
    pub point_labels: bool,
    /// "Only cells that met a condition" is possible (the query had conditions).
    pub hits_enabled: bool,
    pub kinds: ColumnKinds,
    pub x_options: Vec<String>,
    pub y_options: Vec<String>,
    pub group_options: Vec<String>,
    pub chart: Option<Chart>,
    pub message: Message,
    result: Option<QueryResult>,
}

impl Default for ChartState {
    fn default() -> ChartState {
        ChartState {
            tables: Vec::new(),
            table: String::new(),
            kind: "line".into(),
            x: String::new(),
            y: String::new(),
            group: String::new(),
            agg: "mean".into(),
            bins: 20,
            stacked: false,
            title: String::new(),
            extra_y: Vec::new(),
            datasets: Vec::new(),
            merge_numbered: true,
            only_hits: false,
            point_labels: false,
            hits_enabled: false,
            kinds: ColumnKinds::default(),
            x_options: Vec::new(),
            y_options: Vec::new(),
            group_options: Vec::new(),
            chart: None,
            message: Message::Notes("Run a query to chart its results.".into()),
            result: None,
        }
    }
}

/// Sorted names, with file / sheet / row columns last.
fn ordered(items: &[String]) -> Vec<String> {
    let (extra, rest): (Vec<String>, Vec<String>) = items.iter().cloned().partition(|c| is_provenance(c));
    let mut out = sort_columns(&rest);
    out.extend(extra);
    out
}

/// Keep `current` when it's still offered, else `default` when offered; else blank (optional) or the
/// first item.
fn pick(current: &str, items: &[String], optional: bool, default: &str) -> String {
    if !current.is_empty() && items.iter().any(|i| i == current) {
        current.to_string()
    } else if !default.is_empty() && items.iter().any(|i| i == default) {
        default.to_string()
    } else if optional {
        String::new()
    } else {
        items.first().cloned().unwrap_or_default()
    }
}

impl ChartState {
    fn info(&self) -> &'static wbquery_charts::KindInfo {
        kind_info(&self.kind).unwrap_or(&wbquery_charts::KINDS[0])
    }

    fn multi_y(&self) -> bool {
        MULTI_Y.contains(&self.kind.as_str())
    }

    /// The table being charted.
    pub fn frame(&self) -> Option<&ResultTable> {
        self.result.as_ref()?.table(&self.table)
    }

    /// Show `result`'s tables, keeping the chosen table and columns where they still exist.
    pub fn set_result(&mut self, result: Option<&QueryResult>) {
        self.result = result.cloned();
        self.tables = result.map(|r| r.tables.iter().map(|t| t.name.clone()).collect()).unwrap_or_default();
        if !self.tables.contains(&self.table) {
            self.table = self.tables.first().cloned().unwrap_or_default();
        }
        self.hits_enabled = result.is_some_and(|r| !r.query.r#where.conditions(true).is_empty());
        self.table_changed();
    }

    pub fn set_table(&mut self, table: &str) {
        self.table = table.to_string();
        self.table_changed();
    }

    /// Refill the column lists from the chosen table.
    pub fn table_changed(&mut self) {
        let (cols, nums, dates) = match self.frame() {
            None => (Vec::new(), Vec::new(), Vec::new()),
            Some(t) => {
                let cols = t.frame.names();
                let nums = numeric_columns(&t.frame.columns);
                let dates: Vec<String> =
                    t.frame.columns.iter().filter(|c| is_date(c)).map(|c| c.name.clone()).collect();
                (cols, nums, dates)
            }
        };
        let extra: Vec<String> = cols.iter().filter(|c| is_provenance(c)).cloned().collect();
        let plain: Vec<String> = cols.iter().filter(|c| !is_provenance(c)).cloned().collect();
        let numeric: Vec<String> = nums.into_iter().filter(|c| plain.contains(c)).collect();
        let others: Vec<String> = plain.iter().filter(|c| !numeric.contains(c)).cloned().collect();
        let date: Vec<String> = others.iter().filter(|c| dates.contains(c)).cloned().collect();
        let category: Vec<String> = others.iter().filter(|c| !date.contains(c)).cloned().collect();
        self.kinds = ColumnKinds { numeric, date, category, extra };
        let all: Vec<String> = plain.iter().chain(&self.kinds.extra).cloned().collect();
        self.y_options = ordered(&all);
        self.x_options = ordered(&all);
        self.group_options = ordered(&all);
        let info = self.info();
        let y_default = self.kinds.numeric.first().cloned().unwrap_or_default();
        self.y = pick(&self.y, &self.y_options, info.y == Need::Optional, &y_default);
        let x_default = self.kinds.date.first().or(self.kinds.category.first()).cloned().unwrap_or_default();
        self.x = pick(&self.x, &self.x_options, true, &x_default);
        self.group = pick(&self.group, &self.group_options, true, "");
        let present = |c: &String| all.contains(c);
        self.extra_y = self.extra_y.iter().filter(|c| present(c)).cloned().collect();
        let datasets = self
            .datasets
            .iter()
            .map(|d| Dataset {
                x: if present(&d.x) { d.x.clone() } else { String::new() },
                ys: d.ys.iter().filter(|c| present(c)).cloned().collect(),
            })
            .collect();
        self.set_datasets(datasets);
        self.set_extra_y(self.extra_y.clone());
        self.fill_required();
        self.redraw();
    }

    pub fn set_kind(&mut self, kind: &str) {
        self.kind = kind.to_string();
        if self.info().y == Need::Required && self.y.is_empty() {
            self.y = self.y_options.first().cloned().unwrap_or_default();
        }
        self.fill_required();
        self.redraw();
    }

    /// Y columns after the Y box's (without repeats or the Y column itself).
    pub fn set_extra_y(&mut self, columns: Vec<String>) {
        let mut out: Vec<String> = Vec::new();
        for c in columns {
            if !c.is_empty() && c != self.y && !out.contains(&c) {
                out.push(c);
            }
        }
        self.extra_y = out;
    }

    /// Use `datasets` as the overlays (ones without a Y column are dropped).
    pub fn set_datasets(&mut self, datasets: Vec<Dataset>) {
        self.datasets = datasets
            .into_iter()
            .filter_map(|d| {
                let mut ys: Vec<String> = Vec::new();
                for c in d.ys.into_iter().filter(|c| !c.is_empty()) {
                    if !ys.contains(&c) {
                        ys.push(c);
                    }
                }
                (!ys.is_empty()).then_some(Dataset { x: d.x, ys })
            })
            .collect();
    }

    /// Choose every Y column at once: the first goes in the Y box.
    pub fn pick_y(&mut self, columns: &[String]) -> bool {
        let Some((first, rest)) = columns.split_first() else { return false };
        self.y = first.clone();
        self.set_extra_y(rest.to_vec());
        self.redraw();
        true
    }

    /// Pick a column for an X or Group the chart type needs but that is still blank.
    pub fn fill_required(&mut self) {
        let info = self.info();
        let k = &self.kinds;
        if info.x == Need::Required && self.x.is_empty() {
            let order: Vec<&String> = if self.kind == "scatter" {
                k.date.iter().chain(&k.numeric).collect()
            } else {
                k.category.iter().chain(&k.date).collect()
            };
            if let Some(c) = order.into_iter().find(|c| **c != self.y) {
                self.x = c.clone();
            }
        }
        if info.group == Need::Required && self.group.is_empty() {
            let order = k.category.iter().chain(&k.date).chain(&k.extra).chain(&k.numeric);
            if let Some(c) = order.into_iter().find(|c| **c != self.x && **c != self.y) {
                self.group = c.clone();
            }
        }
    }

    /// The chart the controls describe.
    pub fn spec(&self) -> ChartSpec {
        let info = self.info();
        let multi = self.multi_y();
        ChartSpec {
            kind: self.kind.clone(),
            x: if info.x != Need::None { self.x.clone() } else { String::new() },
            y: self.y.clone(),
            group: if info.group != Need::None { self.group.clone() } else { String::new() },
            agg: self.agg.clone(),
            bins: self.bins,
            stacked: self.stacked,
            title: self.title.trim().to_string(),
            ys: if multi { self.extra_y.clone() } else { Vec::new() },
            merge_numbered: self.merge_numbered,
            only_hits: self.only_hits && self.hits_enabled,
            point_labels: self.point_labels,
            datasets: if multi { self.datasets.clone() } else { Vec::new() },
        }
    }

    /// Build the chart; false (with the reason in `message`) when it can't be drawn.
    pub fn redraw(&mut self) -> bool {
        let built = self.frame().map(|t| wbquery_charts::build(&t.frame, Some(&t.hits), &self.spec()));
        match built {
            None => {
                self.chart = None;
                self.message = Message::Notes("Run a query to chart its results.".into());
                false
            }
            Some(Ok(chart)) => {
                self.message = Message::Notes(chart.notes.join(" · "));
                self.chart = Some(chart);
                true
            }
            Some(Err(e)) => {
                self.chart = None;
                self.message = Message::Error(e.0);
                false
            }
        }
    }

    /// Save the chart as PNG, SVG or PDF (by extension; anything else becomes PNG).
    pub fn save(&mut self, path: &Path) -> Result<PathBuf, String> {
        if !self.redraw() {
            return Err(match &self.message {
                Message::Error(e) => e.clone(),
                _ => "There's no chart to save; run a query first.".into(),
            });
        }
        let chart = self.chart.as_ref().expect("drawn");
        let saved = render::save(chart, path, DEFAULT_SIZE).map_err(|e| e.0)?;
        let full = std::path::absolute(&saved.path).unwrap_or(saved.path.clone());
        self.message = Message::Saved(full.clone());
        Ok(full)
    }
}
