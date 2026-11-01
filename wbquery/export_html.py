"""Write query results to a single self-contained HTML report."""

from __future__ import annotations

import html
from pathlib import Path

import pandas as pd

from . import __version__
from .conditions import text_color
from .query import QueryResult
from .report import SOURCE_HEADERS, source_rows, summary_rows

CSS = """
:root { --bg:#fff; --fg:#1d232b; --muted:#5b6672; --line:#d6dce3; --head:#eef2f6; --stripe:#f7f9fb; --warn:#8a5a00; }
@media (prefers-color-scheme: dark) {
  :root { --bg:#16191d; --fg:#e4e8ec; --muted:#9aa5b1; --line:#323940; --head:#222830; --stripe:#1b1f24; --warn:#e0b050; }
}
body { background:var(--bg); color:var(--fg); font:14px/1.45 system-ui, -apple-system, "Segoe UI", sans-serif;
       margin:0 auto; padding:24px 16px; max-width:1400px; }
h1 { font-size:22px; margin:0 0 4px; } h2 { font-size:17px; margin:28px 0 8px; }
.muted { color:var(--muted); } .warn { color:var(--warn); }
.scroll { overflow-x:auto; border:1px solid var(--line); border-radius:6px; }
table { border-collapse:collapse; width:100%; font-variant-numeric:tabular-nums; }
th, td { padding:5px 10px; border-bottom:1px solid var(--line); text-align:left; white-space:nowrap; }
th { background:var(--head); position:sticky; top:0; }
tbody tr:nth-child(even) td { background:var(--stripe); }
table.kv th { width:12em; white-space:normal; } table.kv td { white-space:pre-line; }
code { font-size:12px; }
"""


def _table(df: pd.DataFrame, colors: pd.DataFrame | None = None) -> str:
    def cell(v):
        if v is None or (not isinstance(v, str) and pd.isna(v)):
            return ""
        if isinstance(v, pd.Timestamp):
            return v.strftime("%Y-%m-%d") if v == v.normalize() else str(v)
        return str(v)

    head = "".join(f"<th>{html.escape(str(c))}</th>" for c in df.columns)

    def td(v, color):
        style = f' style="background:{color};color:{text_color(color)}"' if color else ""
        return f"<td{style}>{html.escape(cell(v))}</td>"

    rows = df.itertuples(index=False, name=None)
    if colors is None:
        colors = pd.DataFrame("", index=df.index, columns=df.columns)
    marks = colors.itertuples(index=False, name=None)
    body = "\n".join("<tr>" + "".join(td(v, c) for v, c in zip(row, mark)) + "</tr>"
                     for row, mark in zip(rows, marks))
    return f'<div class="scroll"><table><thead><tr>{head}</tr></thead><tbody>\n{body}\n</tbody></table></div>'


def render_html(result: QueryResult, title: str = "") -> str:
    title = title or result.query.name.strip() or "Workbook Query results"
    kv = "".join(f"<tr><th>{html.escape(k)}</th><td>{html.escape(str(v))}</td></tr>" for k, v in summary_rows(result))
    warn = "".join(f'<p class="warn">⚠ {html.escape(w)}</p>' for w in result.warnings)
    src_head = "".join(f"<th>{h}</th>" for h in SOURCE_HEADERS)
    src_body = "".join("<tr>" + "".join(f"<td>{'<code>' if i == 5 else ''}{html.escape(str(v))}"
                                        f"{'</code>' if i == 5 else ''}</td>" for i, v in enumerate(r)) + "</tr>"
                       for r in source_rows(result))
    sections = "".join(
        f"<h2>{html.escape(name)} <span class=\"muted\">({len(df)} row{'s' if len(df) != 1 else ''})</span></h2>"
        + _table(df, result.colors.get(name)) for name, df in result.tables.items())
    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{html.escape(title)}</title><style>{CSS}</style></head>
<body>
<h1>{html.escape(title)}</h1>
<p class="muted">Workbook Query {__version__}</p>
<div class="scroll"><table class="kv"><tbody>{kv}</tbody></table></div>
{warn}
{sections}
<h2>Sources</h2>
<div class="scroll"><table><thead><tr>{src_head}</tr></thead><tbody>{src_body}</tbody></table></div>
</body></html>
"""


def write_html(result: QueryResult, path: str | Path) -> Path:
    path = Path(path)
    if path.suffix.lower() not in (".html", ".htm"):
        path = path.with_suffix(".html")
    path.write_text(render_html(result), encoding="utf-8")
    return path
