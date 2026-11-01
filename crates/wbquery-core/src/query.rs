//! A query over several workbook sheets, and running it.
//!
//! Two modes:
//!
//! * `independent`: each source is filtered on its own. A condition naming another source is left
//!   out for this one, as is an any-sheet condition on a column this source lacks. When the query
//!   has no active conditions every source keeps all its rows; otherwise a source that none of them
//!   apply to matches no rows (a warning names it). Each source with output columns gives one result
//!   table; when there are several, an "All results" table stacking them comes first.
//! * `join`: rows are linked across sources by each source's key column, and the whole condition
//!   tree is tested against the joined row. One result table.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde_json::{Map, Value, json};

use crate::cell::{Cell, Column, DType, Frame, with_gaps};
use crate::conditions::{
    ANY_SOURCE, CellHit, Condition, Group, cell_hits, evaluate_group, resolve_column, truthy,
};
use crate::error::{Error, Result};
use crate::loader;
use crate::proof::file_sha256;
use crate::pyfmt::{casefold, json_ensure_ascii, py_float, py_float_repr};

pub const MODES: &[&str] = &["independent", "join"];
pub const JOIN_HOW: &[&str] = &["inner", "left", "outer"];
pub const FILE_COL: &str = "File";
pub const SHEET_COL: &str = "Sheet";
pub const ROW_COL: &str = "Excel row";
pub const KEY_COL: &str = "Key";
pub const JOINED: &str = "Joined";
pub const SOURCE_COL: &str = "Source";
pub const COMBINED: &str = "All results";
pub const FORMAT_VERSION: i64 = 1;
const BLANK_PREFIX: &str = "\0blank:";

/// A joined row: the left and right row it came from (None where that side had no match).
type Pair = (Option<usize>, Option<usize>);
type Pick<'a> = &'a dyn Fn(&Pair) -> Option<usize>;

/// The file / sheet / row columns a query adds to its result tables.
pub fn is_provenance(col: &str) -> bool {
    [SOURCE_COL, FILE_COL, SHEET_COL, ROW_COL].contains(&col) || col.ends_with(&format!(" {ROW_COL}"))
}

/// One workbook sheet (or CSV file) in a query, under a short name (`alias`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Source {
    pub alias: String,
    pub path: String,
    pub sheet: String,
    /// The column to join on (join mode).
    pub key: String,
}

impl Source {
    pub fn new(alias: &str, path: &str, sheet: &str, key: &str) -> Source {
        Source { alias: alias.into(), path: path.into(), sheet: sheet.into(), key: key.into() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub sources: Vec<Source>,
    pub r#where: Group,
    /// Output columns per source alias, in order.
    pub outputs: Vec<(String, Vec<String>)>,
    pub mode: String,
    pub join_how: String,
    pub include_provenance: bool,
    /// What the query is called, e.g. "EOJ personnel hits, full year".
    pub name: String,
    /// What it does and why, in the author's words.
    pub description: String,
}

impl Default for Query {
    fn default() -> Query {
        Query {
            sources: Vec::new(),
            r#where: Group::default(),
            outputs: Vec::new(),
            mode: "independent".into(),
            join_how: "inner".into(),
            include_provenance: true,
            name: String::new(),
            description: String::new(),
        }
    }
}

impl Query {
    pub fn source(&self, alias: &str) -> Result<&Source> {
        self.sources
            .iter()
            .find(|s| s.alias == alias)
            .ok_or_else(|| Error::Key(format!("No source named '{alias}'")))
    }

    pub fn source_mut(&mut self, alias: &str) -> Result<&mut Source> {
        self.sources
            .iter_mut()
            .find(|s| s.alias == alias)
            .ok_or_else(|| Error::Key(format!("No source named '{alias}'")))
    }

    /// The output columns chosen for `alias` (empty when none).
    pub fn outputs_for(&self, alias: &str) -> &[String] {
        self.outputs.iter().find(|(a, _)| a == alias).map(|(_, c)| c.as_slice()).unwrap_or(&[])
    }

    pub fn set_outputs(&mut self, alias: &str, cols: Vec<String>) {
        match self.outputs.iter_mut().find(|(a, _)| a == alias) {
            Some(entry) => entry.1 = cols,
            None => self.outputs.push((alias.to_string(), cols)),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut doc = Map::new();
        doc.insert("version".into(), json!(FORMAT_VERSION));
        doc.insert("name".into(), json!(self.name));
        doc.insert("description".into(), json!(self.description));
        doc.insert("mode".into(), json!(self.mode));
        doc.insert("join_how".into(), json!(self.join_how));
        doc.insert("include_provenance".into(), json!(self.include_provenance));
        let sources: Vec<Value> = self
            .sources
            .iter()
            .map(|s| json!({"alias": s.alias, "path": s.path, "sheet": s.sheet, "key": s.key}))
            .collect();
        doc.insert("sources".into(), Value::Array(sources));
        doc.insert("where".into(), self.r#where.to_json());
        let mut outputs = Map::new();
        for (alias, cols) in &self.outputs {
            outputs.insert(alias.clone(), json!(cols));
        }
        doc.insert("outputs".into(), Value::Object(outputs));
        Value::Object(doc)
    }

    /// A query from its saved JSON. Unknown keys are ignored; `name` and `description` may be absent.
    pub fn from_json(doc: &Value) -> Result<Query> {
        let Value::Object(doc) = doc else { return Err(Error::msg("A query must be a JSON object")) };
        let text = |v: &Value| crate::conditions::py_str(v);
        let mut sources = Vec::new();
        if let Some(Value::Array(list)) = doc.get("sources") {
            for s in list {
                let Value::Object(s) = s else {
                    return Err(Error::msg("Each source needs \"alias\", \"path\" and \"sheet\""));
                };
                let (Some(alias), Some(path), Some(sheet)) = (s.get("alias"), s.get("path"), s.get("sheet"))
                else {
                    return Err(Error::msg("Each source needs \"alias\", \"path\" and \"sheet\""));
                };
                let key = s.get("key").filter(|k| truthy(k)).map(text).unwrap_or_default();
                sources.push(Source { alias: text(alias), path: text(path), sheet: text(sheet), key });
            }
        }
        let mut outputs = Vec::new();
        match doc.get("outputs") {
            None => {}
            Some(Value::Object(map)) => {
                for (alias, cols) in map {
                    let cols = match cols {
                        Value::Array(a) => a.iter().map(text).collect(),
                        Value::String(s) => s.chars().map(String::from).collect(),
                        _ => {
                            return Err(Error::msg(
                                "\"outputs\" must map each source alias to a list of columns",
                            ));
                        }
                    };
                    outputs.push((alias.clone(), cols));
                }
            }
            Some(_) => return Err(Error::msg("\"outputs\" must map each source alias to a list of columns")),
        }
        let empty = Value::Object(Map::new());
        let or_text = |key: &str| doc.get(key).filter(|v| truthy(v)).map(text).unwrap_or_default();
        Ok(Query {
            sources,
            r#where: Group::from_json(doc.get("where").unwrap_or(&empty))?,
            outputs,
            mode: doc.get("mode").map(text).unwrap_or_else(|| "independent".into()),
            join_how: doc.get("join_how").map(text).unwrap_or_else(|| "inner".into()),
            include_provenance: doc.get("include_provenance").is_none_or(truthy),
            name: or_text("name"),
            description: or_text("description"),
        })
    }
}

/// One result table with, cell by cell, whether a condition counted there and the highlight color
/// of the first colored condition that did (`""` for none). Both are column-major, like the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultTable {
    pub name: String,
    pub frame: Frame,
    pub hits: Vec<Vec<bool>>,
    pub colors: Vec<Vec<String>>,
}

/// What a run read from one source.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceInfo {
    pub alias: String,
    pub path: String,
    pub sheet: String,
    pub rows: usize,
    pub matched: Option<usize>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub tables: Vec<ResultTable>,
    pub warnings: Vec<String>,
    pub sources: Vec<SourceInfo>,
    pub query: Query,
    pub run_at: NaiveDateTime,
}

impl QueryResult {
    pub fn table(&self, name: &str) -> Option<&ResultTable> {
        self.tables.iter().find(|t| t.name == name)
    }
}

pub fn default_alias(path: &str, sheet: &str) -> String {
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if loader::is_csv(path) || sheet == stem { stem } else { format!("{stem}/{sheet}") }
}

pub fn unique_alias<S: AsRef<str>>(base: &str, taken: &[S]) -> String {
    let taken: HashSet<&str> = taken.iter().map(AsRef::as_ref).collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    let mut n = 2;
    while taken.contains(format!("{base} ({n})").as_str()) {
        n += 1;
    }
    format!("{base} ({n})")
}

/// The query as JSON text, formatted exactly as the Python version wrote it.
pub fn query_to_string(query: &Query) -> String {
    let text = serde_json::to_string_pretty(&query.to_json()).expect("JSON value serializes");
    json_ensure_ascii(&text) + "\n"
}

pub fn save_query(query: &Query, path: impl AsRef<Path>) -> Result<PathBuf> {
    let path = path.as_ref();
    std::fs::write(path, query_to_string(query))
        .map_err(|e| Error::msg(format!("{}: {e}", path.display())))?;
    Ok(path.to_path_buf())
}

pub fn load_query(path: impl AsRef<Path>) -> Result<Query> {
    let path = path.as_ref();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::msg(format!("Query file not found: {}", path.display())));
        }
        Err(e) => return Err(Error::msg(format!("Query file {}: {e}", path.display()))),
    };
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| Error::msg(format!("Query file {} is not valid JSON: {e}", path.display())))?;
    Query::from_json(&doc).map_err(|e| Error::msg(format!("Query file {}: {e}", path.display())))
}

/// Each source's table, read once per (file, sheet).
pub fn load_frames(query: &Query) -> Result<HashMap<String, Frame>> {
    let mut cache: HashMap<(PathBuf, String), Frame> = HashMap::new();
    let mut out = HashMap::new();
    for s in &query.sources {
        let resolved = std::path::absolute(&s.path).unwrap_or_else(|_| PathBuf::from(&s.path));
        let k = (std::fs::canonicalize(&resolved).unwrap_or(resolved), s.sheet.clone());
        if !cache.contains_key(&k) {
            let frame = loader::load_table(&s.path, Some(&s.sheet))
                .map_err(|e| Error::msg(format!("{}: {e}", s.alias)))?;
            cache.insert(k.clone(), frame);
        }
        out.insert(s.alias.clone(), cache[&k].clone());
    }
    Ok(out)
}

/// Problems that stop `query` running, given each source's column names.
pub fn validate(query: &Query, columns: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut errors = Vec::new();
    if !MODES.contains(&query.mode.as_str()) {
        errors.push(format!("Mode must be one of {}, not '{}'", MODES.join(", "), query.mode));
    }
    if query.mode == "join" && !JOIN_HOW.contains(&query.join_how.as_str()) {
        errors.push(format!("Join type must be one of {}, not '{}'", JOIN_HOW.join(", "), query.join_how));
    }
    if query.sources.is_empty() {
        errors.push("Add at least one workbook sheet".into());
    }
    let aliases: Vec<&str> = query.sources.iter().map(|s| s.alias.as_str()).collect();
    let mut dupes: Vec<&str> =
        aliases.iter().copied().filter(|a| aliases.iter().filter(|b| *b == a).count() > 1).collect();
    dupes.sort();
    dupes.dedup();
    for a in dupes {
        errors.push(format!("Source name '{a}' is used more than once"));
    }
    for s in &query.sources {
        if s.alias.trim().is_empty() {
            errors.push(format!("{} [{}]: give the source a name", s.path, s.sheet));
        }
        if query.mode == "join" {
            if s.key.is_empty() {
                errors.push(format!("{}: choose a key column to join on", s.alias));
            } else if !columns.get(&s.alias).is_some_and(|c| c.contains(&s.key)) {
                errors.push(format!("{}: key column '{}' not found", s.alias, s.key));
            }
        }
    }
    for c in query.r#where.conditions(true) {
        if c.source == ANY_SOURCE {
            if columns.values().all(|cols| resolve_column(cols, &c.column).is_none()) {
                errors.push(format!("{}: no sheet has a column '{}'", c.describe(), c.column));
            }
        } else if let Some(cols) = columns.get(&c.source) {
            if resolve_column(cols, &c.column).is_none() {
                errors.push(format!("{}: column '{}' not found in {}", c.describe(), c.column, c.source));
            }
        } else {
            errors.push(format!("{}: no source named '{}'", c.describe(), c.source));
        }
        if let Err(e) = c.check_value() {
            errors.push(e.to_string());
        }
    }
    for (alias, cols) in &query.outputs {
        if cols.is_empty() {
            continue;
        }
        let Some(have) = columns.get(alias) else {
            errors.push(format!("Output columns given for unknown source '{alias}'"));
            continue;
        };
        let missing: Vec<&str> = cols.iter().filter(|c| !have.contains(c)).map(String::as_str).collect();
        if !missing.is_empty() {
            errors.push(format!("{alias}: output column(s) not found: {}", missing.join(", ")));
        }
    }
    if !aliases.iter().any(|a| !query.outputs_for(a).is_empty()) {
        errors.push("Choose at least one output column".into());
    }
    errors
}

/// Run `query`. `frames` (alias -> table) skips re-reading files the caller already loaded.
pub fn run_query(query: &Query, frames: Option<&HashMap<String, Frame>>) -> Result<QueryResult> {
    let loaded;
    let frames = match frames {
        Some(f) => f,
        None => {
            loaded = load_frames(query)?;
            &loaded
        }
    };
    let columns: HashMap<String, Vec<String>> = query
        .sources
        .iter()
        .filter_map(|s| frames.get(&s.alias).map(|f| (s.alias.clone(), f.names())))
        .collect();
    let errors = validate(query, &columns);
    if !errors.is_empty() {
        return Err(Error::msg(errors.join("\n")));
    }
    let mut warnings = Vec::new();
    let (tables, matched) = if query.mode == "join" {
        run_join(query, frames, &mut warnings)?
    } else {
        run_independent(query, frames, &mut warnings)?
    };
    let sources = query
        .sources
        .iter()
        .map(|s| SourceInfo {
            alias: s.alias.clone(),
            path: s.path.clone(),
            sheet: s.sheet.clone(),
            rows: frames[&s.alias].len(),
            matched: matched.get(&s.alias).copied(),
            sha256: file_sha256(&s.path),
        })
        .collect();
    Ok(QueryResult { tables, warnings, sources, query: query.clone(), run_at: Local::now().naive_local() })
}

/// (hits, colors) for `table`, whose rows are `rows` of the frame `found` was computed on.
fn cell_frames(table: &Frame, rows: &[usize], found: &[CellHit<'_>]) -> (Vec<Vec<bool>>, Vec<Vec<String>>) {
    let n = table.len();
    let mut hits = vec![vec![false; n]; table.columns.len()];
    let mut colors = vec![vec![String::new(); n]; table.columns.len()];
    for h in found {
        let Some(j) = table.position(&h.column) else { continue };
        let hit: Vec<bool> = rows.iter().map(|&r| h.rows[r]).collect();
        if !hit.iter().any(|b| *b) {
            continue;
        }
        for (i, &on) in hit.iter().enumerate() {
            if on {
                hits[j][i] = true;
                if !h.condition.color.is_empty() && colors[j][i].is_empty() {
                    colors[j][i] = h.condition.color.clone(); // the first color wins
                }
            }
        }
    }
    (hits, colors)
}

type Tables = (Vec<ResultTable>, BTreeMap<String, usize>);

fn run_independent(
    query: &Query,
    frames: &HashMap<String, Frame>,
    warnings: &mut Vec<String>,
) -> Result<Tables> {
    let mut tables = Vec::new();
    let mut matched = BTreeMap::new();
    let filtering = !query.r#where.conditions(true).is_empty();
    let mut untested = Vec::new();
    for s in &query.sources {
        let df = &frames[&s.alias];
        let names = df.names();
        let column_for = |c: &Condition| -> Vec<&Column> {
            if c.source != ANY_SOURCE && c.source != s.alias {
                return Vec::new();
            }
            resolve_column(&names, &c.column).and_then(|n| df.column(&n)).into_iter().collect()
        };
        let mask = evaluate_group(&query.r#where, &column_for)?;
        let rows: Vec<usize> = match mask {
            None if filtering => {
                untested.push(s.alias.clone());
                Vec::new()
            }
            None => (0..df.len()).collect(),
            Some(m) => m.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect(),
        };
        matched.insert(s.alias.clone(), rows.len());
        let cols = query.outputs_for(&s.alias);
        if cols.is_empty() {
            continue;
        }
        let picked = df.take(&rows);
        let mut out: Vec<Column> = cols.iter().filter_map(|c| picked.column(c).cloned()).collect();
        if query.include_provenance {
            let n = rows.len();
            let file =
                Path::new(&s.path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            let prov = [
                Column::new(FILE_COL, DType::Object, vec![Cell::Text(file); n]),
                Column::new(SHEET_COL, DType::Object, vec![Cell::Text(s.sheet.clone()); n]),
                Column::new(ROW_COL, DType::Int, picked.index.iter().map(|i| Cell::Int(*i)).collect()),
            ];
            out.splice(0..0, prov);
        }
        let frame = Frame { columns: out, index: (0..rows.len() as i64).collect() };
        let found = cell_hits(&query.r#where, &column_for)?;
        let (hits, colors) = cell_frames(&frame, &rows, &found);
        tables.push(ResultTable { name: s.alias.clone(), frame, hits, colors });
    }
    if !untested.is_empty() {
        let n = untested.len();
        let mut names = untested.iter().take(5).cloned().collect::<Vec<_>>().join(", ");
        if n > 5 {
            names.push_str(&format!(" and {} more", n - 5));
        }
        let (s, verb) = if n == 1 { ("", "it matches") } else { ("s", "they match") };
        warnings
            .push(format!("{n} sheet{s} lack every column the conditions test, so {verb} no rows: {names}"));
    }
    if tables.len() >= 2 {
        let combined = combine(&tables, query.include_provenance);
        tables.insert(0, combined);
    }
    Ok((tables, matched))
}

/// Stack the per-source tables (columns matched by name); a column a source lacks is blank for its rows.
fn combine(tables: &[ResultTable], provenance: bool) -> ResultTable {
    let mut names: Vec<String> = Vec::new();
    if provenance {
        names.push(SOURCE_COL.into());
    }
    for t in tables {
        for c in &t.frame.columns {
            if !names.contains(&c.name) {
                names.push(c.name.clone());
            }
        }
    }
    let total: usize = tables.iter().map(|t| t.frame.len()).sum();
    let mut columns = Vec::new();
    let mut hits = Vec::new();
    let mut colors = Vec::new();
    for name in &names {
        let mut cells = Vec::with_capacity(total);
        let mut h = Vec::with_capacity(total);
        let mut k = Vec::with_capacity(total);
        let mut dtypes = Vec::new();
        let mut gaps = false;
        for t in tables {
            let n = t.frame.len();
            if provenance && name == SOURCE_COL {
                cells.extend(std::iter::repeat_n(Cell::Text(t.name.clone()), n));
                h.extend(std::iter::repeat_n(false, n));
                k.extend(std::iter::repeat_n(String::new(), n));
                dtypes.push(DType::Object);
                continue;
            }
            match t.frame.position(name) {
                Some(j) => {
                    cells.extend(t.frame.columns[j].cells.iter().cloned());
                    h.extend(t.hits[j].iter().copied());
                    k.extend(t.colors[j].iter().cloned());
                    dtypes.push(t.frame.columns[j].dtype);
                }
                None => {
                    cells.extend(std::iter::repeat_n(Cell::Empty, n));
                    h.extend(std::iter::repeat_n(false, n));
                    k.extend(std::iter::repeat_n(String::new(), n));
                    gaps = true;
                }
            }
        }
        let dtype = concat_dtype(&dtypes, gaps, &mut cells);
        columns.push(Column::new(name.clone(), dtype, cells));
        hits.push(h);
        colors.push(k);
    }
    ResultTable {
        name: COMBINED.into(),
        frame: Frame { columns, index: (0..total as i64).collect() },
        hits,
        colors,
    }
}

/// The dtype of a column stacked from parts with these dtypes (`gaps`: some parts lack it).
fn concat_dtype(dtypes: &[DType], gaps: bool, cells: &mut [Cell]) -> DType {
    let Some(&first) = dtypes.first() else { return DType::Float };
    let numeric = dtypes.iter().all(|d| matches!(d, DType::Int | DType::Float));
    if numeric && dtypes.contains(&DType::Float) {
        return with_gaps(DType::Int, cells); // ints become floats
    }
    if dtypes.iter().all(|d| *d == first) {
        return if gaps { with_gaps(first, cells) } else { first };
    }
    DType::Object
}

/// Join key for one cell: numbers compare by value (1 == 1.0 == "1"), dates by day, text ignoring
/// case and surrounding spaces. Blank cells get `blank_token` so they never match.
pub fn normalize_key(value: &Cell, blank_token: &str) -> String {
    fn number(f: f64) -> String {
        if f.is_finite() && f.fract() == 0.0 {
            if f == 0.0 { "0".into() } else { format!("{f:.0}") }
        } else {
            py_float_repr(f)
        }
    }
    match value {
        Cell::Empty => blank_token.into(),
        Cell::Date(d) => d.date().format("%Y-%m-%d").to_string(),
        Cell::Bool(b) => if *b { "true" } else { "false" }.into(),
        Cell::Int(i) => number(*i as f64),
        Cell::Float(f) => number(*f),
        Cell::Text(s) => {
            let text = s.trim();
            if text.is_empty() {
                return blank_token.into();
            }
            match py_float(&text.replace(',', "")) {
                Some(f) if f.is_finite() => number(f),
                _ => casefold(text),
            }
        }
    }
}

/// The joined rows of several sources: prefixed columns, which sources each row came from, and its key.
struct Merged {
    columns: Vec<Column>,
    present: Vec<(String, Vec<bool>)>,
    keys: Vec<String>,
}

impl Merged {
    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

fn keyed(s: &Source, df: &Frame, provenance: bool) -> Merged {
    let mut columns = Vec::new();
    if provenance {
        columns.push(Column::new(
            format!("{} {ROW_COL}", s.alias),
            DType::Int,
            df.index.iter().map(|i| Cell::Int(*i)).collect(),
        ));
    }
    for c in &df.columns {
        columns.push(Column::new(format!("{}.{}", s.alias, c.name), c.dtype, c.cells.clone()));
    }
    let key_col = df.column(&s.key).expect("key column validated");
    let keys = key_col
        .cells
        .iter()
        .zip(&df.index)
        .map(|(v, i)| normalize_key(v, &format!("{BLANK_PREFIX}{}:{i}", s.alias)))
        .collect();
    Merged { columns, present: vec![(s.alias.clone(), vec![true; df.len()])], keys }
}

/// pandas `merge(on=key, how=...)`: inner and left keep the left order (each left row followed by its
/// matches); outer sorts by key.
fn merge(left: Merged, right: Merged, how: &str) -> Merged {
    let mut by_key: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, k) in right.keys.iter().enumerate() {
        by_key.entry(k.as_str()).or_default().push(i);
    }
    let mut pairs: Vec<Pair> = Vec::new();
    if how == "outer" {
        let mut left_by_key: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, k) in left.keys.iter().enumerate() {
            left_by_key.entry(k.as_str()).or_default().push(i);
        }
        let mut keys: Vec<&str> = left_by_key.keys().chain(by_key.keys()).copied().collect();
        keys.sort_unstable();
        keys.dedup();
        for k in keys {
            match (left_by_key.get(k), by_key.get(k)) {
                (Some(ls), Some(rs)) => {
                    for &l in ls {
                        pairs.extend(rs.iter().map(|&r| (Some(l), Some(r))));
                    }
                }
                (Some(ls), None) => pairs.extend(ls.iter().map(|&l| (Some(l), None))),
                (None, Some(rs)) => pairs.extend(rs.iter().map(|&r| (None, Some(r)))),
                (None, None) => {}
            }
        }
    } else {
        for (l, k) in left.keys.iter().enumerate() {
            match by_key.get(k.as_str()) {
                Some(rs) => pairs.extend(rs.iter().map(|&r| (Some(l), Some(r)))),
                None if how == "left" => pairs.push((Some(l), None)),
                None => {}
            }
        }
    }
    let side = |cols: Vec<Column>, pick: Pick<'_>| {
        let gaps = pairs.iter().any(|p| pick(p).is_none());
        cols.into_iter()
            .map(|c| {
                let mut cells: Vec<Cell> =
                    pairs.iter().map(|p| pick(p).map_or(Cell::Empty, |i| c.cells[i].clone())).collect();
                let dtype = if gaps && !c.name.ends_with(&format!(" {ROW_COL}")) {
                    with_gaps(c.dtype, &mut cells)
                } else {
                    c.dtype
                };
                Column::new(c.name, dtype, cells)
            })
            .collect::<Vec<_>>()
    };
    let presence = |present: Vec<(String, Vec<bool>)>, pick: Pick<'_>| {
        present
            .into_iter()
            .map(|(a, p)| (a, pairs.iter().map(|x| pick(x).is_some_and(|i| p[i])).collect::<Vec<bool>>()))
            .collect::<Vec<_>>()
    };
    let keys = pairs
        .iter()
        .map(|(l, r)| match (l, r) {
            (Some(l), _) => left.keys[*l].clone(),
            (None, Some(r)) => right.keys[*r].clone(),
            (None, None) => unreachable!(),
        })
        .collect();
    let mut columns = side(left.columns, &|p| p.0);
    columns.extend(side(right.columns, &|p| p.1));
    let mut present = presence(left.present, &|p| p.0);
    present.extend(presence(right.present, &|p| p.1));
    Merged { columns, present, keys }
}

fn run_join(query: &Query, frames: &HashMap<String, Frame>, warnings: &mut Vec<String>) -> Result<Tables> {
    let mut merged: Option<Merged> = None;
    for s in &query.sources {
        let part = keyed(s, &frames[&s.alias], query.include_provenance);
        let real: Vec<&String> = part.keys.iter().filter(|k| !k.starts_with(BLANK_PREFIX)).collect();
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for k in &real {
            *counts.entry(k.as_str()).or_default() += 1;
        }
        let dupes = counts.values().filter(|n| **n > 1).count();
        if dupes > 0 {
            warnings.push(format!(
                "{}: {dupes} key value(s) in '{}' appear on more than one row; each such row is paired with every \
                 match in the other sheets",
                s.alias, s.key
            ));
        }
        let blanks = part.keys.len() - real.len();
        if blanks > 0 {
            warnings.push(format!("{}: {blanks} row(s) with a blank key never match another sheet", s.alias));
        }
        merged = Some(match merged {
            None => part,
            Some(m) => merge(m, part, &query.join_how),
        });
    }
    let merged = merged.expect("validated: at least one source");

    let column_for = |c: &Condition| -> Vec<&Column> {
        let mut out = Vec::new();
        for s in &query.sources {
            if c.source != ANY_SOURCE && c.source != s.alias {
                continue;
            }
            if let Some(col) = resolve_column(&frames[&s.alias].names(), &c.column) {
                if let Some(m) = merged.column(&format!("{}.{col}", s.alias)) {
                    out.push(m);
                }
            }
        }
        out
    };
    let mask = evaluate_group(&query.r#where, &column_for)?;
    let n = merged.keys.len();
    let rows: Vec<usize> = match mask {
        None => (0..n).collect(),
        Some(m) => (0..n).filter(|&i| m[i]).collect(),
    };

    let key_cells: Vec<Cell> = rows
        .iter()
        .map(|&r| {
            query
                .sources
                .iter()
                .filter_map(|s| merged.column(&format!("{}.{}", s.alias, s.key)))
                .map(|c| &c.cells[r])
                .find(|c| !c.is_empty())
                .cloned()
                .unwrap_or(Cell::Empty)
        })
        .collect();
    let mut columns = vec![Column::new(KEY_COL, DType::Object, key_cells)];
    for s in &query.sources {
        let mut names = Vec::new();
        if query.include_provenance {
            names.push(format!("{} {ROW_COL}", s.alias));
        }
        names.extend(query.outputs_for(&s.alias).iter().map(|c| format!("{}.{c}", s.alias)));
        for name in names {
            let col = merged.column(&name).expect("validated output column");
            columns.push(col.take(&rows));
        }
    }
    let mut matched = BTreeMap::new();
    for (alias, present) in &merged.present {
        matched.insert(alias.clone(), rows.iter().filter(|&&r| present[r]).count());
    }
    let frame = Frame { columns, index: (0..rows.len() as i64).collect() };
    let found = cell_hits(&query.r#where, &column_for)?;
    let (hits, colors) = cell_frames(&frame, &rows, &found);
    Ok((vec![ResultTable { name: JOINED.into(), frame, hits, colors }], matched))
}
