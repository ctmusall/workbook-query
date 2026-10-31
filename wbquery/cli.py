"""Command line: run a saved query (JSON) and write the results."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

import pandas as pd

from . import __version__, loader
from .query import load_query, run_query

FORMATS = ("xlsx", "html")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="wbquery-cli",
        description="Filter rows across Excel workbooks and sheets with a saved query, and export the matches.",
    )
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    p.add_argument("-q", "--query", metavar="FILE.json", help="query saved from the app (or written by hand)")
    p.add_argument("--source", action="append", default=[], metavar="ALIAS=PATH",
                   help="read source ALIAS from PATH instead of the path saved in the query (repeatable)")
    p.add_argument("-o", "--out", default="query_results",
                   help="output file name without extension, or an existing directory (default: query_results)")
    p.add_argument("--formats", default="xlsx,html", help="comma-separated: xlsx, html (default: both)")
    p.add_argument("--print", action="store_true", dest="print_", help="also print the result tables")
    p.add_argument("--list", nargs="+", metavar="FILE", help="list each file's sheets and columns, then exit")
    return p


def _list(files: list[str]) -> int:
    for f in files:
        print(f)
        for sheet in loader.list_sheets(f):
            cols = loader.load_table(f, sheet).columns
            print(f"  [{sheet}] " + ", ".join(map(str, cols)))
    return 0


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.list:
            return _list(args.list)
        if not args.query:
            print("error: give --query FILE.json (or --list FILE to see sheets and columns)", file=sys.stderr)
            return 2
        formats = [f.strip().lower() for f in args.formats.split(",") if f.strip()]
        bad = [f for f in formats if f not in FORMATS]
        if bad or not formats:
            print(f"error: --formats takes {', '.join(FORMATS)}", file=sys.stderr)
            return 2
        query = load_query(args.query)
        for item in args.source:
            alias, sep, path = item.partition("=")
            if not sep:
                print(f"error: --source needs ALIAS=PATH, not '{item}'", file=sys.stderr)
                return 2
            query.source(alias.strip()).path = path.strip()
        result = run_query(query)
    except (ValueError, KeyError, FileNotFoundError) as exc:
        msg = exc.args[0] if isinstance(exc, KeyError) and exc.args else exc
        print(f"error: {msg}", file=sys.stderr)
        return 1

    for w in result.warnings:
        print(f"warning: {w}", file=sys.stderr)
    out = Path(args.out)
    if out.is_dir():
        out = out / "query_results"
    out.parent.mkdir(parents=True, exist_ok=True)
    written = []
    if "xlsx" in formats:
        from .export_xlsx import write_xlsx

        written.append(write_xlsx(result, out.with_suffix(".xlsx")))
    if "html" in formats:
        from .export_html import write_html

        written.append(write_html(result, out.with_suffix(".html")))
    for name, table in result.tables.items():
        print(f"{name}: {len(table)} row(s)")
        if args.print_:
            with pd.option_context("display.max_rows", None, "display.max_columns", None, "display.width", 200):
                print(table.to_string(index=False))
    for p in written:
        print(f"wrote {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
