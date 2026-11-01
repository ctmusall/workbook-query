//! Cells, columns and frames: the small table model the engine works on.
//!
//! The Python version used pandas. What matters for identical behavior is kept here: each column
//! has a dtype (`int`, `float`, `bool`, `datetime` or `object`) that decides how its cells read as
//! text, and how it changes when rows are joined or stacked (an int column with gaps turns float).

use chrono::{NaiveDateTime, Timelike};

use crate::pyfmt::{py_float_repr, py_str_datetime};

/// One cell value. `Empty` is a blank (pandas NaN / None / NaT); floats are never NaN.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Empty,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Date(NaiveDateTime),
}

impl Cell {
    /// A float cell, or `Empty` for NaN.
    pub fn float(f: f64) -> Cell {
        if f.is_nan() { Cell::Empty } else { Cell::Float(f) }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Cell::Empty)
    }

    /// Python's `str()` of the value (`50000000.0`, `True`, `2024-01-05 00:00:00`; `nan` for blank).
    pub fn py_str(&self) -> String {
        match self {
            Cell::Empty => "nan".into(),
            Cell::Bool(true) => "True".into(),
            Cell::Bool(false) => "False".into(),
            Cell::Int(i) => i.to_string(),
            Cell::Float(f) => py_float_repr(*f),
            Cell::Text(s) => s.clone(),
            Cell::Date(d) => py_str_datetime(d),
        }
    }

    /// Blank: empty, or text that is only whitespace.
    pub fn is_blank(&self) -> bool {
        match self {
            Cell::Empty => true,
            Cell::Text(s) => s.trim().is_empty(),
            _ => false,
        }
    }

    /// How the cell shows in a table or report: dates without a midnight time, blanks as "".
    pub fn display(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Date(d) if is_midnight(d) => d.format("%Y-%m-%d").to_string(),
            other => other.py_str(),
        }
    }
}

pub fn is_midnight(d: &NaiveDateTime) -> bool {
    d.hour() == 0 && d.minute() == 0 && d.second() == 0 && d.nanosecond() == 0
}

/// A column's storage type, as pandas would have it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DType {
    Int,
    Float,
    Bool,
    Date,
    Object,
}

impl DType {
    pub fn is_numeric(self) -> bool {
        matches!(self, DType::Int | DType::Float | DType::Bool)
    }
}

/// The dtype pandas infers for these cells (`infer_objects`); converts ints to floats when the
/// column turns float. `all_empty` is the dtype of a column with no values at all.
pub fn infer_dtype(cells: &mut [Cell], all_empty: DType) -> DType {
    let (mut ints, mut floats, mut bools, mut dates, mut other, mut empty) = (0, 0, 0, 0, 0, 0);
    for c in cells.iter() {
        match c {
            Cell::Empty => empty += 1,
            Cell::Int(_) => ints += 1,
            Cell::Float(_) => floats += 1,
            Cell::Bool(_) => bools += 1,
            Cell::Date(_) => dates += 1,
            Cell::Text(_) => other += 1,
        }
    }
    let values = cells.len() - empty;
    if values == 0 {
        return all_empty;
    }
    if ints + floats == values {
        if floats == 0 && empty == 0 {
            return DType::Int;
        }
        to_float(cells);
        return DType::Float;
    }
    if bools == values && empty == 0 {
        return DType::Bool;
    }
    if dates == values {
        return DType::Date;
    }
    let _ = other;
    DType::Object
}

fn to_float(cells: &mut [Cell]) {
    for c in cells.iter_mut() {
        if let Cell::Int(i) = c {
            *c = Cell::Float(*i as f64);
        }
    }
}

/// The dtype a column takes once some of its rows are blank (a join that found no match, or a
/// stacked table that lacks the column): ints turn float, bools turn object.
pub fn with_gaps(dtype: DType, cells: &mut [Cell]) -> DType {
    match dtype {
        DType::Int => {
            to_float(cells);
            DType::Float
        }
        DType::Bool => DType::Object,
        other => other,
    }
}

/// A named column of cells.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub dtype: DType,
    pub cells: Vec<Cell>,
}

impl Column {
    pub fn new(name: impl Into<String>, dtype: DType, cells: Vec<Cell>) -> Column {
        Column { name: name.into(), dtype, cells }
    }

    /// A column whose dtype is inferred from its cells (as `pd.Series(list)` would).
    pub fn inferred(name: impl Into<String>, mut cells: Vec<Cell>) -> Column {
        let dtype = infer_dtype(&mut cells, DType::Object);
        Column { name: name.into(), dtype, cells }
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// pandas `astype(str)` of each cell: a datetime column whose times are all midnight shows dates only.
    pub fn as_text(&self) -> Vec<String> {
        if self.dtype == DType::Date {
            let date_only = self.cells.iter().all(|c| match c {
                Cell::Date(d) => is_midnight(d),
                _ => true,
            });
            return self
                .cells
                .iter()
                .map(|c| match c {
                    Cell::Date(d) if date_only => d.format("%Y-%m-%d").to_string(),
                    Cell::Empty => "NaT".into(),
                    other => other.py_str(),
                })
                .collect();
        }
        self.cells.iter().map(Cell::py_str).collect()
    }

    /// The cells at `rows`, in that order.
    pub fn take(&self, rows: &[usize]) -> Column {
        Column::new(self.name.clone(), self.dtype, rows.iter().map(|&i| self.cells[i].clone()).collect())
    }
}

/// A table: named columns of equal length plus each row's line number in the source file
/// (the Excel row, or the CSV line).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub columns: Vec<Column>,
    pub index: Vec<i64>,
}

impl Frame {
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn names(&self) -> Vec<String> {
        self.columns.iter().map(|c| c.name.clone()).collect()
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// The rows at `rows`, in that order, with their line numbers.
    pub fn take(&self, rows: &[usize]) -> Frame {
        Frame {
            columns: self.columns.iter().map(|c| c.take(rows)).collect(),
            index: rows.iter().map(|&i| self.index[i]).collect(),
        }
    }

    /// One row's cells.
    pub fn row(&self, i: usize) -> Vec<&Cell> {
        self.columns.iter().map(|c| &c.cells[i]).collect()
    }
}
