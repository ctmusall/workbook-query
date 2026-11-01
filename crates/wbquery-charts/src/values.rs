//! Reading table cells for charts: numbers, dates, category labels.

use std::sync::LazyLock;

use chrono::NaiveDateTime;
use regex::Regex;
use wbquery_core::cell::{Cell, Column, DType, is_midnight};
use wbquery_core::loader::{cell_number, leading_number};
use wbquery_core::pyfmt::py_float_repr;

pub const BLANK: &str = "(blank)";

static COPY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\(\d+\)$").unwrap());
static NUMBERED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*\d+$").unwrap());

/// The series a numbered column belongs to: `Chest 1 (2)` -> `Chest`, `Hand 2` -> `Hand`.
pub fn series_name(column: &str) -> String {
    let base = NUMBERED.replace(&COPY.replace(column, ""), "").into_owned();
    if base.is_empty() { column.to_string() } else { base }
}

/// The text after a cell's number (`1 Micrococcus luteus` -> `Micrococcus luteus`), else "".
pub fn point_label(c: &Cell) -> String {
    match c {
        Cell::Text(s) => leading_number(s).map(|(_, t)| t).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The column's values as floats; blanks, booleans, dates and text that isn't a number are NaN.
pub fn numeric(col: &Column) -> Vec<f64> {
    match col.dtype {
        DType::Bool => vec![f64::NAN; col.len()],
        _ => col
            .cells
            .iter()
            .map(|c| match c {
                Cell::Bool(_) | Cell::Date(_) | Cell::Empty => f64::NAN,
                c if c.is_blank() => f64::NAN,
                c => cell_number(c),
            })
            .collect(),
    }
}

/// A date column: dates throughout (blanks aside).
pub fn is_date(col: &Column) -> bool {
    col.dtype == DType::Date
        || (col.cells.iter().any(|c| !c.is_empty())
            && col.cells.iter().all(|c| matches!(c, Cell::Date(_) | Cell::Empty)))
}

/// At least `share` of the non-blank values are numbers (dates aren't).
pub fn is_numeric(col: &Column, share: f64) -> bool {
    if is_date(col) {
        return false;
    }
    let nums = numeric(col);
    let present: Vec<f64> =
        col.cells.iter().zip(&nums).filter(|(c, _)| !c.is_blank()).map(|(_, n)| *n).collect();
    if present.is_empty() {
        return false;
    }
    present.iter().filter(|n| !n.is_nan()).count() as f64 / present.len() as f64 >= share
}

/// The columns charts can plot as numbers.
pub fn numeric_columns(columns: &[Column]) -> Vec<String> {
    columns.iter().filter(|c| is_numeric(c, 0.8)).map(|c| c.name.clone()).collect()
}

/// A value as a category label: `(blank)`, dates without a midnight time, whole floats without `.0`.
pub fn label(c: &Cell) -> String {
    match c {
        c if c.is_blank() => BLANK.into(),
        Cell::Date(d) if is_midnight(d) => d.format("%Y-%m-%d").to_string(),
        Cell::Float(f) if f.is_finite() && f.fract() == 0.0 => {
            if *f == 0.0 {
                "0".into()
            } else {
                format!("{f:.0}")
            }
        }
        Cell::Float(f) => py_float_repr(*f),
        other => other.py_str(),
    }
}

/// Matplotlib's date number: days since 1970-01-01 (fractions are times of day).
pub fn date_number(d: &NaiveDateTime) -> f64 {
    d.and_utc().timestamp_micros() as f64 / 86_400e6
}

/// The column's dates as matplotlib date numbers (NaN where a cell isn't a date).
pub fn date_numbers(col: &Column) -> Vec<f64> {
    col.cells.iter().map(|c| if let Cell::Date(d) = c { date_number(d) } else { f64::NAN }).collect()
}

/// A sort key per row: dates or numbers when the column holds them, else None (keep first-seen order).
pub fn sort_key(col: &Column) -> Option<Vec<f64>> {
    if is_date(col) {
        Some(date_numbers(col))
    } else if is_numeric(col, 0.8) {
        Some(numeric(col))
    } else {
        None
    }
}

/// Python's `format(v, ".{p}g")`: `1.105e+08`, `4.4e+07`, `3.267`, `0`.
pub fn format_g(v: f64, p: usize) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let p = p.max(1);
    let sci = format!("{:.*e}", p - 1, v);
    let (mant, exp) = sci.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent");
    let strip = |s: &str| -> String {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s.to_string()
        }
    };
    if exp < -4 || exp >= p as i32 {
        format!("{}e{}{:02}", strip(mant), if exp < 0 { '-' } else { '+' }, exp.abs())
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        strip(&format!("{v:.decimals$}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_format() {
        for (v, s) in [
            (110489361.70212767, "1.105e+08"),
            (44000000.0, "4.4e+07"),
            (-22489361.702127665, "-2.249e+07"),
            (0.0, "0"),
            (4.159574468085107, "4.16"),
            (1.5, "1.5"),
            (3.2672872340425534, "3.267"),
            (1.0, "1"),
            (0.0001234, "0.0001234"),
            (0.00001234, "1.234e-05"),
            (1234.5, "1234"),
            (12345.0, "1.234e+04"),
        ] {
            assert_eq!(format_g(v, 4), s, "{v}");
        }
    }
}
