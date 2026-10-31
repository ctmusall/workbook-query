"""Conditions and AND/OR groups that select rows.

A :class:`Condition` tests one column of one source (a workbook sheet), e.g.
``Results › Value > 5e7`` or ``Lots › Status = Pass``. The source may be ``*``
("any sheet"): it then tests every source that has that column. Conditions are
combined in nested :class:`Group` s, each joined by AND or OR and optionally
negated.

Comparison semantics are imr-chart's highlight rules: a value that reads as a
number (``5e7``, ``1,000``, ``50%``) compares numerically, one that reads as a
date compares as a date, and otherwise ``=`` / ``<>`` compare text ignoring case
and surrounding spaces. Cells that can't be compared never match.
"""

from __future__ import annotations

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
        """Boolean per cell of ``s``; cells that can't be compared never match."""
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
            left = loader.to_numeric(s)
            return (cmp(left, number) & left.notna()).astype(bool)
        when = pd.to_datetime(value, errors="coerce", format="mixed") if value.strip() else pd.NaT
        if not pd.isna(when):
            left = loader.to_datetime(s)
            return (cmp(left, when) & left.notna()).astype(bool)
        if op not in ("=", "<>"):
            raise ValueError(f"{self.describe()}: '{op}' needs a number or date, not '{value}'")
        return cmp(_text(s, blank), value.strip().casefold()).astype(bool)

    def to_dict(self) -> dict:
        return {"source": self.source, "column": self.column, "op": self.op,
                "value": self.value, "enabled": self.enabled}


@dataclass
class Group:
    logic: str = "AND"
    items: list[Union[Condition, "Group"]] = field(default_factory=list)
    negate: bool = False

    def __post_init__(self):
        self.logic = str(self.logic).upper()
        if self.logic not in LOGIC:
            raise ValueError(f"Group: logic must be AND or OR, not '{self.logic}'")

    def conditions(self) -> list[Condition]:
        """Every condition in this group and its sub-groups, in order."""
        out = []
        for item in self.items:
            out.extend(item.conditions() if isinstance(item, Group) else [item])
        return out

    def describe(self, top: bool = True) -> str:
        parts = [i.describe(top=False) if isinstance(i, Group) else i.describe()
                 for i in self.items if isinstance(i, Group) or i.enabled]
        parts = [p for p in parts if p]
        if not parts:
            return "" if not top else "(all rows)"
        body = f" {self.logic} ".join(parts)
        if len(parts) > 1 and not (top and not self.negate):
            body = f"({body})"
        return f"NOT {body}" if self.negate else body

    def to_dict(self) -> dict:
        return {"logic": self.logic, "negate": self.negate,
                "items": [i.to_dict() for i in self.items]}

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
        return cls(logic=doc.get("logic", "AND"), items=items, negate=bool(doc.get("negate", False)))


ColumnResolver = Callable[[Condition], list[pd.Series]]


def evaluate(node: Condition | Group, column_for: ColumnResolver) -> pd.Series | None:
    """Rows of the current frame that ``node`` selects, or None when nothing in it applies.

    ``column_for(condition)`` returns the frame's columns the condition tests (several for an
    any-sheet condition after a join; a row matches if any of them does), or ``[]`` when the
    condition doesn't apply to this frame. Conditions that don't apply, and disabled ones, are
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
    results = [r for r in (evaluate(i, column_for) for i in node.items) if r is not None]
    if not results:
        return None
    out = results[0]
    for r in results[1:]:
        out = (out & r) if node.logic == "AND" else (out | r)
    return ~out if node.negate else out
