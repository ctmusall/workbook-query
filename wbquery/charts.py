"""Charts of a result table, drawn with matplotlib onto a Figure (no Qt needed).

``draw(fig, df, spec)`` clears ``fig`` and draws the chart ``spec`` describes. Values are
read the same way the query reads them: numbers as numbers, text such as ``1,000``,
``5e7``, ``50%`` or ``1 Micrococcus luteus`` (-> 1) parsed (see :func:`loader.cell_number`),
anything else skipped. Line and scatter charts can take several Y columns; numbered columns
(``Chest 1``, ``Chest 2 (3)``) can merge into one series (``Chest``), and ``only_hits`` keeps
just the cells a condition counted for (``QueryResult.hits``). They can also overlay more datasets,
each with its own X column and Y columns (``ChartSpec.datasets``).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

import numpy as np
import pandas as pd
from matplotlib.figure import Figure
from matplotlib.ticker import MaxNLocator

from .loader import cell_number, leading_number

AGGREGATES = ("count", "sum", "mean", "median", "min", "max")
MAX_CATEGORIES = 50
ANNOTATE_CELLS = 400  # heatmaps with at most this many cells show each cell's value
D2 = 1.128  # d2 for moving ranges of span 2 (as in imr-chart)
OUT_COLOR = "#d62728"
LIMIT_COLOR = "#c0392b"
CENTER_COLOR = "#2c7a3f"
BLANK = "(blank)"
OTHER = "Other"
MAX_POINT_LABELS = 300  # labels tried (text after a number, "Micrococcus luteus"); overlapping ones are dropped
MULTI_Y = ("line", "scatter")


class ChartError(ValueError):
    """The chosen columns can't make this chart; the message says why."""


@dataclass(frozen=True)
class KindInfo:
    label: str
    x: str  # "none" | "optional" | "required"
    y: str  # "optional" | "required"
    group: str  # "none" | "optional" | "required"
    agg: bool = False
    bins: bool = False
    stacked: bool = False
    x_label: str = "X"
    group_label: str = "Group by"


KINDS: dict[str, KindInfo] = {
    "line": KindInfo("Line / run chart", "optional", "required", "optional", x_label="X (blank: row order)"),
    "scatter": KindInfo("Scatter", "required", "required", "optional", group_label="Color by"),
    "bar": KindInfo("Bar", "required", "optional", "none", agg=True, x_label="Category"),
    "grouped_bar": KindInfo("Grouped / stacked bar", "required", "optional", "required", agg=True, stacked=True,
                            x_label="Category"),
    "histogram": KindInfo("Histogram", "none", "required", "optional", bins=True),
    "box": KindInfo("Box plot", "none", "required", "optional"),
    "pie": KindInfo("Pie", "required", "optional", "none", agg=True, x_label="Category"),
    "heatmap": KindInfo("Heatmap / pivot", "required", "optional", "required", agg=True, x_label="Columns",
                        group_label="Rows"),
    "imr": KindInfo("I-MR control chart", "optional", "required", "none", x_label="Order by (blank: row order)"),
}


@dataclass
class Dataset:
    """One overlay on a line / scatter chart: an X column (blank: row order) and its Y columns."""
    x: str = ""
    ys: list[str] = field(default_factory=list)


@dataclass
class ChartSpec:
    kind: str = "line"
    x: str = ""
    y: str = ""
    group: str = ""
    agg: str = "mean"
    bins: int = 20
    stacked: bool = False
    title: str = ""
    ys: list[str] = field(default_factory=list)  # line / scatter: more Y columns after ``y``
    merge_numbered: bool = True  # line / scatter: "Chest 1", "Chest 2 (3)" -> one "Chest" series
    only_hits: bool = False  # line / scatter: only cells a condition counted for
    point_labels: bool = False  # line / scatter: label points with the text after their number
    datasets: list[Dataset] = field(default_factory=list)  # line / scatter: more overlays, each its own X

    def y_columns(self) -> list[str]:
        """Every Y column: ``y`` then ``ys`` (without repeats); line and scatter only use ``ys``."""
        cols = [self.y] if self.y else []
        if self.kind in MULTI_Y:
            cols += [c for c in self.ys if c and c not in cols]
        return cols

    def all_datasets(self) -> list[Dataset]:
        """The first dataset (``x`` with :meth:`y_columns`), then the extra ones that have a Y column."""
        out = [Dataset(self.x, self.y_columns())]
        if self.kind in MULTI_Y:
            out += [Dataset(d.x, list(dict.fromkeys(c for c in d.ys if c))) for d in self.datasets
                    if any(d.ys)]
        return out


_COPY = re.compile(r"\s*\(\d+\)$")
_NUMBERED = re.compile(r"\s*\d+$")


def series_name(column: str) -> str:
    """The series a numbered column belongs to: ``Chest 1 (2)`` -> ``Chest``, ``Hand 2`` -> ``Hand``."""
    base = _NUMBERED.sub("", _COPY.sub("", str(column)))
    return base or str(column)


# --- reading values ------------------------------------------------------------

def _is_blank(v) -> bool:
    return v is None or (not isinstance(v, str) and pd.isna(v)) or (isinstance(v, str) and not v.strip())


def _to_float(v) -> float:
    if _is_blank(v) or isinstance(v, (bool, np.bool_)):
        return np.nan
    if isinstance(v, (pd.Timestamp, np.datetime64)):
        return np.nan
    return cell_number(v)


def _point_label(v) -> str:
    """The text after a cell's number (``1 Micrococcus luteus`` -> ``Micrococcus luteus``), else ''."""
    if not isinstance(v, str):
        return ""
    lead = leading_number(v)
    return lead[1] if lead else ""


def numeric(s: pd.Series) -> pd.Series:
    """``s`` as floats; blanks and values that aren't numbers become NaN."""
    if pd.api.types.is_bool_dtype(s):
        return pd.Series(np.nan, index=s.index)
    if pd.api.types.is_numeric_dtype(s):
        return s.astype(float)
    return s.map(_to_float).astype(float)


def is_date(s: pd.Series) -> bool:
    if pd.api.types.is_datetime64_any_dtype(s):
        return True
    vals = s.dropna()
    return len(vals) > 0 and all(isinstance(v, pd.Timestamp) for v in vals)


def is_numeric(s: pd.Series, share: float = 0.8) -> bool:
    """True when at least ``share`` of the non-blank values are numbers."""
    if is_date(s):
        return False
    present = s[~s.map(_is_blank)]
    if present.empty:
        return False
    return numeric(present).notna().mean() >= share


def numeric_columns(df: pd.DataFrame) -> list[str]:
    return [str(c) for c in df.columns if is_numeric(df[c])]


def label(v) -> str:
    if _is_blank(v):
        return BLANK
    if isinstance(v, pd.Timestamp):
        return v.strftime("%Y-%m-%d") if v == v.normalize() else str(v)
    if isinstance(v, float) and v.is_integer():
        return str(int(v))
    return str(v)


def _column(df: pd.DataFrame, name: str, role: str) -> pd.Series:
    if name not in df.columns:
        raise ChartError(f"Choose a column for {role}" if not name else f"No column '{name}' in this table")
    return df[name]


def _y_values(df: pd.DataFrame, y: str, notes: list[str]) -> pd.Series:
    raw = _column(df, y, "Y")
    vals = numeric(raw)
    if vals.notna().sum() == 0:
        raise ChartError(f"'{y}' has no numeric values to plot")
    skipped = int((vals.isna() & ~raw.map(_is_blank)).sum())
    if skipped:
        notes.append(f"{skipped} non-numeric value{'s' if skipped != 1 else ''} in '{y}' skipped")
    return vals


def _x_axis(s: pd.Series) -> tuple[pd.Series, list[str] | None]:
    """Plot positions for ``s``: dates and numbers as they are; anything else as category codes,
    returned with the category labels for the ticks."""
    if is_date(s):
        return pd.to_datetime(s, errors="coerce"), None
    if is_numeric(s):
        return numeric(s), None
    labels = s.map(label)
    cats = list(dict.fromkeys(labels))
    codes = {c: i for i, c in enumerate(cats)}
    return labels.map(codes).astype(float), cats


def _sort_key(s: pd.Series):
    if is_date(s):
        return pd.to_datetime(s, errors="coerce")
    if is_numeric(s):
        return numeric(s)
    return None


def _groups(df: pd.DataFrame, group: str) -> list[tuple[str, pd.Index]]:
    """(label, row index) per group in first-seen order, or one unnamed group of all rows."""
    if not group:
        return [("", df.index)]
    labels = _column(df, group, "the group").map(label)
    order = list(dict.fromkeys(labels))
    if len(order) > MAX_CATEGORIES:
        raise ChartError(f"'{group}' has {len(order)} distinct values; group by a column with at most "
                         f"{MAX_CATEGORIES}")
    return [(g, labels.index[labels == g]) for g in order]


# --- aggregation ------------------------------------------------------------------

def aggregate(df: pd.DataFrame, x: str, y: str = "", group: str = "", agg: str = "count") -> pd.DataFrame:
    """One row per X category, one column per group (a single column ``value`` without a group),
    holding ``agg`` of Y; with no Y (or ``count``), the number of rows / numeric Y values."""
    if agg not in AGGREGATES:
        raise ChartError(f"Aggregate must be one of {', '.join(AGGREGATES)}")
    xs = _column(df, x, "X")
    if not y and agg != "count":
        agg = "count"
    vals = numeric(_column(df, y, "Y")) if y else pd.Series(1.0, index=df.index)
    frame = pd.DataFrame({"x": xs.map(label), "g": df[group].map(label) if group else "value", "v": vals})
    if agg == "count":
        out = frame.dropna(subset=["v"]).groupby(["x", "g"], sort=False)["v"].count()
    else:
        out = frame.dropna(subset=["v"]).groupby(["x", "g"], sort=False)["v"].agg(agg)
    table = out.unstack("g")
    rows = list(dict.fromkeys(frame["x"]))
    cols = list(dict.fromkeys(frame["g"]))
    key = _sort_key(xs)
    if key is not None:
        first = pd.Series(key.to_numpy(), index=frame["x"].to_numpy())
        first = first[~first.index.duplicated()]
        rows = sorted(rows, key=lambda r: (pd.isna(first.get(r)), first.get(r) if pd.notna(first.get(r)) else 0))
    table = table.reindex(index=rows, columns=cols)
    if agg == "count":
        table = table.fillna(0)
    table.index.name, table.columns.name = x, (group or None)
    return table


def _limit(table: pd.DataFrame, notes: list[str], lump: bool) -> pd.DataFrame:
    """At most MAX_CATEGORIES rows, keeping the largest; ``lump`` sums the rest into 'Other'."""
    if len(table) <= MAX_CATEGORIES:
        return table
    total = table.abs().sum(axis=1)
    keep = total.sort_values(ascending=False).index[: MAX_CATEGORIES - (1 if lump else 0)]
    rest = table.drop(index=keep)
    out = table.loc[[r for r in table.index if r in set(keep)]]
    if lump:
        out = pd.concat([out, rest.sum().to_frame(OTHER).T])
        notes.append(f"{len(rest)} smaller categories grouped as '{OTHER}'")
    else:
        notes.append(f"Showing the {MAX_CATEGORIES} largest of {len(table)} categories")
    return out


def _agg_label(spec: ChartSpec) -> str:
    if not spec.y or spec.agg == "count":
        return f"Count of {spec.y}" if spec.y else "Rows"
    return f"{spec.agg.capitalize()} of {spec.y}"


# --- I-MR ---------------------------------------------------------------------------

@dataclass
class ImrStats:
    values: np.ndarray
    moving_ranges: np.ndarray  # first entry NaN
    mean: float
    mr_bar: float
    ucl: float
    lcl: float
    mr_ucl: float

    @property
    def out_of_limits(self) -> np.ndarray:
        return (self.values > self.ucl) | (self.values < self.lcl)


def imr_stats(values, k: float = 3.0) -> ImrStats:
    """Individuals and moving-range limits: mean ± k·MR̄/d2 and MR UCL = (1 + k·d3/d2)·MR̄
    (= 2.66·MR̄ and 3.267·MR̄ for k = 3)."""
    v = np.asarray(values, dtype=float)
    if len(v) < 2:
        raise ChartError("An I-MR chart needs at least 2 numeric values")
    mr = np.concatenate([[np.nan], np.abs(np.diff(v))])
    mean, mr_bar = float(v.mean()), float(np.nanmean(mr))
    sigma = mr_bar / D2
    return ImrStats(v, mr, mean, mr_bar, mean + k * sigma, mean - k * sigma, mr_bar * (1 + k * 0.8525 / D2))


# --- drawing ---------------------------------------------------------------------------

def _color(i: int):
    from matplotlib import colormaps

    return colormaps["tab10"](i % 10)


def _category_ticks(ax, cats: list[str], axis: str = "x"):
    if len(cats) > MAX_CATEGORIES:
        return
    pos = range(len(cats))
    if axis == "x":
        ax.set_xticks(pos, cats, rotation=45 if max(map(len, cats), default=0) > 6 or len(cats) > 8 else 0,
                      ha="right" if len(cats) > 8 or max(map(len, cats), default=0) > 6 else "center")
    else:
        ax.set_yticks(pos, cats)


def _y_series(df, cols, spec) -> list[tuple[str, list[str]]]:
    """(label, Y columns) per series of one dataset: one per column, or numbered columns merged by
    :func:`series_name`."""
    for c in cols:
        _column(df, c, "Y")
    if not spec.merge_numbered:
        return [(c, [c]) for c in cols]
    merged: dict[str, list[str]] = {}
    for c in cols:
        merged.setdefault(series_name(c), []).append(c)
    return list(merged.items())


def _series_values(df, cols, spec, hits, notes) -> pd.Series:
    """Every column's numeric values as one Series indexed by (column, row): NaN where blank,
    unreadable, or (with ``only_hits``) not a cell a condition counted for."""
    parts = []
    for c in cols:
        raw = df[c]
        vals = numeric(raw)
        keep = np.ones(len(df), dtype=bool)
        if spec.only_hits and hits is not None:
            keep = hits[c].to_numpy(dtype=bool) if c in hits.columns else np.zeros(len(df), dtype=bool)
        skipped = int((vals.isna() & ~raw.map(_is_blank) & keep).sum())
        if skipped:
            notes.append(f"{skipped} non-numeric value{'s' if skipped != 1 else ''} in '{c}' skipped")
        parts.append(vals.where(keep))
    return pd.concat(parts, keys=cols)


_X_KINDS = {"date": "dates", "number": "numbers", "text": "text"}


def _x_kind(s: pd.Series) -> str:
    return "date" if is_date(s) else "number" if is_numeric(s) else "text"


def _dataset_xs(df, datasets, spec, notes) -> tuple[list[pd.Series], list[str] | None]:
    """Plot positions of every dataset's X (row order where blank), and the category labels when the
    X columns hold text (shared by all datasets, so equal labels line up). Every X must be the same kind."""
    kinds = []
    for i, ds in enumerate(datasets):
        if ds.x:
            kinds.append(_x_kind(_column(df, ds.x, "X")))
        elif spec.kind == "scatter":
            raise ChartError("Choose a column for X" + (f" in dataset {i + 1}" if i else ""))
        else:
            kinds.append("number")  # row order
    if len(set(kinds)) > 1:
        raise ChartError("The datasets' X columns must all be dates, all numbers or all text ("
                         + ", ".join(f"{ds.x or 'row order'}: {_X_KINDS[k]}" for ds, k in zip(datasets, kinds)) + ")")
    if any(not ds.x for ds in datasets) and not any(is_date(df[c]) for c in df.columns):
        notes.append("X is row order; add a date column (e.g. Monitor Date) to the outputs to plot over time")
    if kinds[0] == "text":
        labels = [df[ds.x].map(label) for ds in datasets]
        cats = list(dict.fromkeys(v for lab in labels for v in lab))
        codes = {c: i for i, c in enumerate(cats)}
        return [lab.map(codes).astype(float) for lab in labels], cats
    rows = pd.Series(np.arange(1, len(df) + 1, dtype=float), index=df.index)
    return [_x_axis(df[ds.x])[0] if ds.x else rows for ds in datasets], None


def _multi_y(fig, df, spec, notes, hits, plot):
    """Line / scatter over every dataset's Y columns: one series per (dataset, group, Y series)."""
    ax = fig.add_subplot()
    datasets = spec.all_datasets()
    if not datasets[0].ys:
        raise ChartError("Choose a numeric column for Y")
    series = [_y_series(df, ds.ys, spec) for ds in datasets]
    if spec.only_hits and hits is None:
        notes.append("No condition results to limit the points to; showing every value")
    all_xs, cats = _dataset_xs(df, datasets, spec, notes)
    ax.set_xlabel(" / ".join(dict.fromkeys(ds.x or "Row" for ds in datasets)))
    groups = _groups(df, spec.group)
    named = sum(len(s) for s in series) > 1
    overlaid = len(datasets) > 1
    labels: list[tuple[float, float, str]] = []
    ax.wbq_points = []  # (numeric xy array, tooltip per point) per series, for hovering in the app
    drawn = n = 0
    for ds, ds_series, xs in zip(datasets, series, all_xs):
        for gname, idx in groups:
            for sname, cols in ds_series:
                sub = df.loc[idx]
                ys = _series_values(sub, cols, spec, hits.loc[idx] if hits is not None else None, notes)
                d = pd.DataFrame({
                    "x": pd.concat([xs[idx]] * len(cols), keys=cols),
                    "y": ys,
                    "text": pd.concat([sub[c].map(_point_label) for c in cols], keys=cols),
                }).dropna(subset=["x", "y"])
                if ds.x and cats is None:
                    d = d.sort_values("x", kind="stable")
                name = " · ".join(p for p in (sname if named else "", gname) if p)
                if overlaid:
                    name = f"{name or sname} ({ds.x or 'row order'})"
                label = name or None
                artist = plot(ax, d, _color(n), label)
                n += 1
                xy = artist.get_xydata() if hasattr(artist, "get_xydata") else artist.get_offsets()
                raw = pd.concat([sub[c] for c in cols], keys=cols).reindex(d.index)
                ax.wbq_points.append((np.asarray(xy, dtype=float), [
                    _tooltip(label or sname, col, raw_v, ds.x, df.loc[row, ds.x] if ds.x else None)
                    for (col, row), raw_v in zip(d.index, raw)]))
                drawn += len(d)
                if spec.point_labels:
                    labels += [(x, y, t) for x, y, t in zip(d["x"], d["y"], d["text"]) if t]
    if not drawn:
        raise ChartError("No numeric values to plot" + (" in the cells the conditions matched"
                                                        if spec.only_hits and hits is not None else ""))
    # Highest values first, so the rarer large counts keep their labels when space runs out.
    labels.sort(key=lambda t: -t[1])
    ax.wbq_labels = [ax.annotate(text, (x, y), xytext=(4, 4), textcoords="offset points", fontsize=7, alpha=0.85)
                     for x, y, text in labels[:MAX_POINT_LABELS]]
    ax.wbq_label_total = len(labels)
    if cats:
        _category_ticks(ax, cats)
    y_cols = list(dict.fromkeys(c for ds in datasets for c in ds.ys))
    names = list(dict.fromkeys(name for s in series for name, _ in s))
    ax.set_ylabel(y_cols[0] if len(y_cols) == 1 else ", ".join(names) if len(names) <= 4 else "Value")
    return [ax]


def _tooltip(series: str, column: str, raw, xname: str, xval) -> str:
    """What hovering a point shows: series, column and cell, and the X value."""
    lines = [f"{series}" if series == column else f"{series} — {column}", label(raw)]
    if xname:
        lines.append(f"{xname}: {label(xval)}")
    return "\n".join(lines)


_LABEL_SPOTS = (((4, 4), "left", "bottom"), ((-4, 4), "right", "bottom"), ((4, -4), "left", "top"),
                ((-4, -4), "right", "top"))


def _thin_labels(fig, axes) -> int:
    """Place each point label (in the order drawn) at the first corner around its point where it stays
    inside the plot without overlapping a label already placed; remove it when no corner works.
    Returns how many of the labels asked for aren't shown."""
    hidden = 0
    try:
        renderer = fig.canvas.get_renderer() if hasattr(fig.canvas, "get_renderer") else fig._get_renderer()
    except Exception:  # noqa: BLE001 - no renderer: keep every label
        return 0
    for ax in axes:
        texts = getattr(ax, "wbq_labels", None)
        if not texts:
            continue
        area = ax.get_window_extent(renderer)
        legend = ax.get_legend()
        kept = [legend.get_window_extent(renderer)] if legend is not None else []
        legend_boxes = len(kept)
        for t in texts:
            for (dx, dy), ha, va in _LABEL_SPOTS:  # above right, above left, below right, below left
                t.set_position((dx, dy))
                t.set_horizontalalignment(ha)
                t.set_verticalalignment(va)
                box = t.get_window_extent(renderer).padded(5)  # a clear gap, so neighbours don't read as one
                inside = (area.x0 <= box.x0 + 5 and box.x1 - 5 <= area.x1 and area.y0 <= box.y0 + 5
                          and box.y1 - 5 <= area.y1)
                if inside and not any(box.overlaps(k) for k in kept):
                    kept.append(box)
                    break
            else:
                t.remove()
        hidden += ax.wbq_label_total - (len(kept) - legend_boxes)
    return hidden


def _line(fig, df, spec, notes, hits=None):
    return _multi_y(fig, df, spec, notes, hits, lambda ax, d, color, label: ax.plot(
        d["x"], d["y"], marker="o", markersize=3, linewidth=1.3, color=color, label=label)[0])


def _scatter(fig, df, spec, notes, hits=None):
    return _multi_y(fig, df, spec, notes, hits, lambda ax, d, color, label: ax.scatter(
        d["x"], d["y"], s=22, alpha=0.8, color=color, label=label))


def _bar(fig, df, spec, notes):
    ax = fig.add_subplot()
    if spec.y:
        _y_values(df, spec.y, notes)
    table = _limit(aggregate(df, spec.x, spec.y, "", spec.agg), notes, lump=False)
    vals = table.iloc[:, 0].fillna(0)
    ax.bar(range(len(vals)), vals.to_numpy(), color=_color(0))
    _category_ticks(ax, [str(c) for c in table.index])
    ax.set_xlabel(spec.x)
    ax.set_ylabel(_agg_label(spec))
    return [ax]


def _grouped_bar(fig, df, spec, notes):
    ax = fig.add_subplot()
    if not spec.group:
        raise ChartError("Choose a column to group the bars by")
    if spec.y:
        _y_values(df, spec.y, notes)
    _groups(df, spec.group)  # checks the number of groups
    table = _limit(aggregate(df, spec.x, spec.y, spec.group, spec.agg), notes, lump=False)
    n = len(table.columns)
    pos = np.arange(len(table))
    bottom = np.zeros(len(table))
    width = 0.8 if spec.stacked else 0.8 / max(n, 1)
    for i, g in enumerate(table.columns):
        vals = table[g].fillna(0).to_numpy(dtype=float)
        if spec.stacked:
            ax.bar(pos, vals, width, bottom=bottom, color=_color(i), label=str(g))
            bottom += vals
        else:
            ax.bar(pos - 0.4 + width * (i + 0.5), vals, width, color=_color(i), label=str(g))
    _category_ticks(ax, [str(c) for c in table.index])
    ax.set_xlabel(spec.x)
    ax.set_ylabel(_agg_label(spec))
    return [ax]


def _histogram(fig, df, spec, notes):
    ax = fig.add_subplot()
    y = _y_values(df, spec.y, notes)
    edges = np.histogram_bin_edges(y.dropna().to_numpy(), bins=max(1, int(spec.bins)))
    groups = _groups(df, spec.group)
    for i, (name, idx) in enumerate(groups):
        vals = y[idx].dropna().to_numpy()
        ax.hist(vals, bins=edges, color=_color(i), alpha=0.55 if len(groups) > 1 else 0.9,
                edgecolor="white", linewidth=0.5, label=name or None)
    ax.set_xlabel(spec.y)
    ax.set_ylabel("Count")
    return [ax]


def _box(fig, df, spec, notes):
    ax = fig.add_subplot()
    y = _y_values(df, spec.y, notes)
    data, names = [], []
    for name, idx in _groups(df, spec.group):
        vals = y[idx].dropna().to_numpy()
        if len(vals):
            data.append(vals)
            names.append(name or spec.y)
    ax.boxplot(data, patch_artist=True, boxprops={"facecolor": (*_color(0)[:3], 0.35)},
               medianprops={"color": "black"})
    ax.set_xticks(range(1, len(names) + 1), names, rotation=45 if len(names) > 8 else 0,
                  ha="right" if len(names) > 8 else "center")
    if spec.group:
        ax.set_xlabel(spec.group)
    ax.set_ylabel(spec.y)
    return [ax]


def _pie(fig, df, spec, notes):
    ax = fig.add_subplot()
    if spec.y:
        _y_values(df, spec.y, notes)
    table = aggregate(df, spec.x, spec.y, "", spec.agg)
    vals = table.iloc[:, 0].fillna(0)
    dropped = int((vals <= 0).sum())
    vals = vals[vals > 0]
    if dropped:
        notes.append(f"{dropped} categor{'ies' if dropped != 1 else 'y'} with zero or negative values left out")
    if vals.empty:
        raise ChartError("Nothing positive to show in a pie chart")
    vals = _limit(vals.to_frame(), notes, lump=spec.agg in ("count", "sum") or not spec.y).iloc[:, 0]
    ax.pie(vals.to_numpy(), labels=[str(c) for c in vals.index], autopct="%1.0f%%", startangle=90,
           counterclock=False, colors=[_color(i) for i in range(len(vals))],
           wedgeprops={"edgecolor": "white", "linewidth": 1})
    ax.set_aspect("equal")
    ax.set_xlabel(_agg_label(spec))
    return [ax]


def _heatmap(fig, df, spec, notes):
    ax = fig.add_subplot()
    if not spec.group:
        raise ChartError("Choose a column for the heatmap rows")
    if spec.y:
        _y_values(df, spec.y, notes)
    _groups(df, spec.group)
    table = _limit(aggregate(df, spec.x, spec.y, spec.group, spec.agg), notes, lump=False).T
    data = table.to_numpy(dtype=float)
    im = ax.imshow(np.ma.masked_invalid(data), aspect="auto", cmap="viridis")
    fig.colorbar(im, ax=ax, label=_agg_label(spec))
    _category_ticks(ax, [str(c) for c in table.columns], "x")
    _category_ticks(ax, [str(r) for r in table.index], "y")
    if data.size <= ANNOTATE_CELLS:
        finite = data[np.isfinite(data)]
        mid = (finite.min() + finite.max()) / 2 if finite.size else 0
        for (r, c), v in np.ndenumerate(data):
            if np.isfinite(v):
                ax.text(c, r, f"{v:.4g}", ha="center", va="center", fontsize=8,
                        color="black" if v > mid else "white")
    ax.set_xlabel(spec.x)
    ax.set_ylabel(spec.group)
    return [ax]


def _imr(fig, df, spec, notes):
    y = _y_values(df, spec.y, notes)
    d = pd.DataFrame({"y": y}, index=df.index)
    xlabel = "Observation"
    if spec.x:
        xcol = _column(df, spec.x, "the order")
        key = _sort_key(xcol)
        d["x"] = key if key is not None else xcol.map(label)
        d = d.dropna(subset=["y"]).sort_values("x", kind="stable")
        xlabel = f"Observation (ordered by {spec.x})"
    else:
        d = d.dropna(subset=["y"])
    st = imr_stats(d["y"].to_numpy())
    n = np.arange(1, len(st.values) + 1)
    ax_i, ax_mr = fig.subplots(2, 1, sharex=True, gridspec_kw={"height_ratios": [3, 2]})

    ax_i.plot(n, st.values, marker="o", markersize=3, linewidth=1.2, color=_color(0))
    out = st.out_of_limits
    if out.any():
        ax_i.scatter(n[out], st.values[out], color=OUT_COLOR, s=30, zorder=3, label="Outside limits")
        notes.append(f"{int(out.sum())} point{'s' if out.sum() != 1 else ''} outside the control limits")
    for val, color, style, text in ((st.ucl, LIMIT_COLOR, "--", "UCL"), (st.mean, CENTER_COLOR, "-", "X̄"),
                                    (st.lcl, LIMIT_COLOR, "--", "LCL")):
        ax_i.axhline(val, color=color, linestyle=style, linewidth=1)
        ax_i.annotate(f"{text}={val:.4g}", (1, val), xycoords=("axes fraction", "data"), xytext=(4, 0),
                      textcoords="offset points", va="center", fontsize=8, color=color)
    ax_i.set_ylabel(spec.y)
    ax_i.set_title("Individuals", fontsize=10, loc="left")

    ax_mr.plot(n, st.moving_ranges, marker="o", markersize=3, linewidth=1.2, color=_color(0))
    mr_out = st.moving_ranges > st.mr_ucl
    if mr_out.any():
        ax_mr.scatter(n[mr_out], st.moving_ranges[mr_out], color=OUT_COLOR, s=30, zorder=3)
    for val, color, style, text in ((st.mr_ucl, LIMIT_COLOR, "--", "UCL"), (st.mr_bar, CENTER_COLOR, "-", "MR̄"),
                                    (0.0, LIMIT_COLOR, "--", "LCL")):
        ax_mr.axhline(val, color=color, linestyle=style, linewidth=1)
        ax_mr.annotate(f"{text}={val:.4g}", (1, val), xycoords=("axes fraction", "data"), xytext=(4, 0),
                       textcoords="offset points", va="center", fontsize=8, color=color)
    ax_mr.set_ylabel("Moving range")
    ax_mr.xaxis.set_major_locator(MaxNLocator(integer=True))
    ax_mr.set_xlabel(xlabel)
    ax_mr.set_title("Moving range", fontsize=10, loc="left")
    return [ax_i, ax_mr]


_DRAW = {"line": _line, "scatter": _scatter, "bar": _bar, "grouped_bar": _grouped_bar, "histogram": _histogram,
         "box": _box, "pie": _pie, "heatmap": _heatmap, "imr": _imr}


def draw(fig: Figure, df: pd.DataFrame, spec: ChartSpec, hits: pd.DataFrame | None = None) -> list[str]:
    """Clear ``fig`` and draw ``spec`` from ``df``. ``hits`` (same shape as ``df``) marks the cells
    a condition counted for, used by ``spec.only_hits``. Returns notes about skipped data;
    raises :class:`ChartError` when the chosen columns can't make the chart."""
    fig.clear()
    if spec.kind not in KINDS:
        raise ChartError(f"Unknown chart type '{spec.kind}'")
    info = KINDS[spec.kind]
    if df.empty:
        raise ChartError("The table has no rows to chart")
    if info.y == "required" and not spec.y:
        raise ChartError("Choose a numeric column for Y")
    if info.x == "required" and not spec.x:
        raise ChartError(f"Choose a column for {info.x_label}")
    notes: list[str] = []
    try:
        axes = _DRAW[spec.kind](fig, df, spec, notes, hits) if spec.kind in MULTI_Y \
            else _DRAW[spec.kind](fig, df, spec, notes)
    except ChartError:
        fig.clear()
        raise
    main = axes[0]
    if main.get_legend_handles_labels()[0] and spec.kind != "pie":
        main.legend(fontsize=8, title=spec.group or None, title_fontsize=8)
    if spec.kind not in ("pie", "heatmap"):
        for ax in axes:
            ax.grid(True, alpha=0.3)
    if spec.title:
        fig.suptitle(spec.title)
    if spec.kind in MULTI_Y and any(ds.x in df.columns and is_date(df[ds.x]) for ds in spec.all_datasets()):
        fig.autofmt_xdate()
    try:
        fig.tight_layout()
    except (ValueError, np.linalg.LinAlgError):  # very small canvases
        pass
    hidden = _thin_labels(fig, axes)
    if hidden:
        notes.append(f"{hidden} point label{'s' if hidden != 1 else ''} hidden so labels don't overlap"
                     " (hover a point to see its text)")
    return notes
