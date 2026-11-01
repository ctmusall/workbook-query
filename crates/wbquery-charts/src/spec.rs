//! What chart to draw: the chart types and their options.

/// Whether a chart type uses a column slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    None,
    Optional,
    Required,
}

/// What a chart type needs and how the app labels its options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KindInfo {
    pub key: &'static str,
    pub label: &'static str,
    pub x: Need,
    pub y: Need,
    pub group: Need,
    pub agg: bool,
    pub bins: bool,
    pub stacked: bool,
    pub x_label: &'static str,
    pub group_label: &'static str,
}

const fn kind(key: &'static str, label: &'static str, x: Need, y: Need, group: Need) -> KindInfo {
    KindInfo {
        key,
        label,
        x,
        y,
        group,
        agg: false,
        bins: false,
        stacked: false,
        x_label: "X",
        group_label: "Group by",
    }
}

use Need::{None as No, Optional as Opt, Required as Req};

/// Every chart type, in the order the app lists them.
pub const KINDS: &[KindInfo] = &[
    KindInfo { x_label: "X (blank: row order)", ..kind("line", "Line / run chart", Opt, Req, Opt) },
    KindInfo { group_label: "Color by", ..kind("scatter", "Scatter", Req, Req, Opt) },
    KindInfo { agg: true, x_label: "Category", ..kind("bar", "Bar", Req, Opt, No) },
    KindInfo {
        agg: true,
        stacked: true,
        x_label: "Category",
        ..kind("grouped_bar", "Grouped / stacked bar", Req, Opt, Req)
    },
    KindInfo { bins: true, ..kind("histogram", "Histogram", No, Req, Opt) },
    kind("box", "Box plot", No, Req, Opt),
    KindInfo { agg: true, x_label: "Category", ..kind("pie", "Pie", Req, Opt, No) },
    KindInfo {
        agg: true,
        x_label: "Columns",
        group_label: "Rows",
        ..kind("heatmap", "Heatmap / pivot", Req, Opt, Req)
    },
    KindInfo { x_label: "Order by (blank: row order)", ..kind("imr", "I-MR control chart", Opt, Req, No) },
];

pub fn kind_info(key: &str) -> Option<&'static KindInfo> {
    KINDS.iter().find(|k| k.key == key)
}

pub const AGGREGATES: &[&str] = &["count", "sum", "mean", "median", "min", "max"];
/// Chart types that take several Y columns and overlay datasets.
pub const MULTI_Y: &[&str] = &["line", "scatter"];

/// One overlay on a line / scatter chart: an X column (blank: row order) and its Y columns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dataset {
    pub x: String,
    pub ys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChartSpec {
    pub kind: String,
    pub x: String,
    pub y: String,
    pub group: String,
    pub agg: String,
    pub bins: usize,
    pub stacked: bool,
    pub title: String,
    /// Line / scatter: more Y columns after `y`.
    pub ys: Vec<String>,
    /// Line / scatter: `Chest 1`, `Chest 2 (3)` -> one `Chest` series.
    pub merge_numbered: bool,
    /// Line / scatter: only the cells a condition counted for.
    pub only_hits: bool,
    /// Line / scatter: label points with the text after their number.
    pub point_labels: bool,
    /// Line / scatter: more overlays, each with its own X.
    pub datasets: Vec<Dataset>,
}

impl Default for ChartSpec {
    fn default() -> ChartSpec {
        ChartSpec {
            kind: "line".into(),
            x: String::new(),
            y: String::new(),
            group: String::new(),
            agg: "mean".into(),
            bins: 20,
            stacked: false,
            title: String::new(),
            ys: Vec::new(),
            merge_numbered: true,
            only_hits: false,
            point_labels: false,
            datasets: Vec::new(),
        }
    }
}

impl ChartSpec {
    pub fn new(kind: &str) -> ChartSpec {
        ChartSpec { kind: kind.into(), ..ChartSpec::default() }
    }

    fn multi_y(&self) -> bool {
        MULTI_Y.contains(&self.kind.as_str())
    }

    /// Every Y column: `y` then `ys` (without repeats); only line and scatter use `ys`.
    pub fn y_columns(&self) -> Vec<String> {
        let mut cols: Vec<String> = if self.y.is_empty() { Vec::new() } else { vec![self.y.clone()] };
        if self.multi_y() {
            for c in &self.ys {
                if !c.is_empty() && !cols.contains(c) {
                    cols.push(c.clone());
                }
            }
        }
        cols
    }

    /// The first dataset (`x` with [`ChartSpec::y_columns`]), then the extra ones that have a Y column.
    pub fn all_datasets(&self) -> Vec<Dataset> {
        let mut out = vec![Dataset { x: self.x.clone(), ys: self.y_columns() }];
        if self.multi_y() {
            for d in &self.datasets {
                let mut ys: Vec<String> = Vec::new();
                for c in d.ys.iter().filter(|c| !c.is_empty()) {
                    if !ys.contains(c) {
                        ys.push(c.clone());
                    }
                }
                if !ys.is_empty() {
                    out.push(Dataset { x: d.x.clone(), ys });
                }
            }
        }
        out
    }
}
