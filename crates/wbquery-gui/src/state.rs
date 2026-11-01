//! Everything the window does, without any UI: the sheets loaded, the condition tree, output columns,
//! running, exporting, saving and loading queries, and the messages for the user. The egui view in
//! [`crate::app`] draws this state and calls these methods; tests drive them directly.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use wbquery_core::cell::{Cell, Frame};
use wbquery_core::conditions::{
    ANY_SOURCE, Condition, Group, Node, column_key, parse_number, resolve_column, sort_columns,
};
use wbquery_core::export_html::write_html;
use wbquery_core::export_xlsx::write_xlsx;
use wbquery_core::query::{Query, QueryResult, Source, default_alias, is_provenance, unique_alias};
use wbquery_core::{VERSION, loader};

use crate::chart_state::ChartState;
use crate::tree;

pub const APP: &str = "Workbook Query";
/// The most value suggestions a column offers.
pub const MAX_SUGGESTIONS: usize = 1000;

/// A loaded sheet: where it came from, and its table.
#[derive(Debug, Clone)]
pub struct Entry {
    pub source: Source,
    pub frame: Frame,
}

impl Entry {
    pub fn columns(&self) -> Vec<String> {
        self.frame.names()
    }
}

/// One sheet's output columns: every column, in output order, ticked or not.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputList {
    pub alias: String,
    pub columns: Vec<(String, bool)>,
}

/// A column ticked in every sheet that has it, in some, or in none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    All,
    Some,
    None,
}

/// The line under the buttons: what just happened, optionally with a folder to open.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    pub text: String,
    /// "… saved to <path>": clicking opens the folder.
    pub saved: Option<PathBuf>,
}

/// The result of asking to add a workbook.
#[derive(Debug, Clone, PartialEq)]
pub enum Added {
    /// How many sheets were added.
    Sheets(usize),
    /// The workbook has several sheets: ask which (then call `add_workbook` with them).
    Choose(PathBuf, Vec<String>),
}

/// How sorting a result table works: by which column, which way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SortBy {
    pub column: Option<usize>,
    pub descending: bool,
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub entries: Vec<Entry>,
    pub conditions: Group,
    /// The selected condition or group (path from the root group); None: nothing selected.
    pub selected: Option<Vec<usize>>,
    /// Ask the view to focus the value box of the condition at this path.
    pub focus_value: Option<Vec<usize>>,
    pub outputs: Vec<OutputList>,
    pub join: bool,
    pub join_how: String,
    pub provenance: bool,
    pub name: String,
    pub description: String,
    pub result: Option<QueryResult>,
    /// Per result table (by position): how it's sorted.
    pub sort: Vec<SortBy>,
    pub status: Status,
    /// Errors to show, oldest first; the view shows and removes them.
    pub errors: Vec<String>,
    pub last_dir: Option<PathBuf>,
    pub chart: ChartState,
    values_cache: HashMap<(String, String), Vec<String>>,
}

impl Default for AppState {
    fn default() -> AppState {
        AppState {
            entries: Vec::new(),
            conditions: Group::default(),
            selected: None,
            focus_value: None,
            outputs: Vec::new(),
            join: false,
            join_how: "inner".into(),
            provenance: true,
            name: String::new(),
            description: String::new(),
            result: None,
            sort: Vec::new(),
            status: Status { text: "Add workbooks to start.".into(), saved: None },
            errors: Vec::new(),
            last_dir: None,
            chart: ChartState::default(),
            values_cache: HashMap::new(),
        }
    }
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// A cell as the result tables show it.
pub fn display(c: &Cell) -> String {
    c.display()
}

/// A cell as a condition value: as displayed, but 5.0 reads as 5.
pub fn value_text(c: &Cell) -> String {
    match c {
        Cell::Float(f) if f.is_finite() && f.fract() == 0.0 => {
            if *f == 0.0 {
                "0".into()
            } else {
                format!("{f:.0}")
            }
        }
        other => other.display().trim().to_string(),
    }
}

/// How value suggestions sort: numbers first (in order), then text ignoring case.
pub fn suggestion_key(text: &str) -> (bool, f64, String, String) {
    let number = parse_number(text).ok().flatten();
    (number.is_none(), number.unwrap_or(0.0), wbquery_core::pyfmt::casefold(text), text.to_string())
}

fn sort_suggestions(mut v: Vec<String>) -> Vec<String> {
    v.sort_by(|a, b| {
        let (ka, kb) = (suggestion_key(a), suggestion_key(b));
        ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1)).then(ka.2.cmp(&kb.2)).then(ka.3.cmp(&kb.3))
    });
    v.truncate(MAX_SUGGESTIONS);
    v
}

/// A file name from the query's name: letters, digits, `-` and `_` (spaces become `_`).
pub fn file_stem(name: &str, fallback: &str) -> String {
    let kept: String = name.chars().filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' ')).collect();
    let stem = kept.trim().replace(' ', "_");
    if stem.is_empty() { fallback.to_string() } else { stem }
}

impl AppState {
    // --- messages -------------------------------------------------------------------

    pub fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(msg.into());
    }

    fn say(&mut self, text: impl Into<String>) {
        self.status = Status { text: text.into(), saved: None };
    }

    pub fn title(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() { format!("{APP} {VERSION}") } else { format!("{name} — {APP} {VERSION}") }
    }

    /// Where file dialogs start: the last folder used, else the first workbook's folder, else home.
    pub fn save_dir(&self) -> PathBuf {
        if let Some(d) = self.last_dir.as_ref().filter(|d| d.is_dir()) {
            return d.clone();
        }
        for e in &self.entries {
            if let Some(parent) =
                Path::new(&e.source.path).parent().filter(|p| p.is_dir() && !p.as_os_str().is_empty())
            {
                return parent.to_path_buf();
            }
        }
        std::env::home_dir().unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn remember_dir(&mut self, path: &Path) {
        let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(parent) = full.parent() {
            self.last_dir = Some(parent.to_path_buf());
        }
    }

    /// Say where a file went (the status line links to its folder).
    pub fn saved(&mut self, what: &str, path: &Path) {
        self.remember_dir(path);
        let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.status = Status { text: format!("{what} saved to {}", full.display()), saved: Some(full) };
    }

    // --- sheets ------------------------------------------------------------------------

    pub fn aliases(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.source.alias.clone()).collect()
    }

    pub fn entry(&self, alias: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.source.alias == alias)
    }

    /// A sheet's columns; for any sheet, every sheet's columns once each (ignoring case and spaces).
    pub fn columns_for(&self, alias: &str) -> Vec<String> {
        if alias == ANY_SOURCE {
            let mut out: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for e in &self.entries {
                for c in e.columns() {
                    if seen.insert(column_key(&c)) {
                        out.push(c);
                    }
                }
            }
            return out;
        }
        self.entry(alias).map(Entry::columns).unwrap_or_default()
    }

    /// The sheets that have `column` (ignoring case and surrounding spaces).
    pub fn sheets_with(&self, column: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| resolve_column(&e.columns(), column).is_some())
            .map(|e| e.source.alias.clone())
            .collect()
    }

    /// (column, number of sheets) for each column found in more than one sheet.
    pub fn shared_columns(&self) -> Vec<(String, usize)> {
        if self.entries.len() < 2 {
            return Vec::new();
        }
        sort_columns(&self.columns_for(ANY_SOURCE))
            .into_iter()
            .filter_map(|c| {
                let n = self.sheets_with(&c).len();
                (n > 1).then_some((c, n))
            })
            .collect()
    }

    /// Distinct non-blank values of `column` in sheet `alias` (or every sheet with it), sorted.
    pub fn values_for(&mut self, alias: &str, column: &str) -> Vec<String> {
        let key = (alias.to_string(), column.to_string());
        if let Some(v) = self.values_cache.get(&key) {
            return v.clone();
        }
        let mut seen = std::collections::BTreeSet::new();
        for e in self.entries.iter().filter(|e| alias == ANY_SOURCE || e.source.alias == alias) {
            let Some(col) = resolve_column(&e.columns(), column) else { continue };
            for c in &e.frame.column(&col).expect("resolved column").cells {
                if !c.is_empty() {
                    let text = value_text(c);
                    if !text.is_empty() {
                        seen.insert(text);
                    }
                }
            }
        }
        let values = sort_suggestions(seen.into_iter().collect());
        self.values_cache.insert(key, values.clone());
        values
    }

    /// Suggestions for several columns of any sheet at once.
    pub fn values_of(&mut self, columns: &[String]) -> Vec<String> {
        let mut all = std::collections::BTreeSet::new();
        for c in columns {
            all.extend(self.values_for(ANY_SOURCE, c));
        }
        sort_suggestions(all.into_iter().collect())
    }

    /// Add sheets of `path`. With `sheets` None and several sheets in the workbook, nothing is added
    /// yet: the caller asks which ones.
    pub fn add_workbook(&mut self, path: &Path, sheets: Option<Vec<String>>) -> Added {
        let names = match loader::list_sheets(path) {
            Ok(n) => n,
            Err(e) => {
                self.error(format!("Can't read {}:\n{e}", path.display()));
                return Added::Sheets(0);
            }
        };
        let sheets = match sheets {
            Some(s) => s,
            None if names.len() <= 1 => names,
            None => return Added::Choose(path.to_path_buf(), names),
        };
        let path_text = path.to_string_lossy().into_owned();
        let mut added = 0;
        for s in sheets {
            if self.add_source(&path_text, &s, None, "", false) {
                added += 1;
            }
        }
        self.sources_changed();
        Added::Sheets(added)
    }

    /// Load one sheet as a new source. Returns whether it was added.
    pub fn add_source(
        &mut self,
        path: &str,
        sheet: &str,
        alias: Option<&str>,
        key: &str,
        refresh: bool,
    ) -> bool {
        let frame = match loader::load_table(path, Some(sheet)) {
            Ok(f) => f,
            Err(e) => {
                self.error(format!("Can't read sheet '{sheet}' of {path}:\n{e}"));
                return false;
            }
        };
        let base = alias.map(str::to_string).unwrap_or_else(|| default_alias(path, sheet));
        let alias = unique_alias(&base, &self.aliases());
        let mut e = Entry { source: Source::new(&alias, path, sheet, key), frame };
        if e.source.key.is_empty() || !e.columns().contains(&e.source.key) {
            e.source.key = self.guess_key(&e);
        }
        self.entries.push(e);
        if refresh {
            self.sources_changed();
        }
        true
    }

    /// The first column whose name another sheet also has, else the first column.
    fn guess_key(&self, e: &Entry) -> String {
        let others: std::collections::HashSet<String> =
            self.entries.iter().flat_map(|o| o.columns()).map(|c| column_key(&c)).collect();
        let cols = e.columns();
        cols.iter().find(|c| others.contains(&column_key(c))).or(cols.first()).cloned().unwrap_or_default()
    }

    pub fn remove_source(&mut self, row: usize) -> bool {
        if row >= self.entries.len() {
            return false;
        }
        self.entries.remove(row);
        self.sources_changed();
        true
    }

    /// Rename a source; refuses (with an error) a blank or duplicate name. Returns whether it changed.
    pub fn rename_source(&mut self, row: usize, new: &str) -> bool {
        let Some(e) = self.entries.get(row) else { return false };
        let (old, new) = (e.source.alias.clone(), new.trim().to_string());
        if new == old {
            return true;
        }
        if new.is_empty() || self.aliases().contains(&new) {
            self.error(format!("'{new}' is blank or already used; each sheet needs its own name."));
            return false;
        }
        self.entries[row].source.alias = new.clone();
        for o in &mut self.outputs {
            if o.alias == old {
                o.alias = new.clone();
            }
        }
        self.values_cache.clear();
        tree::rename_source(&mut self.conditions, &old, &new);
        true
    }

    fn sources_changed(&mut self) {
        self.values_cache.clear();
        self.rebuild_outputs(None);
        let n = self.entries.len();
        if n > 0 {
            self.say(format!(
                "{} loaded. Build conditions, tick output columns, then Run query.",
                plural(n, "sheet")
            ));
        } else {
            self.say("Add workbooks to start.");
        }
    }

    // --- output columns ------------------------------------------------------------------

    /// The checked output columns per sheet, in order.
    pub fn outputs_chosen(&self) -> Vec<(String, Vec<String>)> {
        self.outputs
            .iter()
            .map(|o| {
                (o.alias.clone(), o.columns.iter().filter(|(_, on)| *on).map(|(c, _)| c.clone()).collect())
            })
            .collect()
    }

    /// One list per sheet. `keep` (alias -> checked columns, in order) restores choices; a sheet not in
    /// it gets every column ticked. None keeps the current choices.
    pub fn rebuild_outputs(&mut self, keep: Option<HashMap<String, Vec<String>>>) {
        let keep: HashMap<String, Vec<String>> =
            keep.unwrap_or_else(|| self.outputs_chosen().into_iter().collect());
        self.outputs = self
            .entries
            .iter()
            .map(|e| {
                let cols = e.columns();
                let chosen = keep.get(&e.source.alias);
                let mut ordered: Vec<String> = chosen
                    .map(|c| c.iter().filter(|x| cols.contains(x)).cloned().collect())
                    .unwrap_or_default();
                let rest: Vec<String> = cols.iter().filter(|c| !ordered.contains(c)).cloned().collect();
                ordered.extend(rest);
                let columns = ordered
                    .into_iter()
                    .map(|c| {
                        let on = chosen.is_none_or(|ch| ch.contains(&c));
                        (c, on)
                    })
                    .collect();
                OutputList { alias: e.source.alias.clone(), columns }
            })
            .collect();
    }

    pub fn set_all_outputs(&mut self, alias: &str, on: bool) {
        if let Some(o) = self.outputs.iter_mut().find(|o| o.alias == alias) {
            for c in &mut o.columns {
                c.1 = on;
            }
        }
    }

    pub fn set_outputs(&mut self, alias: &str, columns: &[&str]) {
        if let Some(o) = self.outputs.iter_mut().find(|o| o.alias == alias) {
            for c in &mut o.columns {
                c.1 = columns.contains(&c.0.as_str());
            }
        }
    }

    /// Move an output column within its sheet's list.
    pub fn move_output(&mut self, alias: &str, from: usize, to: usize) {
        if let Some(o) = self.outputs.iter_mut().find(|o| o.alias == alias)
            && from < o.columns.len()
        {
            let item = o.columns.remove(from);
            let to = to.min(o.columns.len());
            o.columns.insert(to, item);
        }
    }

    /// Every column of every sheet (once, ignoring case and spaces), the sheets that have it, and
    /// whether it's ticked in all of them, some or none; sorted for a pick list.
    pub fn column_states(&self) -> Vec<(String, Vec<String>, Tick)> {
        let mut rows: Vec<(String, Vec<String>, Tick)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for o in &self.outputs {
            for (col, _) in &o.columns {
                if !seen.insert(column_key(col)) {
                    continue;
                }
                let mut sheets = Vec::new();
                let mut ticked = 0;
                for other in &self.outputs {
                    let names: Vec<&str> = other.columns.iter().map(|(c, _)| c.as_str()).collect();
                    if let Some(name) = resolve_column(&names, col) {
                        sheets.push(other.alias.clone());
                        if other.columns.iter().any(|(c, on)| *c == name && *on) {
                            ticked += 1;
                        }
                    }
                }
                let tick = if ticked == sheets.len() {
                    Tick::All
                } else if ticked == 0 {
                    Tick::None
                } else {
                    Tick::Some
                };
                rows.push((col.clone(), sheets, tick));
            }
        }
        let order = sort_columns(&rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>());
        rows.sort_by_key(|r| order.iter().position(|c| *c == r.0));
        rows
    }

    /// Tick (or untick) each column in every sheet that has it; `Tick::Some` leaves it as it is.
    pub fn apply_column_states(&mut self, states: &[(String, Tick)]) {
        for (col, tick) in states {
            let on = match tick {
                Tick::All => true,
                Tick::None => false,
                Tick::Some => continue,
            };
            for o in &mut self.outputs {
                let names: Vec<String> = o.columns.iter().map(|(c, _)| c.clone()).collect();
                if let Some(name) = resolve_column(&names, col) {
                    for c in &mut o.columns {
                        if c.0 == name {
                            c.1 = on;
                        }
                    }
                }
            }
        }
    }

    // --- conditions -------------------------------------------------------------------------

    /// (path of the group to add into, path of the selection).
    fn target(&self) -> (Vec<usize>, Vec<usize>) {
        let sel = self.selected.clone().unwrap_or_default();
        let sel =
            if tree::node(&self.conditions, &sel).is_some() || sel.is_empty() { sel } else { Vec::new() };
        let gpath =
            if tree::is_group(&self.conditions, &sel) { sel.clone() } else { sel[..sel.len() - 1].to_vec() };
        (gpath, sel)
    }

    /// Same sheet and column as the selected condition; else any sheet (with a column several sheets
    /// share, if there is one) when there are several sheets; else the only sheet.
    pub fn default_condition(&self) -> Condition {
        let (_, sel) = self.target();
        if let Some(Node::Cond(c)) = tree::node(&self.conditions, &sel) {
            return Condition::new(&c.source, &c.column, "=", "").expect("valid condition");
        }
        let aliases = self.aliases();
        let src = if aliases.len() != 1 { ANY_SOURCE.to_string() } else { aliases[0].clone() };
        let shared = self.shared_columns();
        let cols = if src == ANY_SOURCE && !shared.is_empty() {
            vec![shared[0].0.clone()]
        } else {
            self.columns_for(&src)
        };
        let col = cols.first().cloned().unwrap_or_else(|| "?".into());
        Condition::new(&src, &col, "=", "").expect("valid condition")
    }

    /// Add a condition (default: [`AppState::default_condition`]) to the selected group and select it.
    pub fn add_condition(&mut self, cond: Option<Condition>) -> Condition {
        let cond = cond.unwrap_or_else(|| self.default_condition());
        let (gpath, _) = self.target();
        let g = tree::group_mut(&mut self.conditions, &gpath).expect("target group");
        g.items.push(Node::Cond(cond.clone()));
        let mut path = gpath;
        path.push(g.items.len() - 1);
        self.selected = Some(path.clone());
        self.focus_value = Some(path);
        cond
    }

    /// One any-sheet condition per column, all with `op` and `value` (and highlight `color`): in the
    /// selected group, or in a new AND / OR group (`new_group`) added to it. Selects the first.
    pub fn add_shared_many(
        &mut self,
        columns: &[String],
        op: &str,
        value: &str,
        new_group: Option<&str>,
        color: &str,
    ) -> Vec<Condition> {
        let added: Vec<Condition> = columns
            .iter()
            .filter_map(|c| Condition::build(ANY_SOURCE, c, op, value, true, color).ok())
            .collect();
        if added.is_empty() {
            return added;
        }
        let (gpath, _) = self.target();
        let g = tree::group_mut(&mut self.conditions, &gpath).expect("target group");
        let mut first = gpath;
        match new_group {
            Some(logic) => {
                let sub =
                    Group::new(logic, added.iter().cloned().map(Node::Cond).collect()).unwrap_or_default();
                g.items.push(Node::Group(sub));
                first.extend([g.items.len() - 1, 0]);
            }
            None => {
                g.items.extend(added.iter().cloned().map(Node::Cond));
                first.push(g.items.len() - added.len());
            }
        }
        self.selected = Some(first.clone());
        if value.trim().is_empty() {
            self.focus_value = Some(first);
        }
        added
    }

    /// Insert a copy of the selected condition or group right after it.
    pub fn duplicate(&mut self) -> bool {
        let (_, sel) = self.target();
        let Some((last, parent)) = sel.split_last() else { return false };
        let g = tree::group_mut(&mut self.conditions, parent).expect("parent group");
        let copy = g.items[*last].clone();
        g.items.insert(last + 1, copy);
        let mut path = parent.to_vec();
        path.push(last + 1);
        self.selected = Some(path);
        true
    }

    pub fn add_group(&mut self, logic: &str) {
        let (gpath, _) = self.target();
        let g = tree::group_mut(&mut self.conditions, &gpath).expect("target group");
        g.items.push(Node::Group(Group::new(logic, Vec::new()).unwrap_or_default()));
        let mut path = gpath;
        path.push(g.items.len() - 1);
        self.selected = Some(path);
    }

    pub fn remove(&mut self) -> bool {
        let (_, sel) = self.target();
        let Some((last, parent)) = sel.split_last() else { return false };
        let g = tree::group_mut(&mut self.conditions, parent).expect("parent group");
        g.items.remove(*last);
        let n = g.items.len();
        let mut path = parent.to_vec();
        if n > 0 {
            path.push((*last).min(n - 1));
        }
        self.selected = Some(path);
        true
    }

    pub fn move_selected(&mut self, step: isize) -> bool {
        let (_, sel) = self.target();
        let Some((last, parent)) = sel.split_last() else { return false };
        let g = tree::group_mut(&mut self.conditions, parent).expect("parent group");
        let j = *last as isize + step;
        if j < 0 || j as usize >= g.items.len() {
            return false;
        }
        g.items.swap(*last, j as usize);
        let mut path = parent.to_vec();
        path.push(j as usize);
        self.selected = Some(path);
        true
    }

    /// Move the selected condition or group out of its group, to just after that group.
    pub fn move_out(&mut self) -> bool {
        let (_, sel) = self.target();
        if sel.len() < 2 {
            return false; // already at the top level
        }
        let (last, parent) = sel.split_last().expect("non-empty");
        let (parent_index, grandparent) = parent.split_last().expect("nested");
        let item = tree::group_mut(&mut self.conditions, parent).expect("parent group").items.remove(*last);
        let g = tree::group_mut(&mut self.conditions, grandparent).expect("grandparent group");
        g.items.insert(parent_index + 1, item);
        let mut path = grandparent.to_vec();
        path.push(parent_index + 1);
        self.selected = Some(path);
        true
    }

    /// Move the selected condition or group into the group just above it (as its last item).
    pub fn move_in(&mut self) -> bool {
        let (_, sel) = self.target();
        let Some((last, parent)) = sel.split_last() else { return false };
        if *last == 0 {
            return false;
        }
        let g = tree::group_mut(&mut self.conditions, parent).expect("parent group");
        if !matches!(g.items[last - 1], Node::Group(_)) {
            return false;
        }
        let item = g.items.remove(*last);
        let Node::Group(target) = &mut g.items[last - 1] else { unreachable!() };
        target.items.push(item);
        target.collapsed = false;
        let mut path = parent.to_vec();
        path.extend([last - 1, target.items.len() - 1]);
        self.selected = Some(path);
        true
    }

    /// Whether [`AppState::move_out`] / [`AppState::move_in`] would do anything.
    pub fn can_move_out(&self) -> bool {
        self.target().1.len() >= 2
    }

    pub fn can_move_in(&self) -> bool {
        let (_, sel) = self.target();
        match sel.split_last() {
            Some((last, parent)) if *last > 0 => tree::group(&self.conditions, parent)
                .is_some_and(|g| matches!(g.items.get(last - 1), Some(Node::Group(_)))),
            _ => false,
        }
    }

    /// (source, column) a condition on result column `column` should test, or None when it can't be
    /// tested (provenance and key columns). Shared columns test every sheet that has them.
    pub fn condition_target(&self, column: &str) -> Option<(String, String)> {
        if is_provenance(column) {
            return None;
        }
        if self.result.as_ref().is_some_and(|r| r.query.mode == "join") {
            let mut aliases = self.aliases();
            aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));
            return aliases.into_iter().find_map(|a| {
                column.strip_prefix(&format!("{a}.")).map(|rest| (a.clone(), rest.to_string()))
            });
        }
        let sheets = self.sheets_with(column);
        match sheets.len() {
            0 => None,
            1 => Some((sheets[0].clone(), column.to_string())),
            _ => Some((ANY_SOURCE.to_string(), column.to_string())),
        }
    }

    /// Conditions offered for a result cell: = and <> its value, or is (not) blank.
    pub fn cell_conditions(&self, column: &str, value: &Cell) -> Vec<Condition> {
        let Some((src, col)) = self.condition_target(column) else { return Vec::new() };
        let text = value_text(value);
        let make = |op: &str, v: &str| Condition::new(&src, &col, op, v).expect("valid condition");
        if text.is_empty() {
            vec![make("is blank", ""), make("is not blank", "")]
        } else {
            vec![make("=", &text), make("<>", &text)]
        }
    }

    pub fn add_condition_from_result(&mut self, cond: Condition) {
        let text = cond.describe();
        self.add_condition(Some(cond));
        self.say(format!("Added condition: {text}. Run the query to apply it."));
    }

    // --- the query ------------------------------------------------------------------------

    pub fn build_query(&self) -> Query {
        Query {
            sources: self.entries.iter().map(|e| e.source.clone()).collect(),
            r#where: self.conditions.clone(),
            outputs: self.outputs_chosen(),
            mode: if self.join { "join" } else { "independent" }.into(),
            join_how: self.join_how.clone(),
            include_provenance: self.provenance,
            name: self.name.trim().to_string(),
            description: self.description.trim().to_string(),
        }
    }

    /// Replace everything with `query`'s sources, conditions and outputs. False when a sheet couldn't
    /// be read (the error says which).
    pub fn apply_query(&mut self, query: &Query) -> bool {
        self.entries.clear();
        self.values_cache.clear();
        let mut ok = true;
        for s in &query.sources {
            let added = self.add_source(&s.path, &s.sheet, Some(&s.alias), &s.key, false);
            ok &= added && self.entries.last().is_some_and(|e| e.source.alias == s.alias);
        }
        self.join = query.mode == "join";
        self.join_how = query.join_how.clone();
        self.provenance = query.include_provenance;
        let keep: HashMap<String, Vec<String>> = query.outputs.iter().cloned().collect();
        self.rebuild_outputs(Some(keep));
        // A sheet missing from "outputs" outputs nothing, as when the query was saved.
        let missing: Vec<String> =
            self.aliases().into_iter().filter(|a| !query.outputs.iter().any(|(o, _)| o == a)).collect();
        for a in missing {
            self.set_all_outputs(&a, false);
        }
        self.conditions = query.r#where.clone();
        self.selected = None;
        self.name = query.name.clone();
        self.description = query.description.clone();
        self.clear_results();
        let n = plural(self.entries.len(), "sheet");
        if query.name.is_empty() {
            self.say(format!("Query loaded with {n}."));
        } else {
            self.say(format!("Loaded {} with {n}.", query.name));
        }
        ok
    }

    /// The default file name for saving the query.
    pub fn query_file_name(&self) -> String {
        format!("{}.json", file_stem(&self.name, "query"))
    }

    pub fn save_query(&mut self, path: &Path) -> Option<PathBuf> {
        match wbquery_core::save_query(&self.build_query(), path) {
            Ok(p) => {
                self.saved("Query", &p);
                Some(p)
            }
            Err(e) => {
                self.error(format!("Couldn't save the query to {}:\n{e}", path.display()));
                None
            }
        }
    }

    pub fn load_query(&mut self, path: &Path) -> bool {
        self.remember_dir(path);
        match wbquery_core::load_query(path) {
            Ok(q) => self.apply_query(&q),
            Err(e) => {
                self.error(e.to_string());
                false
            }
        }
    }

    // --- running and exporting ------------------------------------------------------------------

    pub fn clear_results(&mut self) {
        self.result = None;
        self.sort.clear();
        self.chart.set_result(None);
    }

    pub fn run(&mut self) -> bool {
        if self.entries.is_empty() {
            self.error("Add workbooks first.");
            return false;
        }
        let frames: HashMap<String, Frame> =
            self.entries.iter().map(|e| (e.source.alias.clone(), e.frame.clone())).collect();
        match wbquery_core::run_query(&self.build_query(), Some(&frames)) {
            Ok(result) => {
                let counts: Vec<String> = result
                    .tables
                    .iter()
                    .map(|t| format!("{}: {}", t.name, plural(t.frame.len(), "row")))
                    .collect();
                let mut text = format!("Matched — {}", counts.join(", "));
                for w in &result.warnings {
                    text.push_str(&format!("\n⚠ {w}"));
                }
                self.sort = vec![SortBy::default(); result.tables.len()];
                self.chart.set_result(Some(&result));
                self.result = Some(result);
                self.say(text);
                true
            }
            Err(e) => {
                self.clear_results();
                self.error(e.to_string());
                false
            }
        }
    }

    /// The default file name for an export.
    pub fn export_file_name(&self, fmt: &str) -> String {
        format!("{}.{fmt}", file_stem(&self.name, "query_results"))
    }

    /// Export the results as `xlsx` or `html`.
    pub fn export(&mut self, fmt: &str, path: &Path) -> Option<PathBuf> {
        let result = self.result.as_ref()?;
        let written = if fmt == "xlsx" { write_xlsx(result, path) } else { write_html(result, path) };
        match written {
            Ok(p) => {
                self.saved(if fmt == "xlsx" { "Excel workbook" } else { "HTML report" }, &p);
                Some(p)
            }
            Err(e) => {
                let msg = e.to_string();
                let hint = if msg.contains("ermission") || msg.contains("denied") {
                    "\n\nIf the file is open in Excel or a browser, close it and try again."
                } else {
                    ""
                };
                self.error(format!("Couldn't export to {}:\n{msg}{hint}", path.display()));
                None
            }
        }
    }

    /// Rows of result table `t` in display order (sorted by its [`SortBy`]; blanks first).
    pub fn table_order(&self, t: usize) -> Vec<usize> {
        let Some(table) = self.result.as_ref().and_then(|r| r.tables.get(t)) else { return Vec::new() };
        let n = table.frame.len();
        let mut order: Vec<usize> = (0..n).collect();
        let by = self.sort.get(t).copied().unwrap_or_default();
        if let Some(j) = by.column.filter(|j| *j < table.frame.columns.len()) {
            let col = &table.frame.columns[j];
            order.sort_by(|&a, &b| compare_cells(&col.cells[a], &col.cells[b]));
            if by.descending {
                order.reverse();
            }
        }
        order
    }

    /// Click a column header: sort by it, then reverse, then back to the original order.
    pub fn toggle_sort(&mut self, t: usize, column: usize) {
        if let Some(s) = self.sort.get_mut(t) {
            *s = match (s.column, s.descending) {
                (Some(c), false) if c == column => SortBy { column: Some(column), descending: true },
                (Some(c), true) if c == column => SortBy::default(),
                _ => SortBy { column: Some(column), descending: false },
            };
        }
    }
}

/// How result cells sort: blanks first, then numbers, dates, then text (ignoring case).
pub fn compare_cells(a: &Cell, b: &Cell) -> std::cmp::Ordering {
    fn rank(c: &Cell) -> u8 {
        match c {
            Cell::Empty => 0,
            Cell::Bool(_) | Cell::Int(_) | Cell::Float(_) => 1,
            Cell::Date(_) => 2,
            Cell::Text(_) => 3,
        }
    }
    fn num(c: &Cell) -> f64 {
        match c {
            Cell::Bool(b) => f64::from(u8::from(*b)),
            Cell::Int(i) => *i as f64,
            Cell::Float(f) => *f,
            _ => 0.0,
        }
    }
    rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
        (Cell::Date(x), Cell::Date(y)) => x.cmp(y),
        (Cell::Text(x), Cell::Text(y)) => {
            wbquery_core::pyfmt::casefold(x).cmp(&wbquery_core::pyfmt::casefold(y)).then(x.cmp(y))
        }
        _ => num(a).total_cmp(&num(b)),
    })
}
