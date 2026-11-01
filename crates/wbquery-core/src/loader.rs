//! Load tables from Excel workbooks or CSV files.
//!
//! Column names come from, in order: a sibling `Column Headers (<sheet>)` tab; the first row when
//! it looks like a header; otherwise `Column 1`, `Column 2`, … Fully blank rows, and rows that
//! repeat the header further down, are dropped. Every row keeps its line number in the source
//! file (the Excel row number, or the CSV line number).

use std::path::Path;
use std::sync::LazyLock;

use calamine::{Data, DataType, Reader, open_workbook_auto};
use regex::Regex;

use crate::cell::{Cell, Column, DType, Frame, infer_dtype};
use crate::dates;
use crate::error::{Error, Result};
use crate::pyfmt::{casefold, pd_numeric, py_float};

pub const HIDDEN_SHEETS: &[&str] = &["MC Tags"];
pub const PREFERRED_SHEET: &str = "Summary";
pub const PREFERRED_DATE_COLUMN: &str = "Date Processed";
const CSV_SUFFIXES: &[&str] = &["csv", "txt", "tsv"];

/// Text that pandas reads as a blank cell.
const NA_VALUES: &[&str] = &[
    "", "#N/A", "#N/A N/A", "#NA", "-1.#IND", "-1.#QNAN", "-NaN", "-nan", "1.#IND", "1.#QNAN", "<NA>", "N/A",
    "NA", "NULL", "NaN", "None", "n/a", "nan", "null",
];

static HEADER_TAB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Column Headers \((?P<sheet>.+)\)$").unwrap());
static DEDUPE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" \(\d+\)$").unwrap());
static LEADING_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([+-]?\d[\d,]*(?:\.\d+)?%?)\s+\S").unwrap());

fn suffix(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn is_csv(path: impl AsRef<Path>) -> bool {
    CSV_SUFFIXES.contains(&suffix(path.as_ref()).as_str())
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn not_found(path: &Path) -> Error {
    Error::NotFound(format!("File not found: {}", path.display()))
}

/// The data-bearing sheet names, the preferred sheet (`Summary`) first.
pub fn list_sheets(path: impl AsRef<Path>) -> Result<Vec<String>> {
    let path = path.as_ref();
    if is_csv(path) {
        return Ok(vec![stem(path)]);
    }
    if !path.exists() {
        return Err(not_found(path));
    }
    let wb = open_workbook_auto(path).map_err(|e| Error::msg(e.to_string()))?;
    let mut data: Vec<String> = wb
        .sheet_names()
        .into_iter()
        .filter(|n| !HEADER_TAB.is_match(n) && !HIDDEN_SHEETS.contains(&n.as_str()))
        .collect();
    if let Some(i) = data.iter().position(|n| n == PREFERRED_SHEET) {
        let s = data.remove(i);
        data.insert(0, s);
    }
    Ok(data)
}

/// Load a sheet (or CSV) with resolved column names. `sheet` None means the first data sheet.
pub fn load_table(path: impl AsRef<Path>, sheet: Option<&str>) -> Result<Frame> {
    let path = path.as_ref();
    if !path.exists() {
        return Err(not_found(path));
    }
    if is_csv(path) {
        return load_csv(path);
    }
    let mut wb = open_workbook_auto(path).map_err(|e| Error::msg(e.to_string()))?;
    let all = wb.sheet_names();
    let sheet = match sheet {
        Some(s) => s.to_string(),
        None => {
            list_sheets(path)?.into_iter().next().ok_or_else(|| Error::msg("The workbook has no sheets"))?
        }
    };
    if !all.contains(&sheet) {
        return Err(Error::msg(format!("Sheet '{sheet}' not found. Available: {}", all.join(", "))));
    }
    let raw = read_grid(&mut wb, &sheet)?;
    let width = raw.iter().map(Vec::len).max().unwrap_or(0);
    let header_tab =
        all.iter().find(|n| HEADER_TAB.captures(n).is_some_and(|m| m["sheet"] == sheet)).cloned();

    // Columns as pandas reads them with header=None: dtype inferred over every row.
    let mut columns: Vec<Column> = (0..width)
        .map(|j| {
            let mut cells: Vec<Cell> = raw.iter().map(|r| r.get(j).cloned().unwrap_or(Cell::Empty)).collect();
            let dtype = infer_dtype(&mut cells, DType::Float);
            Column::new("", dtype, cells)
        })
        .collect();
    let mut index: Vec<i64> = (1..=raw.len() as i64).collect();
    let first_row =
        |columns: &[Column]| -> Vec<Cell> { columns.iter().map(|c| c.cells[0].clone()).collect() };

    let names: Vec<Option<String>>;
    if let Some(tab) = header_tab {
        let tab_grid = read_grid(&mut wb, &tab)?;
        let mut tab_names: Vec<Option<String>> =
            tab_grid.first().map(|r| r.iter().map(header_name).collect()).unwrap_or_default();
        while tab_names.len() < width {
            tab_names.push(None);
        }
        while columns.len() < tab_names.len() {
            columns.push(Column::new("", DType::Float, vec![Cell::Empty; index.len()]));
        }
        names = tab_names;
        if !index.is_empty() && row_looks_like_header(&first_row(&columns)) {
            drop_first(&mut columns, &mut index);
        }
    } else if !index.is_empty() && row_looks_like_header(&first_row(&columns)) {
        names = first_row(&columns).iter().map(header_name).collect();
        drop_first(&mut columns, &mut index);
    } else {
        names = vec![None; width];
    }
    for (c, n) in columns.iter_mut().zip(dedupe(&names)) {
        c.name = n;
    }
    let mut frame = drop_repeated_headers(drop_blank_rows(Frame { columns, index }));
    for c in frame.columns.iter_mut().filter(|c| c.dtype == DType::Object) {
        c.dtype = infer_dtype(&mut c.cells, DType::Object);
    }
    Ok(frame)
}

fn drop_first(columns: &mut [Column], index: &mut Vec<i64>) {
    for c in columns.iter_mut() {
        c.cells.remove(0);
    }
    index.remove(0);
}

/// A sheet's cells from A1 to the last non-blank row and column, as pandas reads them.
fn read_grid<RS: std::io::Read + std::io::Seek>(
    wb: &mut calamine::Sheets<RS>,
    sheet: &str,
) -> Result<Vec<Vec<Cell>>> {
    let range = wb.worksheet_range(sheet).map_err(|e| Error::msg(e.to_string()))?;
    let Some((r0, c0)) = range.start() else { return Ok(Vec::new()) };
    let (r0, c0) = (r0 as usize, c0 as usize);
    let mut grid: Vec<Vec<Cell>> = vec![Vec::new(); r0];
    for row in range.rows() {
        let mut out = vec![Cell::Empty; c0];
        out.extend(row.iter().map(excel_cell));
        while out.last().is_some_and(Cell::is_empty) {
            out.pop();
        }
        grid.push(out);
    }
    while grid.last().is_some_and(|r| r.is_empty()) {
        grid.pop();
    }
    Ok(grid)
}

fn excel_cell(d: &Data) -> Cell {
    match d {
        Data::Empty | Data::Error(_) => Cell::Empty,
        Data::Int(i) => Cell::Int(*i),
        Data::Float(f) => number(*f),
        Data::Bool(b) => Cell::Bool(*b),
        Data::String(s) => text_cell(s),
        Data::DateTime(dt) => {
            if dt.is_duration() {
                number(dt.as_f64())
            } else {
                d.as_datetime().map(Cell::Date).unwrap_or_else(|| number(dt.as_f64()))
            }
        }
        Data::DateTimeIso(s) => d.as_datetime().map(Cell::Date).unwrap_or_else(|| text_cell(s)),
        Data::DurationIso(s) => text_cell(s),
    }
}

/// A number cell: whole numbers read as ints, as pandas does.
fn number(f: f64) -> Cell {
    if f.fract() == 0.0 && f.abs() < 9.007_199_254_740_992e15 { Cell::Int(f as i64) } else { Cell::float(f) }
}

fn text_cell(s: &str) -> Cell {
    if NA_VALUES.contains(&s) { Cell::Empty } else { Cell::Text(s.to_string()) }
}

fn header_name(c: &Cell) -> Option<String> {
    if c.is_empty() { None } else { Some(c.py_str()) }
}

/// At least 80% of the row's non-blank cells are text that isn't a number.
fn row_looks_like_header(row: &[Cell]) -> bool {
    let values: Vec<&Cell> = row.iter().filter(|c| !c.is_empty()).collect();
    if values.is_empty() {
        return false;
    }
    let strings = values.iter().filter(|c| matches!(c, Cell::Text(s) if pd_numeric(s).is_none())).count();
    strings as f64 / values.len() as f64 >= 0.8
}

/// Column names: blank ones become `Column N`, repeats `name (2)`, `name (3)`, …
pub fn dedupe(names: &[Option<String>]) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut name = match n.as_deref().map(str::trim) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => format!("Column {}", i + 1),
            };
            let count = seen.entry(name.clone()).or_insert(0);
            *count += 1;
            if *count > 1 {
                name = format!("{name} ({count})");
            }
            name
        })
        .collect()
}

fn drop_blank_rows(frame: Frame) -> Frame {
    let keep: Vec<usize> =
        (0..frame.len()).filter(|&i| frame.columns.iter().any(|c| !c.cells[i].is_empty())).collect();
    if keep.len() == frame.len() { frame } else { frame.take(&keep) }
}

/// Drop rows that repeat the header: at least three non-blank cells, 80% of them equal to their
/// column's name (`Chest 1` under `Chest 1 (2)` counts too).
fn drop_repeated_headers(frame: Frame) -> Frame {
    if frame.is_empty() {
        return frame;
    }
    let names: Vec<[String; 2]> = frame
        .columns
        .iter()
        .map(|c| [casefold(c.name.trim()), casefold(DEDUPE_SUFFIX.replace(&c.name, "").trim())])
        .collect();
    let keep: Vec<usize> = (0..frame.len())
        .filter(|&i| {
            let cells: Vec<(String, &[String; 2])> = frame
                .columns
                .iter()
                .zip(&names)
                .filter(|(c, _)| !c.cells[i].is_empty() && !c.cells[i].py_str().trim().is_empty())
                .map(|(c, n)| (casefold(c.cells[i].py_str().trim()), n))
                .collect();
            let same = cells.iter().filter(|(v, n)| n.contains(v)).count();
            !(cells.len() >= 3 && same as f64 >= 0.8 * cells.len() as f64)
        })
        .collect();
    if keep.len() == frame.len() { frame } else { frame.take(&keep) }
}

// --- CSV ---------------------------------------------------------------------------

fn load_csv(path: &Path) -> Result<Frame> {
    let bytes = std::fs::read(path).map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let delimiter = if suffix(path) == "tsv" { b'\t' } else { sniff_delimiter(text) };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut header: Option<Vec<String>> = None;
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut index: Vec<i64> = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
        let fields: Vec<String> = record.iter().map(str::to_string).collect();
        if header.is_none() {
            header = Some(fields);
        } else {
            // The line the record starts on (counting blank lines, which pandas keeps as rows).
            let mut byte = record.position().map_or(0, |p| p.byte() as usize).min(text.len());
            while matches!(text.as_bytes().get(byte), Some(b'\n' | b'\r')) {
                byte += 1; // the reader's position includes the blank lines it skipped
            }
            index.push(
                1 + text.as_bytes()[..byte.min(text.len())].iter().filter(|b| **b == b'\n').count() as i64,
            );
            rows.push(fields);
        }
    }
    let header = header.unwrap_or_default();
    let names: Vec<Option<String>> = header.iter().map(|h| Some(h.clone())).collect();
    let columns = dedupe(&names)
        .into_iter()
        .enumerate()
        .map(|(j, name)| {
            let raw: Vec<Option<&str>> = rows
                .iter()
                .map(|r| r.get(j).map(String::as_str).filter(|s| !NA_VALUES.contains(s)))
                .collect();
            let (dtype, cells) = csv_column(&raw);
            Column::new(name, dtype, cells)
        })
        .collect();
    Ok(drop_repeated_headers(drop_blank_rows(Frame { columns, index })))
}

/// One CSV column's values as pandas types them: all ints, all numbers, all booleans, or text.
fn csv_column(raw: &[Option<&str>]) -> (DType, Vec<Cell>) {
    let values: Vec<&str> = raw.iter().flatten().copied().collect();
    let has_blank = values.len() < raw.len();
    let map = |f: &dyn Fn(&str) -> Cell| raw.iter().map(|v| v.map_or(Cell::Empty, f)).collect::<Vec<Cell>>();
    if values.is_empty() {
        return (DType::Float, vec![Cell::Empty; raw.len()]);
    }
    let is_int = |s: &str| {
        let t = s.strip_prefix(['+', '-']).unwrap_or(s);
        !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && s.parse::<i64>().is_ok()
    };
    if values.iter().all(|s| is_int(s)) {
        if has_blank {
            return (DType::Float, map(&|s| Cell::Float(s.parse::<i64>().unwrap() as f64)));
        }
        return (DType::Int, map(&|s| Cell::Int(s.parse().unwrap())));
    }
    let is_float = |s: &str| !s.contains('_') && s.trim() == s && py_float(s).is_some();
    if values.iter().all(|s| is_float(s)) {
        return (DType::Float, map(&|s| Cell::float(py_float(s).unwrap())));
    }
    let as_bool = |s: &str| match s {
        "True" | "TRUE" | "true" => Some(true),
        "False" | "FALSE" | "false" => Some(false),
        _ => None,
    };
    if values.iter().all(|s| as_bool(s).is_some()) {
        let dtype = if has_blank { DType::Object } else { DType::Bool };
        return (dtype, map(&|s| Cell::Bool(as_bool(s).unwrap())));
    }
    (DType::Object, map(&|s| Cell::Text(s.to_string())))
}

/// The delimiter that splits the first lines most consistently (like Python's `csv.Sniffer`).
fn sniff_delimiter(text: &str) -> u8 {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).take(20).collect();
    let mut best = (b',', 0usize, false);
    for d in *b",\t;|:" {
        let counts: Vec<usize> = lines.iter().map(|l| l.bytes().filter(|b| *b == d).count()).collect();
        let Some(&first) = counts.first() else { continue };
        let consistent = first > 0 && counts.iter().all(|&c| c == first);
        if consistent && (!best.2 || first > best.1) || (!best.2 && first > best.1) {
            best = (d, first, consistent);
        }
    }
    best.0
}

// --- reading cells as numbers and dates --------------------------------------------

/// The count at the start of a cell such as `1 Micrococcus luteus` (-> `(1.0, "Micrococcus luteus")`):
/// a number followed by whitespace and then any text. None for `TNTC`, `<1`, `2026-01-05`, `5/2`.
pub fn leading_number(text: &str) -> Option<(f64, String)> {
    let m = LEADING_NUMBER.captures(text)?;
    let g = m.get(1)?;
    let num = g.as_str().replace(',', "");
    let value = match num.strip_suffix('%') {
        Some(n) => py_float(n)? / 100.0,
        None => py_float(&num)?,
    };
    Some((value, text[g.end()..].trim().to_string()))
}

/// One text value as a float: `1,000`, `5e7`, `50%`, or a leading number; NaN otherwise.
pub fn text_number(v: &str) -> f64 {
    let s = v.trim().replace(',', "");
    if s.is_empty() {
        return f64::NAN;
    }
    let parsed = match s.strip_suffix('%') {
        Some(n) => py_float(n).map(|f| f / 100.0),
        None => py_float(&s),
    };
    parsed.or_else(|| leading_number(v).map(|(f, _)| f)).unwrap_or(f64::NAN)
}

/// One cell as a float (see [`text_number`]); NaN when it isn't a number.
pub fn cell_number(c: &Cell) -> f64 {
    match c {
        Cell::Empty | Cell::Date(_) => f64::NAN,
        Cell::Bool(b) => f64::from(u8::from(*b)),
        Cell::Int(i) => *i as f64,
        Cell::Float(f) => *f,
        Cell::Text(s) => text_number(s),
    }
}

/// A column's cells as floats (NaN where they aren't numbers). A datetime column reads as
/// microseconds since 1970, as pandas does.
pub fn to_numeric(col: &Column) -> Vec<f64> {
    if col.dtype == DType::Date {
        return col
            .cells
            .iter()
            .map(|c| match c {
                Cell::Date(d) => d.and_utc().timestamp_micros() as f64,
                _ => f64::NAN,
            })
            .collect();
    }
    col.cells.iter().map(cell_number).collect()
}

/// pandas `to_numeric(errors="coerce")` of one cell: no thousands separators, percents or leading numbers.
fn pd_number(c: &Cell) -> Option<f64> {
    match c {
        Cell::Empty | Cell::Date(_) => None,
        Cell::Bool(b) => Some(f64::from(u8::from(*b))),
        Cell::Int(i) => Some(*i as f64),
        Cell::Float(f) => Some(*f),
        Cell::Text(s) => pd_numeric(s),
    }
}

/// A column's cells as timestamps: dates as they are, Excel serial numbers, or dates written as text.
pub fn to_datetime(col: &Column) -> Vec<Option<chrono::NaiveDateTime>> {
    col.cells
        .iter()
        .map(|c| match c {
            Cell::Date(d) => Some(*d),
            Cell::Empty => None,
            _ if col.dtype == DType::Date => None,
            other => match pd_number(other).filter(|n| (1.0..=dates::MAX_SERIAL).contains(n)) {
                Some(n) => dates::from_serial(n),
                None => dates::parse_datetime(&other.py_str()),
            },
        })
        .collect()
}

/// Columns where at least `threshold` of the non-blank values are numbers (dates excluded).
pub fn numeric_columns(frame: &Frame, threshold: f64) -> Vec<String> {
    frame
        .columns
        .iter()
        .filter(|c| c.dtype != DType::Date)
        .filter(|c| {
            let nonblank: Vec<&Cell> =
                c.cells.iter().filter(|v| !v.is_empty() && !v.py_str().trim().is_empty()).collect();
            if nonblank.is_empty() {
                return false;
            }
            let numbers = nonblank.iter().filter(|v| !cell_number(v).is_nan()).count();
            numbers as f64 / nonblank.len() as f64 >= threshold
        })
        .map(|c| c.name.clone())
        .collect()
}

/// Columns that look like dates, by dtype or by name; `Date Processed` first.
pub fn date_columns(frame: &Frame) -> Vec<String> {
    let mut out: Vec<String> = frame
        .columns
        .iter()
        .filter(|c| c.dtype == DType::Date || c.name.to_lowercase().contains("date"))
        .map(|c| c.name.clone())
        .collect();
    if let Some(i) = out.iter().position(|n| n == PREFERRED_DATE_COLUMN) {
        let n = out.remove(i);
        out.insert(0, n);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_names() {
        let names = [Some("A".into()), None, Some(" A ".into()), Some("".into()), Some("B".into())];
        assert_eq!(dedupe(&names), ["A", "Column 2", "A (2)", "Column 4", "B"]);
    }

    #[test]
    fn sniff() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3\n"), b';');
        assert_eq!(sniff_delimiter("a,b\n1,2\n"), b',');
        assert_eq!(sniff_delimiter("a\tb\n1\t2\n"), b'\t');
        assert_eq!(sniff_delimiter("Name;Count;Ratio\nA;1;0.5\n"), b';');
    }
}
