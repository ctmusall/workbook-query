"""A query over several workbook sheets, and running it.

Two modes:

* ``independent`` - each source is filtered on its own. A condition naming another
  source is left out for this one (so ``A.x > 5 OR B.y < 3`` filters A by ``x > 5`` and
  B by ``y < 3``), as is an any-sheet condition on a column this source lacks. When the
  query has no active conditions every source keeps all its rows; otherwise a source
  that none of them apply to matches no rows (a warning names it). Condition column
  names match exactly when they can, otherwise ignoring case and surrounding spaces.
  Each source with output columns
  gives one result table; when there are several, an "All results" table stacking
  them (columns matched by name) comes first.
* ``join`` - rows are linked across sources by each source's key column, and the whole
  condition tree is tested against the joined row. One result table.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass, field
from datetime import date, datetime
from pathlib import Path

import pandas as pd

from . import loader
from .conditions import ANY_SOURCE, Condition, Group, cell_hits, evaluate, resolve_column
from .proof import file_sha256

MODES = ("independent", "join")
JOIN_HOW = ("inner", "left", "outer")
FILE_COL = "File"
SHEET_COL = "Sheet"
ROW_COL = "Excel row"
KEY_COL = "Key"
JOINED = "Joined"
SOURCE_COL = "Source"
COMBINED = "All results"
_KEY = "__key__"
_PRESENT = "__present__"
FORMAT_VERSION = 1


def is_provenance(col: str) -> bool:
    """The file / sheet / row columns a query adds to its result tables."""
    return col in (SOURCE_COL, FILE_COL, SHEET_COL, ROW_COL) or col.endswith(f" {ROW_COL}")


@dataclass
class Source:
    alias: str
    path: str
    sheet: str
    key: str = ""

    def to_dict(self) -> dict:
        return {"alias": self.alias, "path": self.path, "sheet": self.sheet, "key": self.key}


@dataclass
class Query:
    sources: list[Source] = field(default_factory=list)
    where: Group = field(default_factory=Group)
    outputs: dict[str, list[str]] = field(default_factory=dict)
    mode: str = "independent"
    join_how: str = "inner"
    include_provenance: bool = True
    name: str = ""  # what the query is called, e.g. "EOJ personnel hits, full year"
    description: str = ""  # what it does and why, in your own words

    def source(self, alias: str) -> Source:
        for s in self.sources:
            if s.alias == alias:
                return s
        raise KeyError(f"No source named '{alias}'")

    def to_dict(self) -> dict:
        return {
            "version": FORMAT_VERSION,
            "name": self.name,
            "description": self.description,
            "mode": self.mode,
            "join_how": self.join_how,
            "include_provenance": self.include_provenance,
            "sources": [s.to_dict() for s in self.sources],
            "where": self.where.to_dict(),
            "outputs": {k: list(v) for k, v in self.outputs.items()},
        }

    @classmethod
    def from_dict(cls, doc: dict) -> "Query":
        if not isinstance(doc, dict):
            raise ValueError("A query must be a JSON object")
        sources = []
        for s in doc.get("sources", []):
            if not isinstance(s, dict) or not all(k in s for k in ("alias", "path", "sheet")):
                raise ValueError("Each source needs \"alias\", \"path\" and \"sheet\"")
            sources.append(Source(str(s["alias"]), str(s["path"]), str(s["sheet"]), str(s.get("key", "") or "")))
        outputs = doc.get("outputs", {})
        if not isinstance(outputs, dict):
            raise ValueError("\"outputs\" must map each source alias to a list of columns")
        return cls(
            sources=sources,
            where=Group.from_dict(doc.get("where", {})),
            outputs={str(k): [str(c) for c in v] for k, v in outputs.items()},
            mode=doc.get("mode", "independent"),
            join_how=doc.get("join_how", "inner"),
            include_provenance=bool(doc.get("include_provenance", True)),
            name=str(doc.get("name") or ""),
            description=str(doc.get("description") or ""),
        )


@dataclass
class QueryResult:
    tables: dict[str, pd.DataFrame]
    warnings: list[str]
    sources: list[dict]  # alias, path, sheet, rows, matched, sha256
    query: Query
    run_at: datetime
    # Per table, same shape and columns: the cells a condition counted for, and the highlight color
    # of the first colored condition that counted there ("" for none). See conditions.cell_hits.
    hits: dict[str, pd.DataFrame] = field(default_factory=dict)
    colors: dict[str, pd.DataFrame] = field(default_factory=dict)


def default_alias(path: str | Path, sheet: str) -> str:
    stem = Path(path).stem
    return stem if loader.is_csv(path) or sheet == stem else f"{stem}/{sheet}"


def unique_alias(base: str, taken) -> str:
    taken = set(taken)
    if base not in taken:
        return base
    n = 2
    while f"{base} ({n})" in taken:
        n += 1
    return f"{base} ({n})"


def save_query(query: Query, path: str | Path) -> Path:
    path = Path(path)
    path.write_text(json.dumps(query.to_dict(), indent=2) + "\n", encoding="utf-8")
    return path


def load_query(path: str | Path) -> Query:
    try:
        doc = json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValueError(f"Query file not found: {path}") from None
    except json.JSONDecodeError as exc:
        raise ValueError(f"Query file {path} is not valid JSON: {exc}") from None
    try:
        return Query.from_dict(doc)
    except ValueError as exc:
        raise ValueError(f"Query file {path}: {exc}") from None


def load_frames(query: Query) -> dict[str, pd.DataFrame]:
    """Each source's table, read once per (file, sheet)."""
    cache: dict[tuple[str, str], pd.DataFrame] = {}
    out = {}
    for s in query.sources:
        k = (str(Path(s.path).resolve()), s.sheet)
        if k not in cache:
            try:
                cache[k] = loader.load_table(s.path, s.sheet)
            except Exception as exc:  # noqa: BLE001 - name the source in the message
                raise ValueError(f"{s.alias}: {exc}") from None
        out[s.alias] = cache[k]
    return out


def validate(query: Query, columns: dict[str, list[str]]) -> list[str]:
    """Problems that stop ``query`` running, given each source's column names."""
    errors = []
    if query.mode not in MODES:
        errors.append(f"Mode must be one of {', '.join(MODES)}, not '{query.mode}'")
    if query.mode == "join" and query.join_how not in JOIN_HOW:
        errors.append(f"Join type must be one of {', '.join(JOIN_HOW)}, not '{query.join_how}'")
    if not query.sources:
        errors.append("Add at least one workbook sheet")
    aliases = [s.alias for s in query.sources]
    for a in sorted({a for a in aliases if aliases.count(a) > 1}):
        errors.append(f"Source name '{a}' is used more than once")
    for s in query.sources:
        if not s.alias.strip():
            errors.append(f"{s.path} [{s.sheet}]: give the source a name")
        if query.mode == "join":
            if not s.key:
                errors.append(f"{s.alias}: choose a key column to join on")
            elif s.key not in columns.get(s.alias, []):
                errors.append(f"{s.alias}: key column '{s.key}' not found")
    for c in query.where.conditions(active=True):
        if c.source == ANY_SOURCE:
            if all(resolve_column(cols, c.column) is None for cols in columns.values()):
                errors.append(f"{c.describe()}: no sheet has a column '{c.column}'")
        elif c.source not in columns:
            errors.append(f"{c.describe()}: no source named '{c.source}'")
        elif resolve_column(columns[c.source], c.column) is None:
            errors.append(f"{c.describe()}: column '{c.column}' not found in {c.source}")
        try:
            c.check_value()
        except ValueError as exc:
            errors.append(str(exc))
    for alias, cols in query.outputs.items():
        if not cols:
            continue
        if alias not in columns:
            errors.append(f"Output columns given for unknown source '{alias}'")
            continue
        missing = [c for c in cols if c not in columns[alias]]
        if missing:
            errors.append(f"{alias}: output column(s) not found: {', '.join(missing)}")
    if not any(query.outputs.get(a) for a in aliases):
        errors.append("Choose at least one output column")
    return errors


def run_query(query: Query, frames: dict[str, pd.DataFrame] | None = None) -> QueryResult:
    """Run ``query``; ``frames`` (alias -> table) skips re-reading files the caller already loaded."""
    if frames is None:
        frames = load_frames(query)
    columns = {s.alias: list(frames[s.alias].columns) for s in query.sources if s.alias in frames}
    errors = validate(query, columns)
    if errors:
        raise ValueError("\n".join(errors))
    warnings: list[str] = []
    if query.mode == "join":
        tables, matched, cells = _run_join(query, frames, warnings)
    else:
        tables, matched, cells = _run_independent(query, frames, warnings)
    info = [{"alias": s.alias, "path": str(s.path), "sheet": s.sheet, "rows": len(frames[s.alias]),
             "matched": matched.get(s.alias), "sha256": file_sha256(s.path)} for s in query.sources]
    hits = {name: cells[name][0] for name in tables}
    colors = {name: cells[name][1] for name in tables}
    return QueryResult(tables, warnings, info, query, datetime.now(), hits, colors)


def _cell_frames(table: pd.DataFrame, rows: pd.Index, found, rename=lambda c: c):
    """(hits, colors) frames shaped like ``table`` for the matched ``rows`` of the frame that
    ``found`` (from :func:`cell_hits`) was computed on; ``rename`` maps a tested column to its name
    in ``table``."""
    hits = pd.DataFrame(False, index=range(len(table)), columns=table.columns)
    colors = pd.DataFrame("", index=range(len(table)), columns=table.columns)
    for cond, col, mask in found:
        name = rename(col)
        if name not in table.columns:
            continue
        hit = mask.reindex(rows).fillna(False).to_numpy(dtype=bool)
        if not hit.any():
            continue
        if cond.color:
            colors.loc[hit & (colors[name] == "").to_numpy(), name] = cond.color  # first color wins
        hits[name] = hits[name].to_numpy(dtype=bool) | hit
    return hits, colors


def _run_independent(query, frames, warnings):
    tables, matched, cells = {}, {}, {}
    filtering = bool(query.where.conditions(active=True))
    untested = []
    for s in query.sources:
        df = frames[s.alias]

        def column_for(c: Condition, s=s, df=df):
            col = resolve_column(df.columns, c.column) if c.source in (ANY_SOURCE, s.alias) else None
            return [] if col is None else [df[col]]

        mask = evaluate(query.where, column_for)
        if mask is None and filtering:  # nothing in the query can be tested here
            untested.append(s.alias)
            rows = df.iloc[0:0]
        else:
            rows = df if mask is None else df[mask.to_numpy(dtype=bool)]
        matched[s.alias] = len(rows)
        cols = query.outputs.get(s.alias) or []
        if not cols:
            continue
        out = rows[cols].copy()
        if query.include_provenance:
            out.insert(0, ROW_COL, rows.index.to_numpy())
            out.insert(0, SHEET_COL, s.sheet)
            out.insert(0, FILE_COL, Path(s.path).name)
        tables[s.alias] = out.reset_index(drop=True)
        cells[s.alias] = _cell_frames(tables[s.alias], rows.index, cell_hits(query.where, column_for))
    if untested:
        names = ", ".join(untested[:5]) + (f" and {len(untested) - 5} more" if len(untested) > 5 else "")
        n = len(untested)
        warnings.append(f"{n} sheet{'s' if n != 1 else ''} lack every column the conditions test, so "
                        f"{'they match' if n != 1 else 'it matches'} no rows: {names}")
    if len(tables) >= 2:
        combined = _combine(tables, query.include_provenance)
        cells[COMBINED] = _combine_cells(combined, [cells[a] for a in tables])
        return {COMBINED: combined, **tables}, matched, cells
    return tables, matched, cells


def _combine(tables: dict[str, pd.DataFrame], provenance: bool) -> pd.DataFrame:
    """Stack the per-source tables; a column a source lacks is blank for its rows."""
    parts = []
    for alias, t in tables.items():
        if provenance:
            t = t.copy()
            t.insert(0, SOURCE_COL, alias)
        parts.append(t)
    return pd.concat(parts, ignore_index=True, sort=False)


def _combine_cells(combined: pd.DataFrame, parts: list[tuple[pd.DataFrame, pd.DataFrame]]):
    """Stack per-source (hits, colors) the way :func:`_combine` stacks the tables."""
    hits = pd.concat([h for h, _ in parts], ignore_index=True, sort=False)
    colors = pd.concat([c for _, c in parts], ignore_index=True, sort=False)
    hits = hits.reindex(columns=combined.columns).astype(object).fillna(False).astype(bool)
    colors = colors.reindex(columns=combined.columns).astype(object).fillna("")
    return hits, colors


def normalize_key(value, blank_token: str):
    """Join key for one cell: numbers compare by value (1 == 1.0 == '1'), dates by day,
    text ignoring case and surrounding spaces. Blank cells get ``blank_token`` so they never match."""
    if value is None or (isinstance(value, float) and math.isnan(value)) or value is pd.NaT:
        return blank_token
    if isinstance(value, (pd.Timestamp, datetime, date)):
        return pd.Timestamp(value).date().isoformat()
    if isinstance(value, bool):
        return str(value).casefold()
    if isinstance(value, (int, float)):
        f = float(value)
        return str(int(f)) if f.is_integer() else repr(f)
    text = str(value).strip()
    if not text:
        return blank_token
    try:
        f = float(text.replace(",", ""))
        if math.isfinite(f):
            return str(int(f)) if f.is_integer() else repr(f)
    except ValueError:
        pass
    return text.casefold()


def _keyed(s: Source, df: pd.DataFrame, provenance: bool) -> pd.DataFrame:
    out = df.add_prefix(f"{s.alias}.")
    if provenance:
        out.insert(0, f"{s.alias} {ROW_COL}", df.index.to_numpy())
    out[f"{_PRESENT}{s.alias}"] = True
    out[_KEY] = [normalize_key(v, f"\0blank:{s.alias}:{i}") for i, v in zip(df.index, df[s.key])]
    return out.reset_index(drop=True)


def _run_join(query, frames, warnings):
    merged = None
    for s in query.sources:
        df = frames[s.alias]
        part = _keyed(s, df, query.include_provenance)
        real = part[_KEY][~part[_KEY].str.startswith("\0blank:")]
        dupes = real[real.duplicated(keep=False)].nunique()
        if dupes:
            warnings.append(f"{s.alias}: {dupes} key value(s) in '{s.key}' appear on more than one row; "
                            "each such row is paired with every match in the other sheets")
        blanks = len(part) - len(real)
        if blanks:
            warnings.append(f"{s.alias}: {blanks} row(s) with a blank key never match another sheet")
        merged = part if merged is None else merged.merge(part, on=_KEY, how=query.join_how)

    aliases = [s.alias for s in query.sources]

    def column_for(c: Condition):
        names = []
        for a in aliases:
            col = resolve_column(frames[a].columns, c.column) if c.source in (ANY_SOURCE, a) else None
            if col is not None:
                names.append(f"{a}.{col}")
        return [merged[n] for n in names if n in merged.columns]

    mask = evaluate(query.where, column_for)
    rows = merged if mask is None else merged[mask.to_numpy(dtype=bool)]

    key = pd.Series(pd.NA, index=rows.index, dtype=object)
    for s in query.sources:
        key = key.where(key.notna(), rows[f"{s.alias}.{s.key}"])
    cols = []
    for s in query.sources:
        if query.include_provenance:
            cols.append(f"{s.alias} {ROW_COL}")
        cols += [f"{s.alias}.{c}" for c in query.outputs.get(s.alias) or []]
    out = rows[cols].copy()
    out.insert(0, KEY_COL, key)
    for c in out.columns:
        if c.endswith(f" {ROW_COL}"):
            out[c] = out[c].astype("Int64")
    matched = {s.alias: int(rows[f"{_PRESENT}{s.alias}"].notna().sum()) for s in query.sources}
    out = out.reset_index(drop=True)
    return {JOINED: out}, matched, {JOINED: _cell_frames(out, rows.index, cell_hits(query.where, column_for))}
