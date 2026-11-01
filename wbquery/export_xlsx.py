"""Write query results to an Excel workbook: one sheet per result table plus a Query sheet."""

from __future__ import annotations

import re
from pathlib import Path

import pandas as pd
from openpyxl.styles import Font, PatternFill
from openpyxl.utils import get_column_letter

from .conditions import text_color
from .query import QueryResult
from .report import SOURCE_HEADERS, source_rows, summary_rows

HEADER_FILL = PatternFill("solid", fgColor="DDE6F0")
BOLD = Font(bold=True)
_BAD = re.compile(r"[\[\]:*?/\\]")


def sheet_name(name: str, taken: set[str]) -> str:
    base = _BAD.sub("_", name).strip("'") or "Sheet"
    base = base[:31]
    out, n = base, 2
    while out.casefold() in taken:
        suffix = f" ({n})"
        out = base[: 31 - len(suffix)] + suffix
        n += 1
    taken.add(out.casefold())
    return out


def _autosize(ws, max_width: int = 60):
    for i, col in enumerate(ws.iter_cols(min_row=1, max_row=min(ws.max_row, 500)), start=1):
        width = max((len(str(c.value)) for c in col if c.value is not None), default=8)
        ws.column_dimensions[get_column_letter(i)].width = min(max_width, max(8, width + 2))


def _style_table(ws):
    for c in ws[1]:
        c.font = BOLD
        c.fill = HEADER_FILL
    ws.freeze_panes = "A2"
    if ws.max_row > 1 or ws.max_column > 1:
        ws.auto_filter.ref = ws.dimensions
    _autosize(ws)


def _highlight(ws, colors: pd.DataFrame | None):
    """Fill each cell a condition colored (row 1 is the header)."""
    if colors is None:
        return
    fills: dict[str, tuple[PatternFill, Font]] = {}
    for j, col in enumerate(colors.columns, start=1):
        for i in (colors[col] != "").to_numpy().nonzero()[0]:
            color = colors[col].iat[i]
            if color not in fills:
                fills[color] = (PatternFill("solid", fgColor=color[1:].upper()),
                                Font(color=text_color(color)[1:].upper()))
            cell = ws.cell(int(i) + 2, j)
            cell.fill, cell.font = fills[color]


def write_xlsx(result: QueryResult, path: str | Path) -> Path:
    path = Path(path)
    if path.suffix.lower() != ".xlsx":
        path = path.with_suffix(".xlsx")
    taken = {"query"}
    with pd.ExcelWriter(path, engine="openpyxl") as xw:
        names = {}
        for name, table in result.tables.items():
            names[name] = sheet_name(name, taken)
            table.to_excel(xw, sheet_name=names[name], index=False)
        wb = xw.book
        for name in result.tables:
            _style_table(wb[names[name]])
            _highlight(wb[names[name]], result.colors.get(name))

        ws = wb.create_sheet("Query", 0)
        for label, value in summary_rows(result):
            ws.append([label, value])
            ws.cell(ws.max_row, 1).font = BOLD
        if result.warnings:
            ws.append([])
            for w in result.warnings:
                ws.append(["Warning", w])
                ws.cell(ws.max_row, 1).font = BOLD
        ws.append([])
        ws.append(list(SOURCE_HEADERS))
        for c in ws[ws.max_row]:
            c.font = BOLD
            c.fill = HEADER_FILL
        for row in source_rows(result):
            ws.append(list(row))
        _autosize(ws, max_width=80)
    return path
