//! Conditions and AND / OR groups that select rows.
//!
//! A [`Condition`] tests one column of one source (a workbook sheet), e.g. `Results › Value > 5e7`.
//! Its source may be `*` ("any sheet"): it then tests every source that has the column. Column
//! names match exactly when they can, otherwise ignoring case and surrounding spaces. Conditions
//! are combined in nested [`Group`]s, each joined by AND or OR and optionally negated.
//!
//! A value that reads as a number (`5e7`, `1,000`, `50%`) compares numerically, one that reads as
//! a date compares as a date, and otherwise `=` / `<>` compare text ignoring case and surrounding
//! spaces. A cell that starts with a number followed by text reads as that number
//! (`1 Micrococcus luteus` > 0). Blank cells never match (except `is blank`); other cells that
//! can't be compared match only `<>` / `not in` (`TNTC` <> 0).

use std::sync::LazyLock;

use chrono::NaiveDateTime;
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::cell::Column;
use crate::dates::parse_datetime;
use crate::error::{Error, Result};
use crate::loader::{to_datetime, to_numeric};
use crate::pyfmt::{casefold, py_float, py_float_repr, py_repr_str};

pub const OPERATORS: &[&str] = &[
    "=",
    "<>",
    "<",
    "<=",
    ">",
    ">=",
    "contains",
    "not contains",
    "starts with",
    "ends with",
    "in",
    "not in",
    "is blank",
    "is not blank",
];
pub const NO_VALUE_OPERATORS: &[&str] = &["is blank", "is not blank"];
pub const LOGIC: &[&str] = &["AND", "OR"];
pub const ANY_SOURCE: &str = "*";
const COMPARE: &[&str] = &["=", "<>", "<", "<=", ">", ">="];

static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^#?([0-9a-fA-F]{6})$").unwrap());
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)").unwrap());

/// `#rrggbb` (lower case) for a hex color, `""` for none; an error for anything else.
pub fn normalize_color(color: &str) -> Result<String> {
    let text = color.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    match HEX.captures(text) {
        Some(m) => Ok(format!("#{}", m[1].to_lowercase())),
        None => Err(Error::msg(format!("Highlight color must look like #ff0000, not '{color}'"))),
    }
}

/// Black or white, whichever reads better on `background` (`#rrggbb`).
pub fn text_color(background: &str) -> &'static str {
    let channel = |i: usize| u8::from_str_radix(&background[i..i + 2], 16).unwrap_or(0) as f64;
    if 0.299 * channel(1) + 0.587 * channel(3) + 0.114 * channel(5) > 150.0 { "#000000" } else { "#ffffff" }
}

/// Parse `2e7`, `20,000,000`, `50%` (-> 0.5). Blank -> None; not a number -> an error.
pub fn parse_number(text: &str) -> Result<Option<f64>> {
    let s = text.trim().replace(',', "");
    if s.is_empty() {
        return Ok(None);
    }
    let parsed = match s.strip_suffix('%') {
        Some(n) => py_float(n).map(|f| f / 100.0),
        None => py_float(&s),
    };
    parsed.map(Some).ok_or_else(|| Error::msg(format!("Not a number: '{text}'")))
}

/// How column names compare when no exact match exists: ignoring case and surrounding spaces.
pub fn column_key(name: &str) -> String {
    casefold(name.trim())
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Num(u128),
    Text(String),
}

fn natural_key(name: &str) -> Vec<Part> {
    let key = column_key(name);
    let mut out = Vec::new();
    let mut last = 0;
    for m in DIGITS.find_iter(&key) {
        if m.start() > last {
            out.push(Part::Text(key[last..m.start()].to_string()));
        }
        // Python's isdigit-based split also counts other Unicode digits; ASCII digits cover real headers.
        match m.as_str().parse::<u128>() {
            Ok(n) => out.push(Part::Num(n)),
            Err(_) => out.push(Part::Text(m.as_str().to_string())),
        }
        last = m.end();
    }
    if last < key.len() {
        out.push(Part::Text(key[last..].to_string()));
    }
    out
}

/// Columns in alphabetical order for pick lists: ignoring case, numbers in order (P2 before P10).
pub fn sort_columns<S: AsRef<str>>(columns: &[S]) -> Vec<String> {
    let mut out: Vec<String> = columns.iter().map(|c| c.as_ref().to_string()).collect();
    out.sort_by_cached_key(|c| (natural_key(c), c.clone()));
    out
}

/// The column of `columns` that `name` refers to: an exact match, else the first whose
/// [`column_key`] matches; None when there's neither.
pub fn resolve_column<S: AsRef<str>>(columns: &[S], name: &str) -> Option<String> {
    if let Some(c) = columns.iter().find(|c| c.as_ref() == name) {
        return Some(c.as_ref().to_string());
    }
    let key = column_key(name);
    columns.iter().find(|c| column_key(c.as_ref()) == key).map(|c| c.as_ref().to_string())
}

/// Items of an `in` / `not in` value: comma-separated, blanks dropped.
pub fn split_list(value: &str) -> Vec<String> {
    value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn blank_mask(col: &Column) -> Vec<bool> {
    col.cells.iter().map(|c| c.is_blank()).collect()
}

/// Each cell's text for text comparisons: stripped and case-folded, blanks as "".
fn text_of(col: &Column, blank: &[bool]) -> Vec<String> {
    col.as_text()
        .iter()
        .zip(blank)
        .map(|(t, &b)| if b { String::new() } else { casefold(t.trim()) })
        .collect()
}

/// One test of one column.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    pub source: String,
    pub column: String,
    pub op: String,
    pub value: String,
    pub enabled: bool,
    /// Highlight for the cells it counted for, `#rrggbb` or `""`.
    pub color: String,
}

impl Condition {
    /// A checked condition: known operator, a column, a valid color.
    pub fn new(source: &str, column: &str, op: &str, value: &str) -> Result<Condition> {
        Condition::build(source, column, op, value, true, "")
    }

    pub fn build(
        source: &str,
        column: &str,
        op: &str,
        value: &str,
        enabled: bool,
        color: &str,
    ) -> Result<Condition> {
        let color = normalize_color(color)?;
        let source = if source.is_empty() { ANY_SOURCE } else { source }.trim().to_string();
        if !OPERATORS.contains(&op) {
            return Err(Error::msg(format!(
                "Condition '{column}': unknown operator '{op}' (use one of {})",
                OPERATORS.join(", ")
            )));
        }
        if column.is_empty() {
            return Err(Error::msg("Condition: no column chosen"));
        }
        Ok(Condition { source, column: column.into(), op: op.into(), value: value.into(), enabled, color })
    }

    pub fn with_color(mut self, color: &str) -> Result<Condition> {
        self.color = normalize_color(color)?;
        Ok(self)
    }

    pub fn disabled(mut self) -> Condition {
        self.enabled = false;
        self
    }

    pub fn describe(&self) -> String {
        let place = if self.source == ANY_SOURCE { "Any sheet" } else { &self.source };
        let left = format!("{place} › {}", self.column);
        if NO_VALUE_OPERATORS.contains(&self.op.as_str()) {
            return format!("{left} {}", self.op);
        }
        if self.op == "in" || self.op == "not in" {
            let items: Vec<String> = split_list(&self.value).iter().map(|v| py_repr_str(v)).collect();
            return format!("{left} {} ({})", self.op, items.join(", "));
        }
        format!("{left} {} '{}'", self.op, self.value)
    }

    /// An error when the value can't work with the operator.
    pub fn check_value(&self) -> Result<()> {
        if NO_VALUE_OPERATORS.contains(&self.op.as_str()) {
            return Ok(());
        }
        if self.op == "in" || self.op == "not in" {
            if split_list(&self.value).is_empty() {
                return Err(Error::msg(format!(
                    "{}: give one or more values separated by commas",
                    self.describe()
                )));
            }
            return Ok(());
        }
        if self.value.trim().is_empty() && self.op != "=" && self.op != "<>" {
            return Err(Error::msg(format!("{}: needs a value", self.describe())));
        }
        self.matches(&Column::new("", crate::DType::Object, Vec::new())).map(|_| ())
    }

    /// Whether each cell of `col` matches. Blank cells, and others that can't be compared, don't
    /// (except that `<>` matches non-blank cells that can't be read as its number or date).
    pub fn matches(&self, col: &Column) -> Result<Vec<bool>> {
        let blank = blank_mask(col);
        let op = self.op.as_str();
        match op {
            "is blank" => return Ok(blank),
            "is not blank" => return Ok(blank.iter().map(|b| !b).collect()),
            _ if COMPARE.contains(&op) => return self.compare(op, &self.value, col, &blank),
            "in" | "not in" => {
                let mut hit = vec![false; col.len()];
                for item in split_list(&self.value) {
                    for (h, m) in hit.iter_mut().zip(self.compare("=", &item, col, &blank)?) {
                        *h |= m;
                    }
                }
                if op == "not in" {
                    return Ok(hit.iter().zip(&blank).map(|(h, b)| !h && !b).collect());
                }
                return Ok(hit);
            }
            _ => {}
        }
        let text = text_of(col, &blank);
        let needle = casefold(self.value.trim());
        Ok(text
            .iter()
            .map(|t| match op {
                "contains" => t.contains(&needle),
                "not contains" => !t.contains(&needle),
                "starts with" => t.starts_with(&needle),
                _ => t.ends_with(&needle),
            })
            .collect())
    }

    fn compare(&self, op: &str, value: &str, col: &Column, blank: &[bool]) -> Result<Vec<bool>> {
        if let Ok(Some(number)) = parse_number(value) {
            let left: Vec<Option<f64>> =
                to_numeric(col).into_iter().map(|f| (!f.is_nan()).then_some(f)).collect();
            return Ok(ordered(op, &left, &number, blank));
        }
        let when = if value.trim().is_empty() { None } else { parse_datetime(value) };
        if let Some(when) = when {
            let left: Vec<Option<NaiveDateTime>> = to_datetime(col);
            return Ok(ordered(op, &left, &when, blank));
        }
        if op != "=" && op != "<>" {
            return Err(Error::msg(format!(
                "{}: '{op}' needs a number or date, not '{value}'",
                self.describe()
            )));
        }
        let needle = casefold(value.trim());
        Ok(text_of(col, blank).iter().map(|t| (t == &needle) == (op == "=")).collect())
    }

    pub fn to_json(&self) -> Value {
        let mut doc = Map::new();
        doc.insert("source".into(), json!(self.source));
        doc.insert("column".into(), json!(self.column));
        doc.insert("op".into(), json!(self.op));
        doc.insert("value".into(), json!(self.value));
        doc.insert("enabled".into(), json!(self.enabled));
        if !self.color.is_empty() {
            doc.insert("color".into(), json!(self.color));
        }
        Value::Object(doc)
    }
}

/// Compare cells read as numbers / dates. A non-blank cell that doesn't read as one can't be
/// ordered, so only `<>` matches it.
fn ordered<T: PartialOrd>(op: &str, left: &[Option<T>], value: &T, blank: &[bool]) -> Vec<bool> {
    left.iter()
        .zip(blank)
        .map(|(l, &b)| match l {
            Some(l) => match op {
                "=" => l == value,
                "<>" => l != value,
                "<" => l < value,
                "<=" => l <= value,
                ">" => l > value,
                _ => l >= value,
            },
            None => op == "<>" && !b,
        })
        .collect()
}

/// An item of a group.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Cond(Condition),
    Group(Group),
}

impl From<Condition> for Node {
    fn from(c: Condition) -> Node {
        Node::Cond(c)
    }
}

impl From<Group> for Node {
    fn from(g: Group) -> Node {
        Node::Group(g)
    }
}

/// AND / OR of conditions and sub-groups. A group that isn't `enabled` is left out entirely.
/// `name` is a label for people; `collapsed` is how the editor shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub logic: String,
    pub items: Vec<Node>,
    pub negate: bool,
    pub name: String,
    pub enabled: bool,
    pub collapsed: bool,
}

impl Default for Group {
    fn default() -> Group {
        Group {
            logic: "AND".into(),
            items: Vec::new(),
            negate: false,
            name: String::new(),
            enabled: true,
            collapsed: false,
        }
    }
}

impl Group {
    pub fn new(logic: &str, items: Vec<Node>) -> Result<Group> {
        let logic = logic.to_uppercase();
        if !LOGIC.contains(&logic.as_str()) {
            return Err(Error::msg(format!("Group: logic must be AND or OR, not '{logic}'")));
        }
        Ok(Group { logic, items, ..Group::default() })
    }

    pub fn and(items: Vec<Node>) -> Group {
        Group { items, ..Group::default() }
    }

    pub fn or(items: Vec<Node>) -> Group {
        Group { logic: "OR".into(), items, ..Group::default() }
    }

    pub fn negated(mut self) -> Group {
        self.negate = true;
        self
    }

    pub fn named(mut self, name: &str) -> Group {
        self.name = name.trim().to_string();
        self
    }

    pub fn disabled(mut self) -> Group {
        self.enabled = false;
        self
    }

    /// Every condition in this group and its sub-groups, in order; with `active`, only those that
    /// are switched on and not inside a group that's switched off.
    pub fn conditions(&self, active: bool) -> Vec<&Condition> {
        if active && !self.enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        for item in &self.items {
            match item {
                Node::Group(g) => out.extend(g.conditions(active)),
                Node::Cond(c) if c.enabled || !active => out.push(c),
                Node::Cond(_) => {}
            }
        }
        out
    }

    /// Every condition, mutably, depth-first.
    pub fn conditions_mut(&mut self) -> Vec<&mut Condition> {
        let mut out = Vec::new();
        for item in &mut self.items {
            match item {
                Node::Group(g) => out.extend(g.conditions_mut()),
                Node::Cond(c) => out.push(c),
            }
        }
        out
    }

    pub fn describe(&self) -> String {
        self.describe_at(true)
    }

    fn describe_at(&self, top: bool) -> String {
        let empty = if top { "(all rows)" } else { "" };
        if !self.enabled {
            return empty.into();
        }
        let parts: Vec<String> = self
            .items
            .iter()
            .filter_map(|i| match i {
                Node::Group(g) => Some(g.describe_at(false)),
                Node::Cond(c) if c.enabled => Some(c.describe()),
                Node::Cond(_) => None,
            })
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            return empty.into();
        }
        let mut body = parts.join(&format!(" {} ", self.logic));
        if parts.len() > 1 && !(top && !self.negate) {
            body = format!("({body})");
        }
        if self.negate {
            body = format!("NOT {body}");
        }
        if self.name.is_empty() { body } else { format!("[{}] {body}", self.name) }
    }

    pub fn to_json(&self) -> Value {
        let mut doc = Map::new();
        doc.insert("logic".into(), json!(self.logic));
        doc.insert("negate".into(), json!(self.negate));
        if !self.name.is_empty() {
            doc.insert("name".into(), json!(self.name));
        }
        if !self.enabled {
            doc.insert("enabled".into(), json!(false));
        }
        if self.collapsed {
            doc.insert("collapsed".into(), json!(true));
        }
        let items: Vec<Value> = self
            .items
            .iter()
            .map(|i| match i {
                Node::Group(g) => g.to_json(),
                Node::Cond(c) => c.to_json(),
            })
            .collect();
        doc.insert("items".into(), Value::Array(items));
        Value::Object(doc)
    }

    /// A group from its saved JSON (as Python's `Group.from_dict` reads it).
    pub fn from_json(doc: &Value) -> Result<Group> {
        let Value::Object(doc) = doc else { return Err(Error::msg("A condition group must be an object")) };
        let mut items = Vec::new();
        if let Some(list) = doc.get("items") {
            for item in json_items(list) {
                let Value::Object(map) = item else {
                    return Err(Error::msg("Each condition must be an object"));
                };
                if map.contains_key("items") {
                    items.push(Node::Group(Group::from_json(item)?));
                    continue;
                }
                let (Some(column), Some(op)) = (map.get("column"), map.get("op")) else {
                    return Err(Error::msg("Each condition needs a \"column\" and an \"op\""));
                };
                let source =
                    map.get("source").filter(|v| truthy(v)).map(py_str).unwrap_or_else(|| ANY_SOURCE.into());
                let value = match map.get("value") {
                    None | Some(Value::Null) => String::new(),
                    Some(v) => py_str(v),
                };
                let color = match map.get("color") {
                    None | Some(Value::Null) => String::new(),
                    Some(v) => py_str(v),
                };
                let enabled = map.get("enabled").is_none_or(truthy);
                let column = if truthy(column) { py_str(column) } else { String::new() };
                items.push(Node::Cond(Condition::build(
                    &source,
                    &column,
                    &py_str(op),
                    &value,
                    enabled,
                    &color,
                )?));
            }
        }
        let logic = doc.get("logic").map(py_str).unwrap_or_else(|| "AND".into());
        let mut group = Group::new(&logic, items)?;
        group.negate = doc.get("negate").is_some_and(truthy);
        group.name = match doc.get("name") {
            None | Some(Value::Null) => String::new(),
            Some(v) => py_str(v).trim().to_string(),
        };
        group.enabled = doc.get("enabled").is_none_or(truthy);
        group.collapsed = doc.get("collapsed").is_some_and(truthy);
        Ok(group)
    }
}

/// The items of a JSON list (Python iterates whatever it is given; anything else has none).
fn json_items(v: &Value) -> Vec<&Value> {
    match v {
        Value::Array(a) => a.iter().collect(),
        _ => Vec::new(),
    }
}

/// Python truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// Python `str()` of a JSON value.
pub fn py_str(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(n) if n.is_f64() => py_float_repr(n.as_f64().unwrap_or(f64::NAN)),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Finds the columns a condition tests in the current frame: several for an any-sheet condition
/// after a join (a row matches if any of them does), none when it doesn't apply here.
pub type ColumnResolver<'a> = dyn Fn(&Condition) -> Vec<&'a Column> + 'a;

/// Rows that `node` selects, or None when nothing in it applies. Conditions that don't apply,
/// and disabled ones, are left out, so a group reduces to the parts that do apply.
pub fn evaluate<'a>(node: &Node, column_for: &ColumnResolver<'a>) -> Result<Option<Vec<bool>>> {
    match node {
        Node::Cond(c) => {
            if !c.enabled {
                return Ok(None);
            }
            let cols = column_for(c);
            let Some(first) = cols.first() else { return Ok(None) };
            let mut hit = c.matches(first)?;
            for col in &cols[1..] {
                for (h, m) in hit.iter_mut().zip(c.matches(col)?) {
                    *h |= m;
                }
            }
            Ok(Some(hit))
        }
        Node::Group(g) => evaluate_group(g, column_for),
    }
}

pub fn evaluate_group<'a>(group: &Group, column_for: &ColumnResolver<'a>) -> Result<Option<Vec<bool>>> {
    if !group.enabled {
        return Ok(None);
    }
    let mut out: Option<Vec<bool>> = None;
    for item in &group.items {
        let Some(r) = evaluate(item, column_for)? else { continue };
        out = Some(match out {
            None => r,
            Some(acc) if group.logic == "AND" => acc.iter().zip(&r).map(|(a, b)| *a && *b).collect(),
            Some(acc) => acc.iter().zip(&r).map(|(a, b)| *a || *b).collect(),
        });
    }
    Ok(out.map(|o| if group.negate { o.iter().map(|b| !b).collect() } else { o }))
}

/// One condition's matching cells in one column.
#[derive(Debug, Clone)]
pub struct CellHit<'q> {
    pub condition: &'q Condition,
    pub column: String,
    pub rows: Vec<bool>,
}

/// Which cells each condition counted for: a cell counts when its condition is true there and every
/// group above it is true for that row. Conditions inside a negated group never count.
pub fn cell_hits<'q, 'a>(root: &'q Group, column_for: &ColumnResolver<'a>) -> Result<Vec<CellHit<'q>>> {
    let mut out = Vec::new();
    walk(root, None, column_for, &mut out)?;
    Ok(out)
}

fn walk<'q, 'a>(
    group: &'q Group,
    ok: Option<&[bool]>,
    column_for: &ColumnResolver<'a>,
    out: &mut Vec<CellHit<'q>>,
) -> Result<()> {
    if !group.enabled {
        return Ok(());
    }
    let Some(here) = evaluate_group(group, column_for)? else { return Ok(()) };
    if group.negate {
        return Ok(());
    }
    let ok: Vec<bool> = match ok {
        None => here,
        Some(o) => o.iter().zip(&here).map(|(a, b)| *a && *b).collect(),
    };
    for item in &group.items {
        match item {
            Node::Group(g) => walk(g, Some(&ok), column_for, out)?,
            Node::Cond(c) if c.enabled => {
                for col in column_for(c) {
                    let rows = c.matches(col)?.iter().zip(&ok).map(|(m, o)| *m && *o).collect();
                    out.push(CellHit { condition: c, column: col.name.clone(), rows });
                }
            }
            Node::Cond(_) => {}
        }
    }
    Ok(())
}
