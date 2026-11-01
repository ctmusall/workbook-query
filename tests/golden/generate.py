"""Generate the golden fixtures the Rust port is tested against, from the Python implementation.

Run once from the repo root while the Python package still exists:

    .venv/bin/python tests/golden/generate.py

It writes input workbooks under tests/golden/data/ and tests/golden/smoke/, and the expected
results as JSON (plus normalized HTML) under tests/golden/. Cells are encoded as JSON: null for
blank, true/false, integers, floats (always with a decimal point or exponent), strings, and
{"d": "YYYY-MM-DDTHH:MM:SS"} for dates. Kept after the Python code is removed, for reference.
"""

from __future__ import annotations

import json
import math
import sys
from datetime import date, datetime
from pathlib import Path

import numpy as np
import pandas as pd

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
sys.path.insert(0, str(ROOT))

from wbquery import __version__, loader  # noqa: E402
from wbquery.conditions import (Condition, Group, column_key, normalize_color, parse_number,  # noqa: E402
                                resolve_column, sort_columns, split_list, text_color)
from wbquery.export_html import render_html  # noqa: E402
from wbquery.export_xlsx import sheet_name, write_xlsx  # noqa: E402
from wbquery.query import Query, default_alias, normalize_key, run_query, unique_alias  # noqa: E402

DATA = HERE / "data"
SMOKE = HERE / "smoke"


def tag(v):
    if v is None or v is pd.NaT or v is pd.NA:
        return None
    if isinstance(v, (bool, np.bool_)):
        return bool(v)
    if isinstance(v, (int, np.integer)):
        return int(v)
    if isinstance(v, (float, np.floating)):
        f = float(v)
        if math.isnan(f):
            return None
        if math.isinf(f):
            return {"f": "inf" if f > 0 else "-inf"}
        return f
    if isinstance(v, str):
        return v
    if isinstance(v, (pd.Timestamp, datetime)):
        return {"d": pd.Timestamp(v).strftime("%Y-%m-%dT%H:%M:%S")}
    if isinstance(v, date):
        return {"d": v.strftime("%Y-%m-%dT00:00:00")}
    raise TypeError(f"can't tag {type(v)}: {v!r}")


def cells(series: pd.Series) -> list:
    return [tag(v) for v in series.tolist()]


def dump(name: str, doc):
    (HERE / name).write_text(json.dumps(doc, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")


# --- input files ---------------------------------------------------------------------

LOTS = pd.DataFrame({
    "Lot": ["A1", "A2", "A3", 101, None],
    "Status": ["Pass", "Fail", "pass", "Pass", "Fail"],
    "Made": [datetime(2024, 1, 5), datetime(2024, 2, 10), datetime(2024, 3, 1), datetime(2024, 3, 15),
             datetime(2024, 4, 1)],
    "Notes": [None, "retest", "ok", None, "orphan"],
})
OWNERS = pd.DataFrame({"Lot": ["A1", "A2"], "Owner": ["Kim", "Lee"]})
RESULTS = pd.DataFrame({
    "Lot ID": ["a1", "A2", "A2", 101.0, "Z9"],
    "Value": [5e7, 7e7, 6e7, 1e7, 3e7],
    "Pct": ["50%", "80%", "90%", "10%", "30%"],
})
EM = pd.DataFrame({
    "Monitor Date": pd.to_datetime(["2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08"]),
    "Person 1": ["KSG", "ZEN", "KSG", "LST"],
    "Chest 1": [0, 5, "1 Micrococcus luteus", 0],
    "Hand 1": [2, 0, 0, 0],
    "Person 2": ["TJH", "KSG", "ZEN", "KSG"],
    "Chest 1 (2)": [4, 0, 0, 0],
    "Hand 1 (2)": [0, 3, "TNTC", 0],
})


def write_inputs():
    DATA.mkdir(parents=True, exist_ok=True)
    SMOKE.mkdir(parents=True, exist_ok=True)
    with pd.ExcelWriter(DATA / "lots.xlsx") as xw:
        LOTS.to_excel(xw, sheet_name="Lots", index=False)
        OWNERS.to_excel(xw, sheet_name="Owners", index=False)
    with pd.ExcelWriter(DATA / "results.xlsx") as xw:
        RESULTS.to_excel(xw, sheet_name="Results", index=False)
    with pd.ExcelWriter(DATA / "em.xlsx") as xw:
        EM.to_excel(xw, sheet_name="Sheet", index=False)
    with pd.ExcelWriter(DATA / "tabbed.xlsx") as xw:
        pd.DataFrame([["Q1", 45292, "12%"], [None, None, None], ["Q2", 45293, "7%"]]).to_excel(
            xw, sheet_name="Data", index=False, header=False)
        pd.DataFrame([["Lot", "Date Processed", "Rate"]]).to_excel(
            xw, sheet_name="Column Headers (Data)", index=False, header=False)
        pd.DataFrame([[1]]).to_excel(xw, sheet_name="MC Tags", index=False, header=False)
        pd.DataFrame([[1, 2]]).to_excel(xw, sheet_name="Summary", index=False, header=False)
    rows = [["Person", "Hand", "Hand", "Note"], ["KSG", 0, 0, "ok"], [None, "Hand", "Hand", "Note"],
            ["Hand", 1, 0, None], ["Person", "Hand", "Hand", "Note"]]
    pd.DataFrame(rows).to_excel(DATA / "repeat.xlsx", sheet_name="S", index=False, header=False)
    with pd.ExcelWriter(DATA / "edge.xlsx") as xw:
        pd.DataFrame({
            "Int": [1, 2, None, 3, 4],
            "IntBlank": [1, None, None, 3, None],
            "Float": [1.5, 2.0, None, None, 1e16],
            "Bool": [True, False, None, True, False],
            "When": [datetime(2024, 1, 5), datetime(2024, 1, 6, 13, 30), None, datetime(2024, 2, 1), None],
            "Text": ["n/a", "NULL", None, "ok", "#N/A"],
            "Mixed": ["A", 1, None, 2.5, True],
            "NumText": ["1", "2", None, "3", "4"],
            " Spaced ": ["x", "y", None, "z", "w"],
        }).to_excel(xw, sheet_name="Types", index=False)
        pd.DataFrame([[1, 2.5], [3, 4.0], [5, None]]).to_excel(xw, sheet_name="NoHeader", index=False,
                                                                 header=False)
        pd.DataFrame([[None, None], ["Name", "Value"], ["a", 1], ["b", 2]]).to_excel(
            xw, sheet_name="BlankFirst", index=False, header=False)
        pd.DataFrame([["A", "", "A", None, "B"], [1, 2, 3, 4, 5]]).to_excel(
            xw, sheet_name="Dupes", index=False, header=False)
    (DATA / "data.csv").write_text("Lot,Value\nA,1\nB,2\n")
    (DATA / "edge.csv").write_text("Name;Count;Ratio;Flag;Note\nA;1;0.5;True;x\n\nB;;1.25;False;NA\n"
                                   "C;3;;True;\n")
    (DATA / "tabs.tsv").write_text("Lot\tValue\tWhen\nA\t1\t2024-01-05\nB\t2.5\t2024-01-06\n")

    pd.DataFrame({"Lot": ["A1", "A2", "A3"], "Status": ["Pass", "Fail", "Pass"]}).to_excel(
        SMOKE / "lots.xlsx", sheet_name="Lots", index=False)
    pd.DataFrame({"Lot ID": ["a1", "A2", "A3"], "Value": [5e7, 7e7, 1e7]}).to_excel(
        SMOKE / "results.xlsx", sheet_name="Results", index=False)
    smoke = {
        "version": 1, "mode": "join", "join_how": "inner",
        "sources": [{"alias": "L", "path": "tests/golden/smoke/lots.xlsx", "sheet": "Lots", "key": "Lot"},
                    {"alias": "R", "path": "tests/golden/smoke/results.xlsx", "sheet": "Results",
                     "key": "Lot ID"}],
        "where": {"logic": "AND", "items": [{"source": "L", "column": "Status", "op": "=", "value": "Pass"},
                                            {"source": "R", "column": "Value", "op": ">", "value": "2e7"}]},
        "outputs": {"L": ["Status"], "R": ["Value"]},
    }
    (SMOKE / "query.json").write_text(json.dumps(smoke, indent=2) + "\n")


# --- loader ------------------------------------------------------------------------

LOADER_FILES = ["lots.xlsx", "results.xlsx", "em.xlsx", "tabbed.xlsx", "repeat.xlsx", "edge.xlsx", "data.csv",
                "edge.csv", "tabs.tsv"]


def frame_doc(df: pd.DataFrame) -> dict:
    return {"columns": [str(c) for c in df.columns], "index": [int(i) for i in df.index],
            "cells": {str(c): cells(df[c]) for c in df.columns},
            "numeric_columns": loader.numeric_columns(df), "date_columns": loader.date_columns(df)}


def loader_golden():
    out = []
    for name in LOADER_FILES:
        path = DATA / name
        sheets = loader.list_sheets(path)
        doc = {"file": name, "sheets": sheets, "tables": {}}
        targets = list(sheets) + (["Column Headers (Data)", "Nope"] if name == "tabbed.xlsx" else [])
        for sheet in targets:
            try:
                doc["tables"][sheet] = frame_doc(loader.load_table(path, None if loader.is_csv(path) else sheet))
            except Exception as exc:  # noqa: BLE001
                doc["tables"][sheet] = {"error": str(exc)}
        out.append(doc)
    dump("loader.json", out)


# --- conditions --------------------------------------------------------------------

SERIES = {
    "S": pd.Series(["Pass", " fail ", None, "", "50%", "1,000", "2024-03-01", "Retest needed"]),
    "NE": pd.Series([0, "0", "1 Micrococcus luteus", None, "", 3, "n/a"]),
    "LEAD": pd.Series(["1 Micrococcus luteus", "1 Bacillus", "TNTC", 0]),
    "DATESTR": pd.Series(["2024-01-05", "pending", None, "01/06/2024", "Jan 7, 2024", "2024-01-05 13:00"]),
    "FLOAT": pd.Series([1.5, 2.0, None, 5e7, 1e16, -0.25]),
    "INT": pd.Series([0, 1, 2, 10, 100, 45292]),
    "DATES": pd.Series(pd.to_datetime(["2024-01-05", "2024-03-01 13:30", None], format="mixed")),
    "MIDNIGHT": pd.Series(pd.to_datetime(["2024-01-05", "2024-03-01"])),
    "BOOL": pd.Series([True, False, True]),
    "MIXED": pd.Series(["A1", 101, 101.5, None, "  a1 ", "Kim", datetime(2024, 1, 5), "5e7", "1,200 colonies"]),
}
OPS = ["=", "<>", "<", "<=", ">", ">=", "contains", "not contains", "starts with", "ends with", "in", "not in",
       "is blank", "is not blank"]
VALUES = ["", "0", "1", "pass", "fail", "0.4", "1000", "60%", "2024-02-01", "2024-01-05", "1/5/2024",
          "2024-01-05 13:00", "a", "A1", "test", "1.5", "50000000", "101", "kim, a1", "0.5, 1000", "pass, FAIL",
          "1 Micrococcus luteus", "TNTC", "re", "IL", "5e7", " , ", "1,000", "-0.25", "1e+16", "true", "45292",
          "2024", ":"]


def condition_golden():
    series = []
    for name, s in SERIES.items():
        cases = []
        for op in OPS:
            for value in VALUES:
                if op in ("is blank", "is not blank") and value:
                    continue
                c = Condition("*", "c", op, value)
                case = {"op": op, "value": value}
                try:
                    case["expect"] = [bool(b) for b in c.matches(s).fillna(False).tolist()]
                except ValueError as exc:
                    case["error"] = str(exc)
                except Exception as exc:  # noqa: BLE001 - a Python bug; the Rust port need not copy it
                    case["crash"] = f"{type(exc).__name__}: {exc}"
                cases.append(case)
        series.append({"name": name, "cells": cells(s), "cases": cases})

    checks = []
    for op in OPS:
        for value in VALUES:
            try:
                Condition("X", "c", op, value).check_value()
                checks.append({"op": op, "value": value, "ok": True})
            except ValueError as exc:
                checks.append({"op": op, "value": value, "error": str(exc)})
    try:
        Condition("X", "c", "~", "1")
    except ValueError as exc:
        bad_op = str(exc)

    trees = [
        Group("AND", [Condition("A", "x", ">", "5"),
                      Group("OR", [Condition("B", "s", "=", "Pass"), Condition("*", "s", "is blank")])]),
        Group("AND", [Condition("A", "x", ">", "5"), Group("OR", [Condition("B", "s", "=", "Pass")])], negate=True),
        Group(),
        Group("AND", [Group("OR", [Condition("*", "a", "=", "1")], name="Ones", enabled=False),
                      Condition("*", "b", "<>", "z")]),
        Group("AND", [Group("OR", [Condition("A", "x", "=", "1"), Condition("A", "x", "=", "2")],
                            name="Ones or twos"), Condition("A", "y", "is blank")]),
        Group("OR", [Condition("A", "x", "in", "1, 2,it's"), Group("AND", [Condition("*", "y", "is blank")], True),
                     Condition("A", "z", "not in", "a,b", enabled=False)]),
        Group("AND", [Condition("A", "x", "=", "1", color="#FF0000"), Group("OR", [], name="  Empty  ")],
              negate=True, name="Top"),
    ]
    describes = [{"tree": t.to_dict(), "describe": t.describe(),
                  "active": [c.column for c in t.conditions(active=True)],
                  "all": [c.column for c in t.conditions()]} for t in trees]

    # evaluate / cell_hits on in-memory frames
    frames = {"AB": pd.DataFrame({"a": [1, 2, 3, 4], "b": ["x", "y", "x", "y"]})}
    ev_trees = [
        Group("OR", [Condition("*", "a", ">", "3"),
                     Group("AND", [Condition("*", "a", "<", "3"), Condition("*", "b", "=", "y")])]),
        Group("OR", [Condition("*", "a", ">", "3"),
                     Group("AND", [Condition("*", "a", "<", "3"), Condition("*", "b", "=", "y")])], negate=True),
        Group("AND", [Condition("*", "a", "=", "1"), Condition("*", "missing", "=", "1"),
                      Condition("*", "a", "=", "2", enabled=False)]),
        Group("OR", [Condition("*", "missing", "=", "1")]),
        Group(),
        Group("AND", [Group("OR", [Condition("*", "b", "=", "y")], negate=True), Condition("*", "a", ">", "1")]),
    ]
    from wbquery.conditions import cell_hits, evaluate
    df = frames["AB"]

    def column_for(c):
        return [df[c.column]] if c.column in df.columns else []

    evals = []
    for t in ev_trees:
        r = evaluate(t, column_for)
        evals.append({"tree": t.to_dict(), "expect": None if r is None else [bool(b) for b in r.tolist()],
                      "hits": [{"column": col, "rows": [bool(b) for b in m.tolist()]}
                               for _, col, m in cell_hits(t, column_for)]})

    dump("conditions.json", {"frame": {k: cells(v) for k, v in df.items()}, "series": series, "checks": checks,
                             "bad_op": bad_op, "describes": describes, "evaluate": evals})


# --- scalars -----------------------------------------------------------------------

def attempt(fn, *args):
    try:
        return {"ok": fn(*args)}
    except ValueError as exc:
        return {"error": str(exc)}


def scalar_golden():
    texts = ["1 Micrococcus luteus", "3 CFU", "1,200 colonies", "5% coverage", " 4 mold - see note", "1.5", "2",
             "TNTC", "<1", ">300", "approx 2", "2026-01-05", "5/2", "", "n/a", "1e5", "-3", "+4.5 x", "1,2,3 x",
             ".5 x", "12", "  7  ", "50%", "5 %", "inf", "nan", "1_000", "Infinity", "1.", "-.5", "1e400"]
    doc = {
        "cell_number": [{"in": t, "out": tag(loader.cell_number(t))} for t in texts]
        + [{"in": v, "out": tag(loader.cell_number(v))} for v in [3, 2.5, True]],
        "leading_number": [{"in": t, "out": (lambda r: None if r is None else [r[0], r[1]])(loader.leading_number(t))}
                           for t in texts],
        "parse_number": [{"in": t, **(lambda r: {"ok": tag(r["ok"])} if "ok" in r else r)(attempt(parse_number, t))}
                         for t in texts],
        "normalize_key": [{"in": tag(v), "out": normalize_key(v, "BLANK")} for v in
                          [101, 101.0, " 101 ", "A1 ", "a1", pd.Timestamp("2024-01-05 13:00"), None, float("nan"),
                           "  ", True, False, 1.5, "1,000", "1e3", 0.1, 1e16, 1e17, 1.5e-7, "Straße", "inf", "-0",
                           "2024-01-05", 123456789012.0]],
        "normalize_color": [{"in": c, **attempt(normalize_color, c)} for c in
                            ["#FF00AA", "ff00aa", "", "  #abcdef ", "red", "#abc", "#1234567"]],
        "text_color": [{"in": c, "out": text_color(c)} for c in ["#ffd54f", "#0000ff", "#ff0000", "#000000",
                                                                 "#ffffff", "#969696", "#808080"]],
        "sort_columns": [{"in": cols, "out": sort_columns(cols)} for cols in
                         [["P10", "status", "P2", "Lot ID", "lot", "Donor ID"], ["a10b2", "a10b10", "a9", "A9", "b"],
                          ["Chest 1 (2)", "Chest 1", "chest 10", "Chest 2"]]],
        "resolve_column": [{"columns": cols, "name": n, "out": resolve_column(cols, n)} for cols, n in
                           [(["lot", "Lot"], "Lot"), (["Lot ID", "Value"], " lot id"), (["Lot ID"], "Lot"),
                            (["STRASSE"], "straße"), (["a", "A"], "a ")]],
        "column_key": [{"in": n, "out": column_key(n)} for n in ["  Lot ID ", "STRASSE", "Straße", "ǅ"]],
        "split_list": [{"in": v, "out": split_list(v)} for v in ["a, b,,c ", " , ", "", "x"]],
    }
    seq = ["book/Sheet1", "BOOK/Sheet1", "x" * 40, "x" * 40, "Query", "a[b]:c*d?e/f\\g", "'quoted'", "''", "All results",
           "all results"]
    taken = {"query"}
    doc["sheet_name"] = [{"in": n, "out": sheet_name(n, taken)} for n in seq]
    doc["default_alias"] = [{"path": p, "sheet": s, "out": default_alias(p, s)} for p, s in
                            [("/x/book.xlsx", "Sheet1"), ("/x/data.csv", "data"), ("/x/book.xlsx", "book"),
                             ("C:/x/Report.v2.xlsx", "S")]]
    doc["unique_alias"] = [{"base": b, "taken": t, "out": unique_alias(b, t)} for b, t in
                           [("a", ["a", "a (2)"]), ("b", ["a"]), ("c", ["c"])]]
    dump("scalars.json", doc)


# --- queries -----------------------------------------------------------------------

def C(source, column, op, value="", **kw):
    return Condition(source, column, op, value, **kw).to_dict()


def G(logic, items, **kw):
    doc = {"logic": logic, "negate": kw.get("negate", False)}
    if kw.get("name"):
        doc["name"] = kw["name"]
    if kw.get("enabled") is False:
        doc["enabled"] = False
    doc["items"] = items
    return doc


L = {"alias": "L", "path": "data/lots.xlsx", "sheet": "Lots", "key": "Lot"}
R = {"alias": "R", "path": "data/results.xlsx", "sheet": "Results", "key": "Lot ID"}
O = {"alias": "O", "path": "data/lots.xlsx", "sheet": "Owners", "key": "Lot"}
EMS = {"alias": "EM", "path": "data/em.xlsx", "sheet": "Sheet", "key": "Monitor Date"}
EM_COLS = list(EM.columns)


def person(n, suffix, color):
    return G("AND", [C("*", f"Person {n}", "=", "KSG"),
                     G("OR", [C("*", f"Chest 1{suffix}", "<>", "0", color=color),
                              C("*", f"Hand 1{suffix}", "<>", "0", color=color)])])


KSG = G("OR", [person(1, "", "#ff0000"), person(2, " (2)", "#0000ff")])

QUERIES = {
    "independent_or": {"sources": [L, R], "where": G("OR", [C("L", "Status", "=", "pass"),
                                                            C("R", "Value", ">=", "6e7")]),
                       "outputs": {"L": ["Lot", "Status"], "R": ["Lot ID", "Value"]}},
    "any_sheet_three": {"sources": [L, R, O], "where": G("AND", [C("*", "Lot", "=", "A2"),
                                                                 C("R", "Pct", ">", "85%")]),
                        "outputs": {"L": ["Notes"], "O": ["Owner"], "R": ["Pct"]}, "include_provenance": False},
    "no_conditions": {"sources": [L, R], "outputs": {"L": ["Lot"]}},
    "all_no_provenance": {"sources": [L, R], "outputs": {"L": ["Lot"], "R": ["Value"]}, "include_provenance": False},
    "case_insensitive": {"sources": [L, R], "where": G("AND", [C("*", "lot id", "=", "A2"),
                                                               C("L", " status ", "=", "fail")]),
                         "outputs": {"L": ["Lot"], "R": ["Value"]}, "include_provenance": False},
    "disabled_group": {"sources": [L, R], "where": G("AND", [G("AND", [C("L", "Status", ">", "abc"),
                                                                       C("*", "Ghost", "=", "1")], enabled=False)]),
                       "outputs": {"L": ["Lot"]}},
    "untested_warning": {"sources": [L, R, O], "where": G("AND", [C("*", "Status", "=", "pass")]),
                         "outputs": {"L": ["Lot"], "O": ["Lot"], "R": ["Value"]}},
    "validation": {"sources": [L, R], "where": G("AND", [C("L", "Nope", "=", "1"), C("Q", "Lot", "=", "1"),
                                                         C("*", "Ghost", "=", "1"), C("L", "Status", ">", "abc")]),
                   "outputs": {"L": ["Missing"]}},
    "no_outputs": {"sources": [L, R]},
    "bad_mode": {"sources": [L], "mode": "sideways", "outputs": {"L": ["Lot"]}},
    "dates_made": {"sources": [L], "where": G("AND", [C("L", "Made", ">=", "2024-03-01"),
                                                      C("L", "Made", "<", "03/15/2024")]),
                   "outputs": {"L": ["Lot", "Made", "Notes"]}},
    "join_inner": {"sources": [L, R], "mode": "join", "where": G("AND", [C("L", "Status", "=", "Pass"),
                                                                         C("R", "Value", ">", "2e7")]),
                   "outputs": {"L": ["Status"], "R": ["Value"]}},
    "join_inner_all": {"sources": [L, R], "mode": "join", "outputs": {"L": ["Status"], "R": ["Value"]}},
    "join_left": {"sources": [L, R], "mode": "join", "join_how": "left",
                  "outputs": {"L": ["Status", "Made"], "R": ["Value", "Pct"]}},
    "join_outer": {"sources": [L, R], "mode": "join", "join_how": "outer",
                   "outputs": {"L": ["Status", "Made"], "R": ["Value"]}},
    "join_outer_r": {"sources": [L, R], "mode": "join", "join_how": "outer",
                     "where": G("AND", [C("R", "Value", "is not blank")]), "outputs": {"L": ["Status"], "R": ["Value"]}},
    "join_any_sheet": {"sources": [L, R], "mode": "join", "where": G("AND", [C("*", "Status", "=", "fail")]),
                       "outputs": {"L": ["Status"], "R": ["Value"]}},
    "join_case": {"sources": [L, R], "mode": "join", "where": G("AND", [C("*", "STATUS", "=", "fail"),
                                                                        C("R", "value", ">", "6.5e7")]),
                  "outputs": {"L": ["Status"], "R": ["Value"]}},
    "join_no_key": {"sources": [L, {**R, "key": ""}], "mode": "join", "outputs": {"L": ["Status"]}},
    "join_three": {"sources": [L, R, O], "mode": "join", "join_how": "outer",
                   "outputs": {"L": ["Status"], "R": ["Value"], "O": ["Owner"]}, "include_provenance": False},
    "join_left_three": {"sources": [L, O, R], "mode": "join", "join_how": "left",
                        "where": G("OR", [C("*", "Owner", "is blank"), C("R", "Pct", "<", "60%")]),
                        "outputs": {"L": ["Status"], "R": ["Value"], "O": ["Owner"]}},
    "em_colors": {"sources": [EMS], "where": KSG, "outputs": {"EM": EM_COLS}},
    "em_first_color": {"sources": [EMS], "where": G("AND", [C("*", "Hand 1", ">", "1", color="#0000ff"),
                                                            C("*", "Hand 1", "<>", "0", color="#ff0000")]),
                       "outputs": {"EM": EM_COLS}},
    "em_combined": {"sources": [EMS, {**EMS, "alias": "EM2"}], "where": KSG,
                    "outputs": {"EM": EM_COLS, "EM2": ["Hand 1", "Person 2"]}},
    "em_join": {"sources": [EMS], "mode": "join", "where": KSG, "outputs": {"EM": EM_COLS}},
    "em_not": {"sources": [EMS], "where": G("AND", [G("OR", [C("*", "Hand 1", "<>", "0", color="#00ff00")],
                                                      negate=True), C("*", "Person 1", "=", "KSG", color="#ffd54f")]),
               "outputs": {"EM": EM_COLS}},
    "tabbed": {"sources": [{"alias": "D", "path": "data/tabbed.xlsx", "sheet": "Data", "key": ""}],
               "where": G("AND", [C("D", "Date Processed", ">", "2024-01-01"), C("D", "Rate", "<", "10%")]),
               "outputs": {"D": ["Lot", "Date Processed", "Rate"]}},
    "missing_file": {"sources": [{"alias": "X", "path": "data/missing.xlsx", "sheet": "S", "key": ""}],
                     "outputs": {"X": ["a"]}},
    "missing_sheet": {"sources": [{**L, "sheet": "Nope"}], "outputs": {"L": ["Lot"]}},
    "csv_edge": {"sources": [{"alias": "E", "path": "data/edge.csv", "sheet": "edge", "key": ""},
                             {"alias": "D", "path": "data/data.csv", "sheet": "data", "key": ""}],
                 "where": G("OR", [C("*", "Count", ">=", "1"), C("D", "Value", "in", "2, 3")]),
                 "outputs": {"E": ["Name", "Count", "Ratio", "Flag", "Note"], "D": ["Lot", "Value"]}},
    "edge_types": {"sources": [{"alias": "T", "path": "data/edge.xlsx", "sheet": "Types", "key": ""}],
                   "where": G("OR", [C("T", "Mixed", "contains", "1"), C("T", "When", ">", "2024-01-06"),
                                     C("T", "Float", "contains", "e+16")]),
                   "outputs": {"T": ["Int", "IntBlank", "Float", "Bool", "When", "Text", "Mixed", "NumText"]}},
}
HTML_QUERIES = ["independent_or", "join_left", "em_colors", "em_combined", "dates_made", "edge_types", "join_outer"]


def table_doc(df):
    return {"columns": [str(c) for c in df.columns], "cells": {str(c): cells(df[c]) for c in df.columns}}


def normalize(text: str, run_at: str) -> str:
    return (text.replace(run_at, "{RUN_AT}").replace(str(HERE) + "/", "{DIR}/")
            .replace(f"Workbook Query {__version__}", "Workbook Query {VERSION}"))


def query_golden():
    (HERE / "html").mkdir(exist_ok=True)
    out = []
    for name, spec in QUERIES.items():
        doc = {"version": 1, "name": name.replace("_", " ").title(), "description": f"Golden case {name}",
               "mode": "independent", "join_how": "inner", "include_provenance": True, "where": G("AND", []),
               "outputs": {}, **spec}
        absolute = json.loads(json.dumps(doc))
        for s in absolute["sources"]:
            s["path"] = str(HERE / s["path"])
        case = {"name": name, "query": doc}
        try:
            res = run_query(Query.from_dict(absolute))
        except ValueError as exc:
            case["error"] = normalize(str(exc), "\0")
            out.append(case)
            continue
        case["tables"] = [{"name": n, **table_doc(t)} for n, t in res.tables.items()]
        case["hits"] = {n: {str(c): [bool(b) for b in h[c].tolist()] for c in h.columns} for n, h in res.hits.items()}
        case["colors"] = {n: {str(c): list(h[c].tolist()) for c in h.columns} for n, h in res.colors.items()}
        case["warnings"] = res.warnings
        case["sources"] = [{"alias": s["alias"], "rows": s["rows"], "matched": s["matched"], "sha256": s["sha256"]}
                           for s in res.sources]
        run_at = res.run_at.strftime("%Y-%m-%d %H:%M:%S")
        if name in HTML_QUERIES:
            (HERE / "html" / f"{name}.html").write_text(normalize(render_html(res), run_at), encoding="utf-8")
            import openpyxl
            xlsx = HERE / "html" / f"{name}.xlsx"
            write_xlsx(res, xlsx)
            wb = openpyxl.load_workbook(xlsx)
            sheets = []
            for ws in wb.worksheets:
                rows = [[tag(c.value) if not isinstance(c.value, str) else normalize(c.value, run_at) for c in r]
                        for r in ws.iter_rows()]
                fills = {f"{c.coordinate}": c.fill.fgColor.rgb for r in ws.iter_rows() for c in r
                         if c.fill.fill_type == "solid" and r[0].row > 1}
                widths = {k: v.width for k, v in ws.column_dimensions.items()}
                sheets.append({"name": ws.title, "rows": rows, "fills": fills, "widths": widths,
                               "freeze": ws.freeze_panes, "filter": ws.auto_filter.ref})
            xlsx.unlink()
            case["xlsx"] = sheets
        out.append(case)
    dump("queries.json", out)


if __name__ == "__main__":
    write_inputs()
    loader_golden()
    condition_golden()
    scalar_golden()
    query_golden()
    print("golden fixtures written to", HERE)
