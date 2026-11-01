//! The numbers behind the charts: pivot tables, I-MR limits, histogram bins, box-plot statistics.

use wbquery_core::cell::{Cell, Column};

use crate::ChartError;
use crate::spec::AGGREGATES;
use crate::values::{label, numeric, sort_key};

pub const MAX_CATEGORIES: usize = 50;
pub const OTHER: &str = "Other";
/// d2 for moving ranges of span 2.
pub const D2: f64 = 1.128;

/// A pivot: one row per X category, one column per group, each cell an aggregate (None: no values).
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub rows: Vec<String>,
    pub columns: Vec<String>,
    pub values: Vec<Vec<Option<f64>>>,
}

impl Table {
    /// One column's values, blanks as 0.
    pub fn column_or_zero(&self, j: usize) -> Vec<f64> {
        self.values.iter().map(|r| r[j].unwrap_or(0.0)).collect()
    }
}

fn first_seen(labels: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in labels {
        if !out.contains(l) {
            out.push(l.clone());
        }
    }
    out
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

fn reduce(agg: &str, mut v: Vec<f64>) -> f64 {
    match agg {
        "count" => v.len() as f64,
        "sum" => v.iter().sum(),
        "mean" => v.iter().sum::<f64>() / v.len() as f64,
        "median" => median(&mut v),
        "min" => v.iter().copied().fold(f64::INFINITY, f64::min),
        _ => v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    }
}

/// `agg` of Y per X category (rows) and group (columns; a single `value` column without a group).
/// With no Y, counts rows. Rows follow X's date or number order when it has one, else first-seen order.
pub fn aggregate(
    x: &Column,
    y: Option<&Column>,
    group: Option<&Column>,
    agg: &str,
) -> Result<Table, ChartError> {
    if !AGGREGATES.contains(&agg) {
        return Err(ChartError(format!("Aggregate must be one of {}", AGGREGATES.join(", "))));
    }
    let agg = if y.is_none() { "count" } else { agg };
    let n = x.len();
    let xs: Vec<String> = x.cells.iter().map(label).collect();
    let gs: Vec<String> = match group {
        Some(g) => g.cells.iter().map(label).collect(),
        None => vec!["value".into(); n],
    };
    let vals: Vec<f64> = match y {
        Some(y) => numeric(y),
        None => vec![1.0; n],
    };
    let rows = first_seen(&xs);
    let columns = first_seen(&gs);
    let mut cells: Vec<Vec<Vec<f64>>> = vec![vec![Vec::new(); columns.len()]; rows.len()];
    for i in 0..n {
        if vals[i].is_nan() {
            continue;
        }
        let r = rows.iter().position(|v| *v == xs[i]).expect("row label");
        let c = columns.iter().position(|v| *v == gs[i]).expect("column label");
        cells[r][c].push(vals[i]);
    }
    let mut values: Vec<Vec<Option<f64>>> = cells
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|v| if v.is_empty() { (agg == "count").then_some(0.0) } else { Some(reduce(agg, v)) })
                .collect()
        })
        .collect();
    let mut rows = rows;
    if let Some(key) = sort_key(x) {
        let first = |r: &String| xs.iter().position(|v| v == r).map(|i| key[i]).unwrap_or(f64::NAN);
        let mut order: Vec<usize> = (0..rows.len()).collect();
        order.sort_by(|&a, &b| {
            let (ka, kb) = (first(&rows[a]), first(&rows[b]));
            (ka.is_nan(), if ka.is_nan() { 0.0 } else { ka })
                .partial_cmp(&(kb.is_nan(), if kb.is_nan() { 0.0 } else { kb }))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows = order.iter().map(|&i| rows[i].clone()).collect();
        values = order.iter().map(|&i| values[i].clone()).collect();
    }
    Ok(Table { rows, columns, values })
}

/// At most [`MAX_CATEGORIES`] rows, keeping the largest; `lump` sums the rest into `Other`.
pub fn limit(table: Table, notes: &mut Vec<String>, lump: bool) -> Table {
    let n = table.rows.len();
    if n <= MAX_CATEGORIES {
        return table;
    }
    let total: Vec<f64> = table.values.iter().map(|r| r.iter().flatten().map(|v| v.abs()).sum()).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| total[b].total_cmp(&total[a]));
    let keep_n = MAX_CATEGORIES - usize::from(lump);
    let keep: std::collections::HashSet<usize> = order[..keep_n].iter().copied().collect();
    let mut out = Table { rows: Vec::new(), columns: table.columns.clone(), values: Vec::new() };
    let mut rest = vec![0.0; table.columns.len()];
    for i in 0..n {
        if keep.contains(&i) {
            out.rows.push(table.rows[i].clone());
            out.values.push(table.values[i].clone());
        } else {
            for (j, v) in table.values[i].iter().enumerate() {
                rest[j] += v.unwrap_or(0.0);
            }
        }
    }
    if lump {
        out.rows.push(OTHER.into());
        out.values.push(rest.into_iter().map(Some).collect());
        notes.push(format!("{} smaller categories grouped as '{OTHER}'", n - keep_n));
    } else {
        notes.push(format!("Showing the {MAX_CATEGORIES} largest of {n} categories"));
    }
    out
}

/// Individuals and moving-range control limits.
#[derive(Debug, Clone, PartialEq)]
pub struct ImrStats {
    pub values: Vec<f64>,
    /// The first entry is NaN.
    pub moving_ranges: Vec<f64>,
    pub mean: f64,
    pub mr_bar: f64,
    pub ucl: f64,
    pub lcl: f64,
    pub mr_ucl: f64,
}

impl ImrStats {
    pub fn out_of_limits(&self) -> Vec<bool> {
        self.values.iter().map(|v| *v > self.ucl || *v < self.lcl).collect()
    }
}

/// mean ± k·MR̄/d2, and MR UCL = (1 + k·d3/d2)·MR̄ (2.66·MR̄ and 3.267·MR̄ for k = 3).
pub fn imr_stats(values: &[f64], k: f64) -> Result<ImrStats, ChartError> {
    if values.len() < 2 {
        return Err(ChartError("An I-MR chart needs at least 2 numeric values".into()));
    }
    let mut mr = vec![f64::NAN];
    mr.extend(values.windows(2).map(|w| (w[1] - w[0]).abs()));
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let mr_bar = mr[1..].iter().sum::<f64>() / (mr.len() - 1) as f64;
    let sigma = mr_bar / D2;
    Ok(ImrStats {
        values: values.to_vec(),
        moving_ranges: mr,
        mean,
        mr_bar,
        ucl: mean + k * sigma,
        lcl: mean - k * sigma,
        mr_ucl: mr_bar * (1.0 + k * 0.8525 / D2),
    })
}

/// `bins` equal-width bin edges over the values (numpy's `histogram_bin_edges` with a bin count).
pub fn bin_edges(values: &[f64], bins: usize) -> Vec<f64> {
    let bins = bins.max(1);
    let (mut lo, mut hi) =
        values.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| (a.min(*v), b.max(*v)));
    if values.is_empty() {
        (lo, hi) = (0.0, 1.0);
    }
    if lo == hi {
        lo -= 0.5;
        hi += 0.5;
    }
    let step = (hi - lo) / bins as f64;
    (0..=bins).map(|i| if i == bins { hi } else { lo + step * i as f64 }).collect()
}

/// How many values fall in each bin (the last bin includes its right edge).
pub fn histogram(values: &[f64], edges: &[f64]) -> Vec<f64> {
    let n = edges.len() - 1;
    let mut counts = vec![0.0; n];
    let (lo, hi) = (edges[0], edges[n]);
    for &v in values {
        if v < lo || v > hi {
            continue;
        }
        let mut i = edges.partition_point(|e| *e <= v).saturating_sub(1);
        if i >= n {
            i = n - 1;
        }
        counts[i] += 1.0;
    }
    counts
}

/// One box of a box plot (Tukey whiskers at 1.5 × IQR).
#[derive(Debug, Clone, PartialEq)]
pub struct BoxStats {
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub whisker_lo: f64,
    pub whisker_hi: f64,
    pub fliers: Vec<f64>,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let pos = p / 100.0 * (sorted.len() - 1) as f64;
    let (i, frac) = (pos.floor() as usize, pos - pos.floor());
    if i + 1 >= sorted.len() {
        sorted[sorted.len() - 1]
    } else {
        sorted[i] + (sorted[i + 1] - sorted[i]) * frac
    }
}

/// Quartiles, whiskers and outliers, as matplotlib's `boxplot_stats` computes them.
pub fn box_stats(values: &[f64]) -> BoxStats {
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let (q1, median, q3) = (percentile(&v, 25.0), percentile(&v, 50.0), percentile(&v, 75.0));
    let iqr = q3 - q1;
    let (lo_lim, hi_lim) = (q1 - 1.5 * iqr, q3 + 1.5 * iqr);
    let whisker_lo = v.iter().copied().filter(|x| *x >= lo_lim).fold(f64::INFINITY, f64::min);
    let whisker_hi = v.iter().copied().filter(|x| *x <= hi_lim).fold(f64::NEG_INFINITY, f64::max);
    let whisker_lo = if whisker_lo.is_finite() { whisker_lo.min(q1) } else { q1 };
    let whisker_hi = if whisker_hi.is_finite() { whisker_hi.max(q3) } else { q3 };
    let fliers = values.iter().copied().filter(|x| *x < whisker_lo || *x > whisker_hi).collect();
    BoxStats { q1, median, q3, whisker_lo, whisker_hi, fliers }
}

/// The label of each cell (for group and category columns).
pub fn labels(col: &Column) -> Vec<String> {
    col.cells.iter().map(label).collect()
}

/// Whether a cell is blank (for counting skipped values).
pub fn blank(c: &Cell) -> bool {
    c.is_blank()
}
