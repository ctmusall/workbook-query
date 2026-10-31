"""Shared pieces for the exports: a plain-text summary of a query run."""

from __future__ import annotations

from .query import QueryResult


def summary_rows(result: QueryResult) -> list[tuple[str, str | int]]:
    q = result.query
    rows = [("Mode", "Join on key columns" if q.mode == "join" else "Each sheet filtered on its own")]
    if q.mode == "join":
        rows.append(("Join type", q.join_how))
        rows.append(("Keys", ", ".join(f"{s.alias} › {s.key}" for s in q.sources)))
    rows.append(("Conditions", q.where.describe()))
    rows.append(("Run at", result.run_at.strftime("%Y-%m-%d %H:%M:%S")))
    for name, table in result.tables.items():
        rows.append((f"Rows out: {name}", len(table)))
    return rows


SOURCE_HEADERS = ("Source", "File", "Sheet", "Rows", "Matched", "SHA-256")


def source_rows(result: QueryResult) -> list[tuple]:
    return [(s["alias"], s["path"], s["sheet"], s["rows"], "" if s["matched"] is None else s["matched"],
             s["sha256"] or "") for s in result.sources]
