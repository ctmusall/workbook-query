"""Load tables from Excel workbooks or CSV files (adapted from imr-chart's loader)."""

from __future__ import annotations

import re
from pathlib import Path

import pandas as pd

HEADER_TAB_PATTERN = re.compile(r"^Column Headers \((?P<sheet>.+)\)$")
HIDDEN_SHEETS = {"MC Tags"}
PREFERRED_SHEET = "Summary"
PREFERRED_DATE_COLUMN = "Date Processed"
EXCEL_EPOCH = pd.Timestamp("1899-12-30")
CSV_SUFFIXES = {".csv", ".txt", ".tsv"}
SOURCE_ROW = "Source row"


def is_csv(path: str | Path) -> bool:
    return Path(path).suffix.lower() in CSV_SUFFIXES


def list_sheets(path: str | Path) -> list[str]:
    """Return the data-bearing sheet names, preferred sheet first."""
    if is_csv(path):
        return [Path(path).stem]
    names = pd.ExcelFile(path).sheet_names
    data = [n for n in names if not HEADER_TAB_PATTERN.match(n) and n not in HIDDEN_SHEETS]
    if PREFERRED_SHEET in data:
        data.remove(PREFERRED_SHEET)
        data.insert(0, PREFERRED_SHEET)
    return data


def _header_tab_for(sheet: str, all_sheets: list[str]) -> str | None:
    for name in all_sheets:
        m = HEADER_TAB_PATTERN.match(name)
        if m and m.group("sheet") == sheet:
            return name
    return None


def _row_looks_like_header(row: pd.Series) -> bool:
    values = [v for v in row.tolist() if not pd.isna(v)]
    if not values:
        return False
    strings = sum(isinstance(v, str) and pd.isna(pd.to_numeric(v, errors="coerce")) for v in values)
    return strings / len(values) >= 0.8


def _dedupe(names: list) -> list[str]:
    seen: dict[str, int] = {}
    out = []
    for i, n in enumerate(names):
        name = str(n).strip() if not pd.isna(n) and str(n).strip() else f"Column {i + 1}"
        if name in seen:
            seen[name] += 1
            name = f"{name} ({seen[name]})"
        else:
            seen[name] = 1
        out.append(name)
    return out


_DEDUPE_SUFFIX = re.compile(r" \(\d+\)$")


def _drop_repeated_headers(df: pd.DataFrame) -> pd.DataFrame:
    """Drop rows that repeat the header (a header copied further down the sheet): at least
    three non-blank cells, 80% of them equal to their column's name (``Chest 1`` under ``Chest 1 (2)``
    counts too)."""
    if df.empty:
        return df
    names = [{str(c).strip().casefold(), _DEDUPE_SUFFIX.sub("", str(c)).strip().casefold()} for c in df.columns]

    def repeats(row) -> bool:
        cells = [(str(v).strip().casefold(), n) for v, n in zip(row, names) if not pd.isna(v) and str(v).strip()]
        return len(cells) >= 3 and sum(v in n for v, n in cells) >= 0.8 * len(cells)

    keep = [not repeats(row) for row in df.itertuples(index=False, name=None)]
    return df[keep]


def load_table(path: str | Path, sheet: str | None = None) -> pd.DataFrame:
    """Load a sheet (or CSV) into a DataFrame with resolved column names.

    Header resolution: a sibling ``Column Headers (<sheet>)`` tab wins; otherwise
    the first row is used when it looks like a header; otherwise generic names.

    The index holds each row's line number in the source file (the Excel row
    number, or the CSV line number), so every matched row can be traced back.
    """
    path = Path(path)
    if not path.exists():
        raise FileNotFoundError(f"File not found: {path}")
    if is_csv(path):
        sep = "\t" if path.suffix.lower() == ".tsv" else None
        df = pd.read_csv(path, sep=sep, engine="python", skip_blank_lines=False)
        df.columns = _dedupe(list(df.columns))
        df.index = pd.RangeIndex(2, len(df) + 2, name=SOURCE_ROW)  # line 1 is the header
        return _drop_repeated_headers(df.dropna(how="all"))

    xl = pd.ExcelFile(path)
    if sheet is None:
        sheet = list_sheets(path)[0]
    if sheet not in xl.sheet_names:
        raise ValueError(f"Sheet '{sheet}' not found. Available: {', '.join(xl.sheet_names)}")

    raw = pd.read_excel(xl, sheet_name=sheet, header=None)
    raw.index = pd.RangeIndex(1, len(raw) + 1, name=SOURCE_ROW)  # blank rows are kept, so this is the Excel row
    header_tab = _header_tab_for(sheet, xl.sheet_names)
    if header_tab is not None:
        names = pd.read_excel(xl, sheet_name=header_tab, header=None).iloc[0].tolist()
        if len(names) < raw.shape[1]:
            names += [None] * (raw.shape[1] - len(names))
        raw = raw.reindex(columns=range(len(names)))
        raw.columns = _dedupe(names)
        # Drop a duplicated header row if the data tab also has one.
        if len(raw) and _row_looks_like_header(raw.iloc[0]):
            raw = raw.iloc[1:]
    elif len(raw) and _row_looks_like_header(raw.iloc[0]):
        raw.columns = _dedupe(raw.iloc[0].tolist())
        raw = raw.iloc[1:]
    else:
        raw.columns = _dedupe([None] * raw.shape[1])
    raw = _drop_repeated_headers(raw.dropna(how="all"))
    return raw.infer_objects()


def to_numeric(series: pd.Series) -> pd.Series:
    """Coerce to float; strings like '50%' become 0.5, junk becomes NaN."""
    if pd.api.types.is_numeric_dtype(series):
        return series.astype(float)

    def conv(v):
        if isinstance(v, str):
            s = v.strip().replace(",", "")
            if s.endswith("%"):
                try:
                    return float(s[:-1]) / 100.0
                except ValueError:
                    return float("nan")
            return s
        return v

    return pd.to_numeric(series.map(conv), errors="coerce").astype(float)


def numeric_columns(df: pd.DataFrame, threshold: float = 0.8) -> list[str]:
    """Columns where at least ``threshold`` of non-blank values are numeric."""
    out = []
    for col in df.columns:
        s = df[col]
        if pd.api.types.is_datetime64_any_dtype(s):
            continue
        nonblank = s[~s.isna() & (s.astype(str).str.strip() != "")]
        if nonblank.empty:
            continue
        frac = to_numeric(nonblank).notna().mean()
        if frac >= threshold:
            out.append(col)
    return out


def to_datetime(series: pd.Series) -> pd.Series:
    """Convert Excel serial numbers or date strings/objects to timestamps."""
    if pd.api.types.is_datetime64_any_dtype(series):
        return series
    numeric = pd.to_numeric(series, errors="coerce")
    serial_mask = numeric.notna() & numeric.between(1, 2_958_465)
    out = pd.Series(pd.NaT, index=series.index, dtype="datetime64[ns]")
    if serial_mask.any():
        out[serial_mask] = EXCEL_EPOCH + pd.to_timedelta(numeric[serial_mask], unit="D")
    rest = ~serial_mask & series.notna()
    if rest.any():
        out[rest] = pd.to_datetime(series[rest].astype(str), errors="coerce", format="mixed")
    return out


def date_columns(df: pd.DataFrame) -> list[str]:
    """Columns that look like dates (by name or dtype)."""
    out = []
    for col in df.columns:
        s = df[col]
        if pd.api.types.is_datetime64_any_dtype(s) or "date" in str(col).lower():
            out.append(col)
    if PREFERRED_DATE_COLUMN in out:
        out.remove(PREFERRED_DATE_COLUMN)
        out.insert(0, PREFERRED_DATE_COLUMN)
    return out


def _is_blank(series: pd.Series) -> pd.Series:
    return series.isna() | (series.astype(str).str.strip() == "")
