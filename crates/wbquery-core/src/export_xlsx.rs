//! Write query results to an Excel workbook: a Query sheet, then one sheet per result table.
//!
//! The workbook is first laid out as a [`SheetModel`] list (values, styles, widths), then written;
//! tests check the layout without reading styles back from the file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use rust_xlsxwriter::{Color, Format, FormatAlign, FormatBorder, FormatPattern, Workbook};

use crate::cell::Cell;
use crate::conditions::text_color;
use crate::error::{Error, Result};
use crate::pyfmt::casefold;
use crate::query::QueryResult;
use crate::report::{SOURCE_HEADERS, Value, source_rows, summary_rows};

const HEADER_FILL: u32 = 0xDDE6F0;
const DATETIME_FORMAT: &str = "yyyy-mm-dd hh:mm:ss";
static BAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\[\]:*?/\\]").unwrap());

/// An Excel-safe, unique (ignoring case) sheet name of at most 31 characters; records it in `taken`.
pub fn sheet_name(name: &str, taken: &mut HashSet<String>) -> String {
    let cleaned = BAD.replace_all(name, "_");
    let cleaned = cleaned.trim_matches('\'');
    let base: String = if cleaned.is_empty() { "Sheet" } else { cleaned }.chars().take(31).collect();
    let mut out = base.clone();
    let mut n = 2;
    while taken.contains(&casefold(&out)) {
        let suffix = format!(" ({n})");
        let keep = 31usize.saturating_sub(suffix.chars().count());
        out = base.chars().take(keep).collect::<String>() + &suffix;
        n += 1;
    }
    taken.insert(casefold(&out));
    out
}

/// How a cell is styled.
#[derive(Debug, Clone, PartialEq)]
pub enum Style {
    Plain,
    /// A table header: bold on the header fill, thin borders, centered.
    Header,
    /// A label in the Query sheet: bold, top-aligned.
    Label,
    /// Bold only.
    Bold,
    /// Wrapped and top-aligned (the description).
    Wrap,
    /// A highlighted result cell: this fill, with black or white text.
    Fill(String),
}

/// One worksheet as it will be written.
#[derive(Debug, Clone, PartialEq)]
pub struct SheetModel {
    pub name: String,
    pub rows: Vec<Vec<(Cell, Style)>>,
    pub widths: Vec<f64>,
    /// Freeze the header row and filter the table (result sheets).
    pub table: bool,
}

/// Column widths: the longest value in the first 500 rows plus 2, between 8 and `max_width`.
pub fn column_widths(rows: &[Vec<(Cell, Style)>], max_width: f64) -> Vec<f64> {
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    (0..ncols)
        .map(|j| {
            let longest = rows
                .iter()
                .take(500)
                .filter_map(|r| r.get(j))
                .filter(|(c, _)| !c.is_empty())
                .map(|(c, _)| c.py_str().chars().count())
                .max()
                .unwrap_or(8);
            max_width.min(8f64.max(longest as f64 + 2.0))
        })
        .collect()
}

fn value_cell(v: &Value) -> Cell {
    match v {
        Value::Text(s) => Cell::Text(s.clone()),
        Value::Int(i) => Cell::Int(*i),
    }
}

/// The workbook's sheets: Query first, then the result tables.
pub fn workbook_model(result: &QueryResult) -> Vec<SheetModel> {
    let mut taken: HashSet<String> = ["query".to_string()].into();
    let mut sheets = Vec::new();

    let mut q: Vec<Vec<(Cell, Style)>> = Vec::new();
    for (label, value) in summary_rows(result) {
        let style = if label == "Description" { Style::Wrap } else { Style::Plain };
        q.push(vec![(Cell::Text(label), Style::Label), (value_cell(&value), style)]);
    }
    if !result.warnings.is_empty() {
        q.push(Vec::new());
        for w in &result.warnings {
            q.push(vec![(Cell::Text("Warning".into()), Style::Bold), (Cell::Text(w.clone()), Style::Plain)]);
        }
    }
    q.push(Vec::new());
    q.push(SOURCE_HEADERS.iter().map(|h| (Cell::Text((*h).into()), Style::Header)).collect());
    for r in source_rows(result) {
        q.push(r.iter().map(|v| (value_cell(v), Style::Plain)).collect());
    }
    let widths = column_widths(&q, 80.0);
    sheets.push(SheetModel { name: "Query".into(), rows: q, widths, table: false });

    for t in &result.tables {
        let f = &t.frame;
        let mut rows: Vec<Vec<(Cell, Style)>> =
            vec![f.columns.iter().map(|c| (Cell::Text(c.name.clone()), Style::Header)).collect()];
        for i in 0..f.len() {
            rows.push(
                f.columns
                    .iter()
                    .enumerate()
                    .map(|(j, c)| {
                        let color = &t.colors[j][i];
                        let style = if color.is_empty() { Style::Plain } else { Style::Fill(color.clone()) };
                        (c.cells[i].clone(), style)
                    })
                    .collect(),
            );
        }
        let widths = column_widths(&rows, 60.0);
        sheets.push(SheetModel { name: sheet_name(&t.name, &mut taken), rows, widths, table: true });
    }
    sheets
}

fn hex(color: &str) -> u32 {
    u32::from_str_radix(color.trim_start_matches('#'), 16).unwrap_or(0)
}

fn format_for(style: &Style, date: bool) -> Format {
    let mut f = Format::new();
    if date {
        f = f.set_num_format(DATETIME_FORMAT);
    }
    match style {
        Style::Plain => f,
        Style::Header => f
            .set_bold()
            .set_background_color(Color::RGB(HEADER_FILL))
            .set_pattern(FormatPattern::Solid)
            .set_border(FormatBorder::Thin)
            .set_align(FormatAlign::Center)
            .set_align(FormatAlign::Top),
        Style::Label => f.set_bold().set_align(FormatAlign::Top),
        Style::Bold => f.set_bold(),
        Style::Wrap => f.set_text_wrap().set_align(FormatAlign::Top),
        Style::Fill(c) => f
            .set_background_color(Color::RGB(hex(c)))
            .set_pattern(FormatPattern::Solid)
            .set_font_color(Color::RGB(hex(text_color(c)))),
    }
}

/// `path` with an `.xlsx` suffix (replacing any other suffix).
pub fn xlsx_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("xlsx")) {
        path.to_path_buf()
    } else {
        path.with_extension("xlsx")
    }
}

fn xlsx_err(path: &Path, e: rust_xlsxwriter::XlsxError) -> Error {
    match e {
        rust_xlsxwriter::XlsxError::IoError(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            Error::msg(format!(
                "{}: permission denied (is the file open in Excel? Close it and try again)",
                path.display()
            ))
        }
        other => Error::msg(format!("{}: {other}", path.display())),
    }
}

/// Write the workbook; returns the path written (always ending in `.xlsx`).
pub fn write_xlsx(result: &QueryResult, path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = xlsx_path(path);
    let err = |e| xlsx_err(&path, e);
    let mut wb = Workbook::new();
    for sheet in workbook_model(result) {
        let ws = wb.add_worksheet();
        ws.set_name(&sheet.name).map_err(err)?;
        for (i, row) in sheet.rows.iter().enumerate() {
            let r = i as u32;
            for (j, (cell, style)) in row.iter().enumerate() {
                let c = j as u16;
                let is_date = matches!(cell, Cell::Date(_));
                let fmt = format_for(style, is_date);
                match cell {
                    Cell::Empty => {
                        if *style != Style::Plain {
                            ws.write_blank(r, c, &fmt).map_err(err)?;
                        }
                    }
                    Cell::Bool(b) => drop(ws.write_boolean_with_format(r, c, *b, &fmt).map_err(err)?),
                    Cell::Int(n) => drop(ws.write_number_with_format(r, c, *n as f64, &fmt).map_err(err)?),
                    Cell::Float(f) => drop(ws.write_number_with_format(r, c, *f, &fmt).map_err(err)?),
                    Cell::Text(s) => drop(ws.write_string_with_format(r, c, s, &fmt).map_err(err)?),
                    Cell::Date(d) => drop(ws.write_datetime_with_format(r, c, d, &fmt).map_err(err)?),
                }
            }
        }
        for (j, w) in sheet.widths.iter().enumerate() {
            ws.set_column_width(j as u16, *w).map_err(err)?;
        }
        if sheet.table {
            ws.set_freeze_panes(1, 0).map_err(err)?;
            let nrows = sheet.rows.len();
            let ncols = sheet.rows.first().map_or(0, Vec::len);
            if nrows > 1 || ncols > 1 {
                ws.autofilter(0, 0, (nrows.max(1) - 1) as u32, (ncols.max(1) - 1) as u16).map_err(err)?;
            }
        }
    }
    wb.save(&path).map_err(err)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        let mut taken: HashSet<String> = ["query".to_string()].into();
        assert_eq!(sheet_name("book/Sheet1", &mut taken), "book_Sheet1");
        assert_eq!(sheet_name("BOOK/Sheet1", &mut taken), "BOOK_Sheet1 (2)");
        assert_eq!(sheet_name(&"x".repeat(40), &mut taken).chars().count(), 31);
        assert_eq!(sheet_name("Query", &mut taken), "Query (2)");
    }
}
