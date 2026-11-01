//! Turning a result table and a [`ChartSpec`] into a [`Chart`].
//!
//! Values are read as the query reads them: numbers as numbers, text such as `1,000`, `5e7`, `50%`
//! or `1 Micrococcus luteus` (-> 1) parsed, anything else skipped (with a note saying how many).

use wbquery_core::cell::{Column, Frame};

use crate::ChartError;
use crate::scene::*;
use crate::spec::{ChartSpec, MULTI_Y, kind_info};
use crate::stats::{
    BoxStats, MAX_CATEGORIES, Table, aggregate, bin_edges, box_stats, histogram, imr_stats, labels, limit,
};
use crate::values::{
    date_numbers, format_g, is_date, is_numeric, label, numeric, point_label, series_name, sort_key,
};

/// Heatmaps with at most this many cells show each cell's value.
pub const ANNOTATE_CELLS: usize = 400;
/// Point labels tried; overlapping ones are dropped when drawing.
pub const MAX_POINT_LABELS: usize = 300;

type Result<T> = std::result::Result<T, ChartError>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(ChartError(msg.into()))
}

/// The table being charted, and which of its cells a condition counted for (column-major, like the frame).
struct Data<'a> {
    frame: &'a Frame,
    hits: Option<&'a [Vec<bool>]>,
}

impl<'a> Data<'a> {
    fn column(&self, name: &str, role: &str) -> Result<&'a Column> {
        match self.frame.column(name) {
            Some(c) => Ok(c),
            None if name.is_empty() => err(format!("Choose a column for {role}")),
            None => err(format!("No column '{name}' in this table")),
        }
    }

    fn hits_for(&self, name: &str) -> Option<&'a [bool]> {
        let hits = self.hits?;
        Some(self.frame.position(name).map_or(&[][..], |j| hits[j].as_slice()))
    }

    /// Y as numbers; an error when none are, and a note counting the values skipped.
    fn y_values(&self, y: &str, notes: &mut Vec<String>) -> Result<Vec<f64>> {
        let raw = self.column(y, "Y")?;
        let vals = numeric(raw);
        if vals.iter().all(|v| v.is_nan()) {
            return err(format!("'{y}' has no numeric values to plot"));
        }
        let skipped = vals.iter().zip(&raw.cells).filter(|(v, c)| v.is_nan() && !c.is_blank()).count();
        skip_note(notes, skipped, y);
        Ok(vals)
    }

    /// (label, rows) per group in first-seen order, or one unnamed group of every row.
    fn groups(&self, group: &str) -> Result<Vec<(String, Vec<usize>)>> {
        if group.is_empty() {
            return Ok(vec![(String::new(), (0..self.frame.len()).collect())]);
        }
        let labels = labels(self.column(group, "the group")?);
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, l) in labels.iter().enumerate() {
            match out.iter_mut().find(|(g, _)| g == l) {
                Some((_, rows)) => rows.push(i),
                None => out.push((l.clone(), vec![i])),
            }
        }
        if out.len() > MAX_CATEGORIES {
            return err(format!(
                "'{group}' has {} distinct values; group by a column with at most {MAX_CATEGORIES}",
                out.len()
            ));
        }
        Ok(out)
    }
}

fn skip_note(notes: &mut Vec<String>, skipped: usize, column: &str) {
    if skipped > 0 {
        let s = if skipped == 1 { "" } else { "s" };
        notes.push(format!("{skipped} non-numeric value{s} in '{column}' skipped"));
    }
}

fn agg_label(spec: &ChartSpec) -> String {
    if spec.y.is_empty() || spec.agg == "count" {
        return if spec.y.is_empty() { "Rows".into() } else { format!("Count of {}", spec.y) };
    }
    let mut agg = spec.agg.clone();
    if let Some(first) = agg.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    format!("{agg} of {}", spec.y)
}

/// Draw `spec` from `frame`. `hits` (same shape as the frame) marks the cells a condition counted for,
/// used by `spec.only_hits`. The chart's notes say what data was skipped.
pub fn build(frame: &Frame, hits: Option<&[Vec<bool>]>, spec: &ChartSpec) -> Result<Chart> {
    let Some(info) = kind_info(&spec.kind) else { return err(format!("Unknown chart type '{}'", spec.kind)) };
    if frame.is_empty() {
        return err("The table has no rows to chart");
    }
    if info.y == crate::spec::Need::Required && spec.y.is_empty() {
        return err("Choose a numeric column for Y");
    }
    if info.x == crate::spec::Need::Required && spec.x.is_empty() {
        return err(format!("Choose a column for {}", info.x_label));
    }
    let data = Data { frame, hits };
    let mut notes = Vec::new();
    let mut panels = match spec.kind.as_str() {
        "line" | "scatter" => multi_y(&data, spec, &mut notes)?,
        "bar" => bar(&data, spec, &mut notes)?,
        "grouped_bar" => grouped_bar(&data, spec, &mut notes)?,
        "histogram" => hist(&data, spec, &mut notes)?,
        "box" => boxes(&data, spec, &mut notes)?,
        "pie" => pie(&data, spec, &mut notes)?,
        "heatmap" => heatmap(&data, spec, &mut notes)?,
        _ => imr(&data, spec, &mut notes)?,
    };
    if spec.kind != "pie" && panels[0].marks.iter().any(|m| m.label().is_some()) {
        panels[0].legend = Some(spec.group.clone());
    }
    if spec.kind == "pie" || spec.kind == "heatmap" {
        for p in &mut panels {
            p.grid = false;
        }
    }
    Ok(Chart { title: spec.title.clone(), panels, notes })
}

// --- line / scatter -------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
enum XKind {
    Date,
    Number,
    Text,
}

impl XKind {
    fn plural(self) -> &'static str {
        match self {
            XKind::Date => "dates",
            XKind::Number => "numbers",
            XKind::Text => "text",
        }
    }
}

fn x_kind(col: &Column) -> XKind {
    if is_date(col) {
        XKind::Date
    } else if is_numeric(col, 0.8) {
        XKind::Number
    } else {
        XKind::Text
    }
}

/// (label, Y columns) per series of one dataset: one per column, or numbered columns merged.
fn y_series(data: &Data, cols: &[String], spec: &ChartSpec) -> Result<Vec<(String, Vec<String>)>> {
    for c in cols {
        data.column(c, "Y")?;
    }
    if !spec.merge_numbered {
        return Ok(cols.iter().map(|c| (c.clone(), vec![c.clone()])).collect());
    }
    let mut merged: Vec<(String, Vec<String>)> = Vec::new();
    for c in cols {
        let name = series_name(c);
        match merged.iter_mut().find(|(n, _)| *n == name) {
            Some((_, cs)) => cs.push(c.clone()),
            None => merged.push((name, vec![c.clone()])),
        }
    }
    Ok(merged)
}

/// Every dataset's X positions, the shared category labels (text X), and what kind of values X holds.
type DatasetXs = (Vec<Vec<f64>>, Option<Vec<String>>, XKind);

/// Plot positions of every dataset's X (row order where blank), and the category labels when the X
/// columns hold text (shared by all datasets, so equal labels line up). Every X must be the same kind.
fn dataset_xs(
    data: &Data,
    datasets: &[crate::spec::Dataset],
    spec: &ChartSpec,
    notes: &mut Vec<String>,
) -> Result<DatasetXs> {
    let mut kinds = Vec::new();
    for (i, ds) in datasets.iter().enumerate() {
        if !ds.x.is_empty() {
            kinds.push(x_kind(data.column(&ds.x, "X")?));
        } else if spec.kind == "scatter" {
            let which = if i > 0 { format!(" in dataset {}", i + 1) } else { String::new() };
            return err(format!("Choose a column for X{which}"));
        } else {
            kinds.push(XKind::Number); // row order
        }
    }
    if kinds.iter().any(|k| *k != kinds[0]) {
        let parts: Vec<String> = datasets
            .iter()
            .zip(&kinds)
            .map(|(ds, k)| format!("{}: {}", if ds.x.is_empty() { "row order" } else { &ds.x }, k.plural()))
            .collect();
        return err(format!(
            "The datasets' X columns must all be dates, all numbers or all text ({})",
            parts.join(", ")
        ));
    }
    if datasets.iter().any(|ds| ds.x.is_empty()) && !data.frame.columns.iter().any(is_date) {
        notes.push(
            "X is row order; add a date column (e.g. Monitor Date) to the outputs to plot over time".into(),
        );
    }
    let n = data.frame.len();
    if kinds[0] == XKind::Text {
        let all: Vec<Vec<String>> =
            datasets.iter().map(|ds| labels(data.frame.column(&ds.x).unwrap())).collect();
        let mut cats: Vec<String> = Vec::new();
        for l in all.iter().flatten() {
            if !cats.contains(l) {
                cats.push(l.clone());
            }
        }
        let xs = all
            .iter()
            .map(|ls| ls.iter().map(|l| cats.iter().position(|c| c == l).unwrap() as f64).collect())
            .collect();
        return Ok((xs, Some(cats), XKind::Text));
    }
    let xs = datasets
        .iter()
        .map(|ds| {
            if ds.x.is_empty() {
                (1..=n).map(|i| i as f64).collect()
            } else {
                let col = data.frame.column(&ds.x).unwrap();
                if kinds[0] == XKind::Date { date_numbers(col) } else { numeric(col) }
            }
        })
        .collect();
    Ok((xs, None, kinds[0]))
}

/// What hovering a point shows: series, column and cell, and the X value.
fn tooltip(series: &str, column: &str, raw: &str, xname: &str, xval: Option<String>) -> String {
    let mut lines =
        vec![if series == column { series.to_string() } else { format!("{series} — {column}") }];
    lines.push(raw.to_string());
    if !xname.is_empty() {
        lines.push(format!("{xname}: {}", xval.unwrap_or_default()));
    }
    lines.join("\n")
}

fn multi_y(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    let datasets = spec.all_datasets();
    if datasets[0].ys.is_empty() {
        return err("Choose a numeric column for Y");
    }
    let series: Vec<Vec<(String, Vec<String>)>> =
        datasets.iter().map(|ds| y_series(data, &ds.ys, spec)).collect::<Result<_>>()?;
    let use_hits = spec.only_hits && data.hits.is_some();
    if spec.only_hits && data.hits.is_none() {
        notes.push("No condition results to limit the points to; showing every value".into());
    }
    let (all_xs, cats, kind) = dataset_xs(data, &datasets, spec, notes)?;
    let mut xnames: Vec<&str> = Vec::new();
    for ds in &datasets {
        let n = if ds.x.is_empty() { "Row" } else { ds.x.as_str() };
        if !xnames.contains(&n) {
            xnames.push(n);
        }
    }
    let groups = data.groups(&spec.group)?;
    let named = series.iter().map(Vec::len).sum::<usize>() > 1;
    let overlaid = datasets.len() > 1;
    let x_axis = match (&cats, kind) {
        (Some(c), _) => Axis::Category(c.clone()),
        (None, XKind::Date) => Axis::Date,
        _ => Axis::Number,
    };
    let mut panel = Panel::new(x_axis, Axis::Number);
    panel.xlabel = xnames.join(" / ");
    let mut point_labels: Vec<PointLabel> = Vec::new();
    let (mut drawn, mut n) = (0, 0);
    for ((ds, ds_series), xs) in datasets.iter().zip(&series).zip(&all_xs) {
        let xcol = if ds.x.is_empty() { None } else { data.frame.column(&ds.x) };
        for (gname, rows) in &groups {
            for (sname, cols) in ds_series {
                // (x, y, label text, column, row) for every cell of the series, column by column.
                let mut pts: Vec<(f64, f64, String, &str, usize)> = Vec::new();
                for c in cols {
                    let col = data.frame.column(c).unwrap();
                    let vals = numeric(col);
                    let keep = if use_hits { data.hits_for(c) } else { None };
                    let kept = |i: usize| keep.is_none_or(|k| k.get(i).copied().unwrap_or(false));
                    let skipped = rows
                        .iter()
                        .filter(|&&i| vals[i].is_nan() && !col.cells[i].is_blank() && kept(i))
                        .count();
                    skip_note(notes, skipped, c);
                    for &i in rows {
                        let y = if kept(i) { vals[i] } else { f64::NAN };
                        if !xs[i].is_nan() && !y.is_nan() {
                            pts.push((xs[i], y, point_label(&col.cells[i]), c.as_str(), i));
                        }
                    }
                }
                if !ds.x.is_empty() && cats.is_none() {
                    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
                }
                let mut name: String = [if named { sname.as_str() } else { "" }, gname.as_str()]
                    .iter()
                    .filter(|p| !p.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" · ");
                if overlaid {
                    let base = if name.is_empty() { sname.clone() } else { name };
                    name = format!("{base} ({})", if ds.x.is_empty() { "row order" } else { &ds.x });
                }
                let label_text = if name.is_empty() { None } else { Some(name.clone()) };
                let series_label = label_text.clone().unwrap_or_else(|| sname.clone());
                let tooltips = pts
                    .iter()
                    .map(|(_, _, _, c, i)| {
                        let raw = label(&data.frame.column(c).unwrap().cells[*i]);
                        tooltip(&series_label, c, &raw, &ds.x, xcol.map(|x| label(&x.cells[*i])))
                    })
                    .collect();
                let points: Vec<[f64; 2]> = pts.iter().map(|p| [p.0, p.1]).collect();
                let color = series_color(n);
                n += 1;
                panel.marks.push(if spec.kind == "line" {
                    Mark::Line { label: label_text, color, points, tooltips }
                } else {
                    Mark::Scatter { label: label_text, color, points, size: 22.0, alpha: 0.8, tooltips }
                });
                drawn += pts.len();
                if spec.point_labels {
                    point_labels.extend(pts.iter().filter(|p| !p.2.is_empty()).map(|p| PointLabel {
                        x: p.0,
                        y: p.1,
                        text: p.2.clone(),
                    }));
                }
            }
        }
    }
    if drawn == 0 {
        let why = if use_hits { " in the cells the conditions matched" } else { "" };
        return err(format!("No numeric values to plot{why}"));
    }
    // Highest values first, so the rarer large counts keep their labels when space runs out.
    point_labels.sort_by(|a, b| b.y.total_cmp(&a.y));
    panel.labels_total = point_labels.len();
    point_labels.truncate(MAX_POINT_LABELS);
    panel.labels = point_labels;
    let mut y_cols: Vec<&str> = Vec::new();
    for c in datasets.iter().flat_map(|d| &d.ys) {
        if !y_cols.contains(&c.as_str()) {
            y_cols.push(c);
        }
    }
    let mut names: Vec<&str> = Vec::new();
    for (name, _) in series.iter().flatten() {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    panel.ylabel = if y_cols.len() == 1 {
        y_cols[0].to_string()
    } else if names.len() <= 4 {
        names.join(", ")
    } else {
        "Value".into()
    };
    Ok(vec![panel])
}

// --- bars, histogram, box, pie, heatmap -------------------------------------------------

fn table_for(data: &Data, spec: &ChartSpec, group: &str) -> Result<Table> {
    let x = data.column(&spec.x, "X")?;
    let y = if spec.y.is_empty() { None } else { Some(data.column(&spec.y, "Y")?) };
    let g = if group.is_empty() { None } else { Some(data.column(group, "the group")?) };
    aggregate(x, y, g, &spec.agg)
}

fn bar(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    if !spec.y.is_empty() {
        data.y_values(&spec.y, notes)?;
    }
    let table = limit(table_for(data, spec, "")?, notes, false);
    let vals = table.column_or_zero(0);
    let mut panel = Panel::new(Axis::Category(table.rows.clone()), Axis::Number);
    let rects =
        vals.iter().enumerate().map(|(i, v)| Rect { x: i as f64 - 0.4, y: 0.0, w: 0.8, h: *v }).collect();
    panel.marks.push(Mark::Bars { label: None, color: series_color(0), rects, alpha: 1.0, edge: None });
    panel.xlabel = spec.x.clone();
    panel.ylabel = agg_label(spec);
    panel.sticky_zero = true;
    Ok(vec![panel])
}

fn grouped_bar(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    if spec.group.is_empty() {
        return err("Choose a column to group the bars by");
    }
    if !spec.y.is_empty() {
        data.y_values(&spec.y, notes)?;
    }
    data.groups(&spec.group)?;
    let table = limit(table_for(data, spec, &spec.group)?, notes, false);
    let n = table.columns.len();
    let width = if spec.stacked { 0.8 } else { 0.8 / n.max(1) as f64 };
    let mut bottom = vec![0.0; table.rows.len()];
    let mut panel = Panel::new(Axis::Category(table.rows.clone()), Axis::Number);
    for (j, g) in table.columns.iter().enumerate() {
        let vals = table.column_or_zero(j);
        let rects = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let pos = i as f64;
                if spec.stacked {
                    Rect { x: pos - width / 2.0, y: bottom[i], w: width, h: *v }
                } else {
                    let center = pos - 0.4 + width * (j as f64 + 0.5);
                    Rect { x: center - width / 2.0, y: 0.0, w: width, h: *v }
                }
            })
            .collect();
        if spec.stacked {
            for (b, v) in bottom.iter_mut().zip(&vals) {
                *b += v;
            }
        }
        panel.marks.push(Mark::Bars {
            label: Some(g.clone()),
            color: series_color(j),
            rects,
            alpha: 1.0,
            edge: None,
        });
    }
    panel.xlabel = spec.x.clone();
    panel.ylabel = agg_label(spec);
    panel.sticky_zero = true;
    Ok(vec![panel])
}

fn hist(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    let y = data.y_values(&spec.y, notes)?;
    let present: Vec<f64> = y.iter().copied().filter(|v| !v.is_nan()).collect();
    let edges = bin_edges(&present, spec.bins.max(1));
    let groups = data.groups(&spec.group)?;
    let alpha = if groups.len() > 1 { 0.55 } else { 0.9 };
    let mut panel = Panel::new(Axis::Number, Axis::Number);
    for (i, (name, rows)) in groups.iter().enumerate() {
        let vals: Vec<f64> = rows.iter().map(|&r| y[r]).filter(|v| !v.is_nan()).collect();
        let counts = histogram(&vals, &edges);
        let rects = counts
            .iter()
            .enumerate()
            .map(|(k, c)| Rect { x: edges[k], y: 0.0, w: edges[k + 1] - edges[k], h: *c })
            .collect();
        let label = if name.is_empty() { None } else { Some(name.clone()) };
        panel.marks.push(Mark::Bars { label, color: series_color(i), rects, alpha, edge: Some(WHITE) });
    }
    panel.xlabel = spec.y.clone();
    panel.ylabel = "Count".into();
    panel.sticky_zero = true;
    Ok(vec![panel])
}

fn boxes(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    let y = data.y_values(&spec.y, notes)?;
    let mut stats: Vec<BoxStats> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for (name, rows) in data.groups(&spec.group)? {
        let vals: Vec<f64> = rows.iter().map(|&r| y[r]).filter(|v| !v.is_nan()).collect();
        if !vals.is_empty() {
            stats.push(box_stats(&vals));
            names.push(if name.is_empty() { spec.y.clone() } else { name });
        }
    }
    let n = stats.len();
    let width = (0.15 * (n.saturating_sub(1)) as f64).clamp(0.15, 0.5);
    let ticks = names.iter().enumerate().map(|(i, s)| ((i + 1) as f64, s.clone())).collect();
    let mut panel = Panel::new(Axis::Ticks(ticks), Axis::Number);
    let marks = stats
        .into_iter()
        .enumerate()
        .map(|(i, s)| BoxMark { pos: (i + 1) as f64, width, stats: s })
        .collect();
    panel.marks.push(Mark::Boxes { color: series_color(0), boxes: marks });
    if !spec.group.is_empty() {
        panel.xlabel = spec.group.clone();
    }
    panel.ylabel = spec.y.clone();
    Ok(vec![panel])
}

fn pie(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    if !spec.y.is_empty() {
        data.y_values(&spec.y, notes)?;
    }
    let table = table_for(data, spec, "")?;
    let vals = table.column_or_zero(0);
    let dropped = vals.iter().filter(|v| **v <= 0.0).count();
    if dropped > 0 {
        let s = if dropped == 1 { "y" } else { "ies" };
        notes.push(format!("{dropped} categor{s} with zero or negative values left out"));
    }
    let kept: Vec<usize> = (0..vals.len()).filter(|&i| vals[i] > 0.0).collect();
    if kept.is_empty() {
        return err("Nothing positive to show in a pie chart");
    }
    let positive = Table {
        rows: kept.iter().map(|&i| table.rows[i].clone()).collect(),
        columns: table.columns.clone(),
        values: kept.iter().map(|&i| vec![Some(vals[i])]).collect(),
    };
    let lump = spec.agg == "count" || spec.agg == "sum" || spec.y.is_empty();
    let table = limit(positive, notes, lump);
    let vals = table.column_or_zero(0);
    let total: f64 = vals.iter().sum();
    let mut start = 90.0;
    let slices = vals
        .iter()
        .zip(&table.rows)
        .enumerate()
        .map(|(i, (v, l))| {
            let frac = v / total;
            let theta2 = start;
            let theta1 = start - 360.0 * frac;
            start = theta1;
            Slice {
                label: l.clone(),
                value: *v,
                color: series_color(i),
                theta1,
                theta2,
                pct: format!("{:.0}%", frac * 100.0),
            }
        })
        .collect();
    let mut panel = Panel::new(Axis::Number, Axis::Number);
    panel.marks.push(Mark::Pie { slices });
    panel.xlabel = agg_label(spec);
    Ok(vec![panel])
}

fn heatmap(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    if spec.group.is_empty() {
        return err("Choose a column for the heatmap rows");
    }
    if !spec.y.is_empty() {
        data.y_values(&spec.y, notes)?;
    }
    data.groups(&spec.group)?;
    let table = limit(table_for(data, spec, &spec.group)?, notes, false);
    // Transposed: one row per group, one column per X category.
    let values: Vec<Vec<Option<f64>>> = (0..table.columns.len())
        .map(|j| table.values.iter().map(|r| r[j].filter(|v| v.is_finite())).collect())
        .collect();
    let finite: Vec<f64> = values.iter().flatten().flatten().copied().collect();
    let (min, max) =
        finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
    let (min, max) = if finite.is_empty() { (0.0, 1.0) } else { (min, max) };
    let mut texts = Vec::new();
    let cells = table.columns.len() * table.rows.len();
    if cells <= ANNOTATE_CELLS {
        let mid = if finite.is_empty() { 0.0 } else { (min + max) / 2.0 };
        for (r, row) in values.iter().enumerate() {
            for (c, v) in row.iter().enumerate() {
                if let Some(v) = v {
                    texts.push(CellText { row: r, col: c, text: format_g(*v, 4), dark: *v > mid });
                }
            }
        }
    }
    let mut panel = Panel::new(Axis::Category(table.rows.clone()), Axis::Category(table.columns.clone()));
    panel.y_inverted = true;
    panel.marks.push(Mark::Heatmap { values, min, max, texts });
    panel.colorbar = Some(Colorbar { label: agg_label(spec), min, max });
    panel.xlabel = spec.x.clone();
    panel.ylabel = spec.group.clone();
    Ok(vec![panel])
}

// --- I-MR -------------------------------------------------------------------------------

fn limit_lines(panel: &mut Panel, lines: [(f64, Color, bool, &str); 3]) {
    for (y, color, dashed, name) in lines {
        panel.marks.push(Mark::HLine { y, color, dashed, text: Some(format!("{name}={}", format_g(y, 4))) });
    }
}

fn imr(data: &Data, spec: &ChartSpec, notes: &mut Vec<String>) -> Result<Vec<Panel>> {
    let y = data.y_values(&spec.y, notes)?;
    let mut rows: Vec<usize> = (0..y.len()).filter(|&i| !y[i].is_nan()).collect();
    let mut xlabel = "Observation".to_string();
    if !spec.x.is_empty() {
        let xcol = data.column(&spec.x, "the order")?;
        match sort_key(xcol) {
            Some(key) => rows.sort_by(|&a, &b| {
                let (ka, kb) = (key[a], key[b]);
                ka.is_nan().cmp(&kb.is_nan()).then(ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal))
            }),
            None => {
                let labels = labels(xcol);
                rows.sort_by(|&a, &b| labels[a].cmp(&labels[b]));
            }
        }
        xlabel = format!("Observation (ordered by {})", spec.x);
    }
    let values: Vec<f64> = rows.iter().map(|&i| y[i]).collect();
    let st = imr_stats(&values, 3.0)?;
    let n: Vec<f64> = (1..=values.len()).map(|i| i as f64).collect();

    let mut ind = Panel::new(Axis::Number, Axis::Number);
    ind.title = "Individuals".into();
    ind.ylabel = spec.y.clone();
    ind.height = 3.0;
    let color = series_color(0);
    ind.marks.push(Mark::Line {
        label: None,
        color,
        points: n.iter().zip(&st.values).map(|(x, y)| [*x, *y]).collect(),
        tooltips: Vec::new(),
    });
    let out = st.out_of_limits();
    let n_out = out.iter().filter(|b| **b).count();
    if n_out > 0 {
        let points = (0..n.len()).filter(|&i| out[i]).map(|i| [n[i], st.values[i]]).collect();
        ind.marks.push(Mark::Scatter {
            label: Some("Outside limits".into()),
            color: OUT_COLOR,
            points,
            size: 30.0,
            alpha: 1.0,
            tooltips: Vec::new(),
        });
        notes.push(format!("{n_out} point{} outside the control limits", if n_out == 1 { "" } else { "s" }));
    }
    limit_lines(
        &mut ind,
        [
            (st.ucl, LIMIT_COLOR, true, "UCL"),
            (st.mean, CENTER_COLOR, false, "X\u{304}"),
            (st.lcl, LIMIT_COLOR, true, "LCL"),
        ],
    );

    let mut mr = Panel::new(Axis::Number, Axis::Number);
    mr.title = "Moving range".into();
    mr.ylabel = "Moving range".into();
    mr.xlabel = xlabel;
    mr.height = 2.0;
    mr.integer_x = true;
    mr.marks.push(Mark::Line {
        label: None,
        color,
        points: n.iter().zip(&st.moving_ranges).map(|(x, y)| [*x, *y]).collect(),
        tooltips: Vec::new(),
    });
    let mr_out: Vec<[f64; 2]> = (0..n.len())
        .filter(|&i| st.moving_ranges[i] > st.mr_ucl)
        .map(|i| [n[i], st.moving_ranges[i]])
        .collect();
    if !mr_out.is_empty() {
        mr.marks.push(Mark::Scatter {
            label: None,
            color: OUT_COLOR,
            points: mr_out,
            size: 30.0,
            alpha: 1.0,
            tooltips: Vec::new(),
        });
    }
    limit_lines(
        &mut mr,
        [
            (st.mr_ucl, LIMIT_COLOR, true, "UCL"),
            (st.mr_bar, CENTER_COLOR, false, "MR\u{304}"),
            (0.0, LIMIT_COLOR, true, "LCL"),
        ],
    );
    Ok(vec![ind, mr])
}

/// Whether the chart type takes several Y columns and overlay datasets.
pub fn is_multi_y(kind: &str) -> bool {
    MULTI_Y.contains(&kind)
}
