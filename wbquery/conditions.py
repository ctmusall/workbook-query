"""Conditions and AND/OR groups that select rows.

A :class:`Condition` tests one column of one source (a workbook sheet), e.g.
``Results › Value > 5e7`` or ``Lots › Status = Pass``. The source may be ``*``
("any sheet"): it then tests every source that has that column. Column names
match exactly when they can, otherwise ignoring case and surrounding spaces
(see :func:`resolve_column`). Conditions are combined in nested :class:`Group` s,
each joined by AND or OR and optionally negated.

Comparison semantics are imr-chart's highlight rules: a value that reads as a
number (``5e7``, ``1,000``, ``50%``) compares numerically, one that reads as a
date compares as a date, and otherwise ``=`` / ``<>`` compare text ignoring case
and surrounding spaces. Blank cells never match (except ``is blank``); other cells
that can't be compared match only ``<>`` / ``not in`` (``1 Micrococcus luteus`` <> 0).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Callable, Union

import pandas as pd

from . import loader

OPERATORS = ("=", "<>", "<", "<=", ">", ">=", "contains", "not contains", "starts with", "ends with",
             "in", "not in", "is blank", "is not blank")
NO_VALUE_OPERATORS = ("is blank", "is not blank")
LOGIC = ("AND", "OR")
ANY_SOURCE = "*"

_COMPARE = {
    "=": lambda a, b: a == b,
    "<>": lambda a, b: a != b,
    "<": lambda a, b: a < b,
    "<=": lambda a, b: a <= b,
    ">": lambda a, b: a > b,
    ">=": lambda a, b: a >= b,
}


def parse_number(text: str | float | None) -> float | None:
    """Parse '2e7', '20,000,000', '50%' (-> 0.5). Blank -> None; not a number -> ValueError."""
    if text is None:
        return None
    if isinstance(text, (int, float)):
        return float(text)
    s = str(text).strip().replace(",", "")
    if not s:
        return None
    try:
        if s.endswith("%"):
            return float(s[:-1]) / 100.0
        return float(s)
    except ValueError:
        raise ValueError(f"Not a number: '{text}'") from None


def column_key(name) -> str:
    """How column names compare when no exact match exists: ignoring case and surrounding spaces."""
    return str(name).strip().casefold()


def _natural_key(name) -> list:
    parts = re.split(r"(\d+)", column_key(name))
    return [(0, int(p), "") if p.isdigit() else (1, 0, p) for p in parts if p]


def sort_columns(columns) -> list[str]:
    """Columns in alphabetical order for pick lists: ignoring case, with numbers in order (P2 before P10)."""
    return sorted((str(c) for c in columns), key=lambda c: (_natural_key(c), c))


def resolve_column(columns, name) -> str | None:
    """The column of ``columns`` that ``name`` refers to: an exact match, else the first whose
    :func:`column_key` matches; None when there's neither."""
    columns = [str(c) for c in columns]
    if str(name) in columns:
        return str(name)
    key = column_key(name)
    return next((c for c in columns if column_key(c) == key), None)


def _text(s: pd.Series, blank: pd.Series) -> pd.Series:
    return s.astype(str).str.strip().str.casefold().where(~blank, "")


def split_list(value: str) -> list[str]:
    """Items of an ``in`` / ``not in`` value: comma-separated, blanks dropped."""
    return [v.strip() for v in str(value).split(",") if v.strip()]


@dataclass
class Condition:
    source: str
    column: str
    op: str
    value: str = ""
    enabled: bool = True

    def __post_init__(self):
        self.value = "" if self.value is None else str(self.value)
        self.source = str(self.source or ANY_SOURCE).strip()
        if self.op not in OPERATORS:
            raise ValueError(f"Condition '{self.column}': unknown operator '{self.op}' "
                             f"(use one of {', '.join(OPERATORS)})")
        if not self.column:
            raise ValueError("Condition: no column chosen")

    def describe(self) -> str:
        where = "Any sheet" if self.source == ANY_SOURCE else self.source
        left = f"{where} › {self.column}"
        if self.op in NO_VALUE_OPERATORS:
            return f"{left} {self.op}"
        if self.op in ("in", "not in"):
            return f"{left} {self.op} ({', '.join(repr(v) for v in split_list(self.value))})"
        return f"{left} {self.op} '{self.value}'"

    def check_value(self):
        """Raise ValueError when the value can't work with the operator."""
        if self.op in NO_VALUE_OPERATORS:
            return
        if self.op in ("in", "not in"):
            if not split_list(self.value):
                raise ValueError(f"{self.describe()}: give one or more values separated by commas")
            return
        if not self.value.strip() and self.op not in ("=", "<>"):
            raise ValueError(f"{self.describe()}: needs a value")
        self.matches(pd.Series([], dtype=object))

    def matches(self, s: pd.Series) -> pd.Series:
        """Boolean per cell of ``s``; blank cells, and others that can't be compared, don't match
        (except that ``<>`` matches non-blank cells that can't be read as its number or date)."""
        blank = loader._is_blank(s)
        if self.op == "is blank":
            return blank
        if self.op == "is not blank":
            return ~blank
        if self.op in _COMPARE:
            return self._compare(self.op, self.value, s, blank)
        if self.op in ("in", "not in"):
            hit = pd.Series(False, index=s.index)
            for item in split_list(self.value):
                hit |= self._compare("=", item, s, blank)
            return hit if self.op == "in" else ~hit & ~blank
        text = _text(s, blank)
        needle = self.value.strip().casefold()
        if self.op == "contains":
            return text.str.contains(needle, regex=False)
        if self.op == "not contains":
            return ~text.str.contains(needle, regex=False)
        if self.op == "starts with":
            return text.str.startswith(needle)
        return text.str.endswith(needle)

    def _compare(self, op: str, value: str, s: pd.Series, blank: pd.Series) -> pd.Series:
        cmp = _COMPARE[op]
        try:
            number = parse_number(value)
        except ValueError:
            number = None
        if number is not None:
            return self._ordered(op, cmp, loader.to_numeric(s), number, blank)
        when = pd.to_datetime(value, errors="coerce", format="mixed") if value.strip() else pd.NaT
        if not pd.isna(when):
            return self._ordered(op, cmp, loader.to_datetime(s), when, blank)
        if op not in ("=", "<>"):
            raise ValueError(f"{self.describe()}: '{op}' needs a number or date, not '{value}'")
        return cmp(_text(s, blank), value.strip().casefold()).astype(bool)

    @staticmethod
    def _ordered(op, cmp, left: pd.Series, value, blank: pd.Series) -> pd.Series:
        """Compare cells read as numbers / dates. A non-blank cell that doesn't read as one can't be
        ordered, so only ``<>`` matches it: ``1 Micrococcus luteus`` <> 0."""
        hit = (cmp(left, value) & left.notna()).astype(bool)
        if op == "<>":
            hit |= left.isna() & ~blank
        return hit

    def to_dict(self) -> dict:
        return {"source": self.source, "column": self.column, "op": self.op,
                "value": self.value, "enabled": self.enabled}


@dataclass
class Group:
    """AND / OR of conditions and sub-groups. A group that isn't ``enabled`` is left out entirely,
    like a disabled condition. ``name`` is a label for people; ``collapsed`` is how the editor shows it."""

    logic: str = "AND"
    items: list[Union[Condition, "Group"]] = field(default_factory=list)
    negate: bool = False
    name: str = ""
    enabled: bool = True
    collapsed: bool = False

    def __post_init__(self):
        self.logic = str(self.logic).upper()
        if self.logic not in LOGIC:
            raise ValueError(f"Group: logic must be AND or OR, not '{self.logic}'")
        self.name = "" if self.name is None else str(self.name).strip()

    def conditions(self, active: bool = False) -> list[Condition]:
        """Every condition in this group and its sub-groups, in order; with ``active``, only those that
        are switched on and not inside a group that's switched off."""
        if active and not self.enabled:
            return []
        out = []
        for item in self.items:
            if isinstance(item, Group):
                out.extend(item.conditions(active))
            elif item.enabled or not active:
                out.append(item)
        return out

    def describe(self, top: bool = True) -> str:
        if not self.enabled:
            return "" if not top else "(all rows)"
        parts = [i.describe(top=False) if isinstance(i, Group) else i.describe()
                 for i in self.items if isinstance(i, Group) or i.enabled]
        parts = [p for p in parts if p]
        if not parts:
            return "" if not top else "(all rows)"
        body = f" {self.logic} ".join(parts)
        if len(parts) > 1 and not (top and not self.negate):
            body = f"({body})"
        body = f"NOT {body}" if self.negate else body
        return f"[{self.name}] {body}" if self.name else body

    def to_dict(self) -> dict:
        doc = {"logic": self.logic, "negate": self.negate}
        if self.name:
            doc["name"] = self.name
        if not self.enabled:
            doc["enabled"] = False
        if self.collapsed:
            doc["collapsed"] = True
        doc["items"] = [i.to_dict() for i in self.items]
        return doc

    @classmethod
    def from_dict(cls, doc: dict) -> "Group":
        if not isinstance(doc, dict):
            raise ValueError("A condition group must be an object")
        items = []
        for item in doc.get("items", []):
            if not isinstance(item, dict):
                raise ValueError("Each condition must be an object")
            if "items" in item:
                items.append(cls.from_dict(item))
            else:
                try:
                    items.append(Condition(**{k: item[k] for k in ("source", "column", "op", "value", "enabled")
                                              if k in item}))
                except TypeError:
                    raise ValueError("Each condition needs a \"column\" and an \"op\"") from None
        return cls(logic=doc.get("logic", "AND"), items=items, negate=bool(doc.get("negate", False)),
                   name=doc.get("name", ""), enabled=bool(doc.get("enabled", True)),
                   collapsed=bool(doc.get("collapsed", False)))


ColumnResolver = Callable[[Condition], list[pd.Series]]


def evaluate(node: Condition | Group, column_for: ColumnResolver) -> pd.Series | None:
    """Rows of the current frame that ``node`` selects, or None when nothing in it applies.

    ``column_for(condition)`` returns the frame's columns the condition tests (several for an
    any-sheet condition after a join; a row matches if any of them does), or ``[]`` when the
    condition doesn't apply to this frame. Conditions that don't apply, and disabled ones (or ones
    in a disabled group), are
    left out, so a group reduces to the parts that do apply; a group with none left is None.
    """
    if isinstance(node, Condition):
        if not node.enabled:
            return None
        cols = column_for(node)
        if not cols:
            return None
        hit = node.matches(cols[0]).fillna(False).astype(bool)
        for s in cols[1:]:
            hit |= node.matches(s).fillna(False).astype(bool)
        return hit
    if not node.enabled:
        return None
    results = [r for r in (evaluate(i, column_for) for i in node.items) if r is not None]
    if not results:
        return None
    out = results[0]
    for r in results[1:]:
        out = (out & r) if node.logic == "AND" else (out | r)
    return ~out if node.negate else out
