"""Record what the Python chart code plots, for the Rust chart port's parity tests.

    .venv/bin/python tests/golden/generate_charts.py

Writes tests/golden/charts.json: the input tables, and for each chart spec either the error message or
what matplotlib drew on each axes (labels, line and scatter data, bar rectangles, pie wedges, texts,
tick labels for categories) plus the notes. Dates on axes are matplotlib date numbers (days since
1970-01-01). Notes about point labels hidden for overlap depend on the renderer and are left out.
"""

from __future__ import annotations

import json
import sys
from dataclasses import asdict
from pathlib import Path

import matplotlib

matplotlib.use("Agg")

import numpy as np  # noqa: E402
import pandas as pd  # noqa: E402
from matplotlib.figure import Figure  # noqa: E402
from matplotlib.patches import Rectangle, Wedge  # noqa: E402

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent.parent))
sys.path.insert(0, str(HERE))

from generate import EM, LOTS, RESULTS, cells, tag  # noqa: E402
from wbquery.charts import ChartError, ChartSpec, Dataset, aggregate, draw, imr_stats, is_numeric, numeric, \
    numeric_columns, series_name  # noqa: E402
from wbquery.conditions import Condition, Group  # noqa: E402
from wbquery.query import Query, Source, run_query  # noqa: E402

DF = pd.concat([LOTS, RESULTS], axis=1)
OVERLAY = pd.DataFrame({
    "Monitor Date": pd.to_datetime(["2026-01-05", "2026-01-06", "2026-01-07"]),
    "Sample Date": pd.to_datetime(["2026-02-01", "2026-02-02", "2026-02-03"]),
    "Room": ["A", "B", "C"],
    "Lab": ["C", "D", "A"],
    "Chest 1": [1, 2, 3],
    "Hand 1": [7, 8, 9],
})
MANY = pd.DataFrame({"c": [f"k{i}" for i in range(80)], "v": range(80)})
MIXED = pd.DataFrame({"v": [1, 2, "x", None]})
SPC = pd.DataFrame({
    "Day": pd.date_range("2026-03-01", periods=24),
    "Shift": ["A", "B", "C"] * 8,
    "Count": [5, 6, 5, 7, 6, 5, 30, 6, 5, 6, 7, 5, 6, 5, -20, 6, 5, 7, 6, 5, 6, 5, 7, 6],
    "Loss": [-1, 2, 3, -4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24],
})

RED, BLUE = "#ff0000", "#0000ff"


def person(n, suffix, color):
    return Group("AND", [Condition("*", f"Person {n}", "=", "KSG"),
                         Group("OR", [Condition("*", f"Chest 1{suffix}", "<>", "0", color=color),
                                      Condition("*", f"Hand 1{suffix}", "<>", "0", color=color)])])


def em_result():
    q = Query([Source("EM", "em.xlsx", "Sheet")], Group("OR", [person(1, "", RED), person(2, " (2)", BLUE)]),
              {"EM": list(EM.columns)})
    import wbquery.query as wq
    wq.file_sha256 = lambda p: ""
    res = run_query(q, {"EM": EM.set_axis([5, 6, 7, 8])})
    return res.tables["EM"], res.hits["EM"]


EM_TABLE, EM_HITS = em_result()
EMR = EM.reset_index(drop=True)
FRAMES = {"DF": DF, "OVERLAY": OVERLAY, "MANY": MANY, "MIXED": MIXED, "SPC": SPC, "EM_TABLE": EM_TABLE, "EM": EMR}

EM_YS = dict(x="Monitor Date", y="Chest 1", ys=["Hand 1", "Chest 1 (2)", "Hand 1 (2)"])
CASES = [
    ("line", "DF", ChartSpec("line", x="Made", y="Value"), False),
    ("scatter", "DF", ChartSpec("scatter", x="Pct", y="Value", group="Status"), False),
    ("bar", "DF", ChartSpec("bar", x="Status", y="Value", agg="sum"), False),
    ("grouped_bar_stacked", "DF", ChartSpec("grouped_bar", x="Status", y="", group="Notes", stacked=True), False),
    ("grouped_bar", "DF", ChartSpec("grouped_bar", x="Status", y="Value", group="Notes", agg="max"), False),
    ("histogram", "DF", ChartSpec("histogram", y="Value", group="Status", bins=5), False),
    ("histogram_one", "SPC", ChartSpec("histogram", y="Count", bins=7), False),
    ("box", "DF", ChartSpec("box", y="Value", group="Status"), False),
    ("box_fliers", "SPC", ChartSpec("box", y="Count", group="Shift"), False),
    ("pie", "DF", ChartSpec("pie", x="Status", agg="count"), False),
    ("pie_negative", "SPC", ChartSpec("pie", x="Shift", y="Loss", agg="min"), False),
    ("heatmap", "DF", ChartSpec("heatmap", x="Status", y="Value", group="Notes", agg="mean"), False),
    ("heatmap_count", "SPC", ChartSpec("heatmap", x="Shift", group="Count", agg="count"), False),
    ("imr", "DF", ChartSpec("imr", x="Made", y="Value", title="Value I-MR"), False),
    ("imr_out", "SPC", ChartSpec("imr", y="Count"), False),
    ("imr_text_order", "SPC", ChartSpec("imr", x="Shift", y="Count"), False),
    ("line_row_order", "DF", ChartSpec("line", y="Pct"), False),
    ("line_text_x_group", "DF", ChartSpec("line", x="Status", y="Pct", group="Notes"), False),
    ("line_median", "SPC", ChartSpec("bar", x="Shift", y="Count", agg="median"), False),
    ("bar_count_dates", "DF", ChartSpec("bar", x="Made"), False),
    ("em_merged", "EM_TABLE", ChartSpec("line", point_labels=True, **EM_YS), False),
    ("em_hits", "EM_TABLE", ChartSpec("line", point_labels=True, only_hits=True, **EM_YS), True),
    ("em_unmerged", "EM_TABLE", ChartSpec("line", only_hits=True, merge_numbered=False, **EM_YS), True),
    ("em_hits_none", "EM", ChartSpec("line", y="Hand 1", only_hits=True), "none"),
    ("em_single", "EM", ChartSpec("line", x="Monitor Date", y="Hand 1"), False),
    ("em_scatter", "EM", ChartSpec("scatter", x="Monitor Date", y="Chest 1", ys=["Hand 1"]), False),
    ("em_hits_missing", "EM", ChartSpec("line", y="Hand 1", only_hits=True), False),
    ("overlay", "OVERLAY", ChartSpec("line", x="Monitor Date", y="Chest 1",
                                     datasets=[Dataset("Sample Date", ["Hand 1"]), Dataset("Room", [])]), False),
    ("overlay_text", "OVERLAY", ChartSpec("scatter", x="Room", y="Chest 1", datasets=[Dataset("Lab", ["Hand 1"])]),
     False),
    ("overlay_kinds", "OVERLAY", ChartSpec("line", x="Monitor Date", y="Chest 1", datasets=[Dataset("Room", ["Hand 1"])]),
     False),
    ("overlay_no_x", "OVERLAY", ChartSpec("scatter", x="Room", y="Chest 1", datasets=[Dataset("", ["Hand 1"])]), False),
    ("overlay_row_order", "OVERLAY", ChartSpec("line", y="Chest 1", datasets=[Dataset("", ["Hand 1"])]), False),
    ("many_bar", "MANY", ChartSpec("bar", x="c", y="v", agg="sum"), False),
    ("many_pie", "MANY", ChartSpec("pie", x="c", y="v", agg="sum"), False),
    ("many_pie_mean", "MANY", ChartSpec("pie", x="c", y="v", agg="mean"), False),
    ("many_box", "MANY", ChartSpec("box", y="v", group="c"), False),
    ("mixed_hist", "MIXED", ChartSpec("histogram", y="v"), False),
    ("err_no_numeric", "DF", ChartSpec("histogram", y="Status"), False),
    ("err_group", "DF", ChartSpec("grouped_bar", x="Status"), False),
    ("err_rows", "DF", ChartSpec("heatmap", x="Status"), False),
    ("err_y", "DF", ChartSpec("box"), False),
    ("err_x", "DF", ChartSpec("scatter", y="Value"), False),
    ("err_missing_col", "DF", ChartSpec("line", y="Nope"), False),
    ("err_agg", "DF", ChartSpec("bar", x="Status", agg="mode"), False),
    ("err_kind", "DF", ChartSpec("radar", y="Value"), False),
    ("err_imr_short", "MIXED", ChartSpec("imr", y="v", x=""), False),
]


def axes_doc(ax):
    def xy(a):
        return [[None if not np.isfinite(v) else float(v) for v in p] for p in np.asarray(a, dtype=float)]

    legend = ax.get_legend()
    doc = {
        "xlabel": ax.get_xlabel(), "ylabel": ax.get_ylabel(), "title": ax.get_title(loc="left"),
        "lines": [{"label": ln.get_label(), "xy": xy(ln.get_xydata())} for ln in ax.get_lines()],
        "scatter": [{"label": c.get_label(), "xy": xy(c.get_offsets())} for c in ax.collections
                    if hasattr(c, "get_offsets") and len(c.get_offsets())
                    and type(c).__name__ == "PathCollection"],
        "bars": [[float(p.get_x()), float(p.get_y()), float(p.get_width()), float(p.get_height())]
                 for p in ax.patches if isinstance(p, Rectangle)],
        "wedges": [[float(p.theta1), float(p.theta2)] for p in ax.patches if isinstance(p, Wedge)],
        "texts": [t.get_text() for t in ax.texts],
        "legend": [t.get_text() for t in legend.get_texts()] if legend else None,
        "legend_title": legend.get_title().get_text() if legend else None,
        "xticklabels": [t.get_text() for t in ax.get_xticklabels()],
        "yticklabels": [t.get_text() for t in ax.get_yticklabels()],
        "images": [[[None if np.ma.is_masked(v) or not np.isfinite(v) else float(v) for v in row]
                    for row in np.ma.filled(im.get_array().astype(float), np.nan)] for im in ax.images],
    }
    return doc


def main():
    frames = {name: {"columns": [str(c) for c in df.columns], "cells": {str(c): cells(df[c]) for c in df.columns}}
              for name, df in FRAMES.items()}
    hits = {"EM_TABLE": {str(c): [bool(b) for b in EM_HITS[c]] for c in EM_HITS.columns},
            "EM": {str(c): [False] * len(EMR) for c in EMR.columns}}
    out = []
    for name, frame, spec, use_hits in CASES:
        df = FRAMES[frame]
        h = None
        if use_hits is True:
            h = EM_HITS
        elif use_hits == "none":
            h = pd.DataFrame(False, index=df.index, columns=df.columns)
        case = {"name": name, "frame": frame, "hits": {True: frame, "none": "EM", False: None}[use_hits],
                "spec": asdict(spec)}
        fig = Figure(figsize=(8, 5))
        try:
            notes = draw(fig, df, spec, h)
        except ChartError as exc:
            case["error"] = str(exc)
            out.append(case)
            continue
        case["notes"] = [n for n in notes if "hidden so labels" not in n]
        case["suptitle"] = fig._suptitle.get_text() if fig._suptitle else ""
        case["axes"] = [axes_doc(ax) for ax in fig.axes]
        out.append(case)

    extra = {
        "series_name": {c: series_name(c) for c in ("Chest 1", "Chest 1 (2)", "Hand 2 (6)", "Sleeve", "P10", "12",
                                                    "Room (3)", "A 1 (2) (3)")},
        "numeric": [tag(v) for v in numeric(pd.Series(["50%", "1,000", "5e7", "abc", None, 3, True, " "]))],
        "is_numeric": {c: bool(is_numeric(DF[c])) for c in DF.columns},
        "numeric_columns": numeric_columns(DF),
        "imr": {k: (tag(v) if not isinstance(v, np.ndarray) else [tag(x) for x in v])
                for k, v in asdict(imr_stats([1, 2, 3, 2, 1, 9, 2])).items()},
        "aggregate": [],
    }
    for x, y, g, agg in [("Status", "Value", "", "sum"), ("Status", "", "", "count"), ("Status", "Value", "Notes", "mean"),
                         ("Made", "", "", "count"), ("Pct", "Value", "", "median"), ("Lot", "Value", "Status", "min"),
                         ("Status", "", "", "sum")]:
        t = aggregate(DF, x, y, g, agg)
        extra["aggregate"].append({"x": x, "y": y, "group": g, "agg": agg, "rows": [str(r) for r in t.index],
                                   "columns": [str(c) for c in t.columns],
                                   "values": [[tag(v) for v in row] for row in t.to_numpy(dtype=float)]})
    (HERE / "charts.json").write_text(json.dumps({"frames": frames, "hits": hits, "cases": out, **extra}, indent=1,
                                                 ensure_ascii=False) + "\n", encoding="utf-8")
    print("wrote", HERE / "charts.json", len(out), "cases")


if __name__ == "__main__":
    main()
