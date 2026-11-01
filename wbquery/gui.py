"""Desktop app: pick workbook sheets, build an AND/OR query, choose output columns, run and export."""

from __future__ import annotations

import copy
import html
import sys
from dataclasses import dataclass
from pathlib import Path

import pandas as pd
from PySide6.QtCore import QAbstractTableModel, QModelIndex, QSortFilterProxyModel, QStringListModel, Qt, Signal
from PySide6.QtGui import QColor, QIcon, QKeySequence, QPixmap, QShortcut
from PySide6.QtWidgets import (
    QAbstractItemView, QApplication, QButtonGroup, QCheckBox, QColorDialog, QComboBox, QCompleter, QDialog,
    QDialogButtonBox, QFileDialog,
    QGroupBox, QHBoxLayout, QHeaderView, QLabel, QLineEdit, QListWidget, QListWidgetItem, QMainWindow,
    QMenu, QMessageBox, QPushButton, QRadioButton, QScrollArea, QSplitter, QTableView, QTableWidget, QTableWidgetItem,
    QTabWidget, QToolBox, QToolButton, QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget,
)

from . import __version__, loader
from .conditions import ANY_SOURCE, LOGIC, NO_VALUE_OPERATORS, OPERATORS, Condition, Group, column_key, \
    parse_number, resolve_column, sort_columns, text_color
from .query import JOIN_HOW, Query, QueryResult, Source, default_alias, is_provenance, load_query, run_query, \
    save_query, unique_alias

APP = "Workbook Query"
ANY_LABEL = "Any sheet"
WORKBOOK_FILTER = "Workbooks (*.xlsx *.xlsm *.xls *.csv *.tsv *.txt);;All files (*)"
MAX_SUGGESTIONS = 1000
ICON = Path(__file__).resolve().parent / "assets" / "icon.png"


def _esc(text) -> str:
    """``text`` for a rich-text label: ``<`` ``>`` ``&`` escaped so condition text such as ``<> '0'`` shows."""
    return html.escape(str(text), quote=False)


def _is_blank(v) -> bool:
    return v is None or (not isinstance(v, str) and pd.isna(v))


def _display(v) -> str:
    """A cell as the result tables show it."""
    if _is_blank(v):
        return ""
    if isinstance(v, pd.Timestamp):
        return v.strftime("%Y-%m-%d") if v == v.normalize() else str(v)
    return str(v)


def _value_text(v) -> str:
    """A cell as a condition value: like :func:`_display`, but 5.0 reads as 5."""
    if isinstance(v, float) and v.is_integer():
        return str(int(v))
    return _display(v).strip()


def _sort_key(text: str):
    try:
        number = parse_number(text)
    except ValueError:
        number = None
    return (number is None, number or 0.0, text.casefold(), text)


def _search_completer(parent, items=None) -> QCompleter:
    """Case-insensitive, match-anywhere completion."""
    comp = QCompleter(parent)
    if items is not None:
        comp.setModel(QStringListModel(list(items), comp))
    comp.setCaseSensitivity(Qt.CaseInsensitive)
    comp.setFilterMode(Qt.MatchContains)
    comp.setCompletionMode(QCompleter.PopupCompletion)
    return comp


@dataclass
class Entry:
    source: Source
    frame: pd.DataFrame

    @property
    def columns(self) -> list[str]:
        return [str(c) for c in self.frame.columns]


class FrameModel(QAbstractTableModel):
    """Read-only view of a DataFrame."""

    def __init__(self, df: pd.DataFrame, parent=None, colors: pd.DataFrame | None = None):
        super().__init__(parent)
        self.df = df
        self.colors = colors  # same shape as df: "#rrggbb" highlight per cell, "" for none

    def rowCount(self, parent=QModelIndex()):
        return 0 if parent.isValid() else len(self.df)

    def columnCount(self, parent=QModelIndex()):
        return 0 if parent.isValid() else self.df.shape[1]

    def data(self, index, role=Qt.DisplayRole):
        if not index.isValid():
            return None
        v = self.df.iat[index.row(), index.column()]
        if role == Qt.DisplayRole:
            return _display(v)
        if role == Qt.UserRole:  # sort key
            if _is_blank(v):
                return None
            return v if isinstance(v, (int, float)) else str(v)
        if role in (Qt.BackgroundRole, Qt.ForegroundRole) and self.colors is not None:
            color = self.colors.iat[index.row(), index.column()]
            if color:
                return QColor(color if role == Qt.BackgroundRole else text_color(color))
        return None

    def headerData(self, section, orientation, role=Qt.DisplayRole):
        if role != Qt.DisplayRole:
            return None
        return str(self.df.columns[section]) if orientation == Qt.Horizontal else str(section + 1)


class SortProxy(QSortFilterProxyModel):
    def lessThan(self, left, right):
        a, b = left.data(Qt.UserRole), right.data(Qt.UserRole)
        if a is None or b is None:
            return a is None and b is not None
        try:
            return a < b
        except TypeError:
            return str(a) < str(b)


class SheetPicker(QDialog):
    def __init__(self, path: str, sheets: list[str], parent=None):
        super().__init__(parent)
        self.setWindowTitle(f"Sheets in {Path(path).name}")
        lay = QVBoxLayout(self)
        lay.addWidget(QLabel("Choose the sheets to query:"))
        self.list = QListWidget()
        for i, s in enumerate(sheets):
            item = QListWidgetItem(s)
            item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
            item.setCheckState(Qt.Checked if i == 0 else Qt.Unchecked)
            self.list.addItem(item)
        lay.addWidget(self.list)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        lay.addWidget(buttons)

    def chosen(self) -> list[str]:
        return [self.list.item(i).text() for i in range(self.list.count())
                if self.list.item(i).checkState() == Qt.Checked]


class ColumnPicker(QDialog):
    """Tick several columns at once; nothing happens until OK.

    ``rows`` are (column, sheets that have it, initial state). A row starting partly checked
    (ticked in some sheets only) keeps a third, "leave as is" state."""

    def __init__(self, title: str, prompt: str, rows: list[tuple[str, list[str], Qt.CheckState]], parent=None,
                 show_sheets: bool = True):
        super().__init__(parent)
        self.setWindowTitle(title)
        self.resize(420, 480)
        lay = QVBoxLayout(self)
        label = QLabel(prompt)
        label.setWordWrap(True)
        lay.addWidget(label)
        self.search = QLineEdit()
        self.search.setPlaceholderText("Search columns")
        self.search.setClearButtonEnabled(True)
        self.search.textChanged.connect(self._filter)
        lay.addWidget(self.search)
        self.list = QListWidget()
        for col, sheets, state in rows:
            n = len(sheets)
            item = QListWidgetItem(f"{col}   ({n} sheet{'s' if n != 1 else ''})" if show_sheets else col)
            item.setData(Qt.UserRole, col)
            if show_sheets:
                item.setToolTip("In: " + ", ".join(sheets))
            flags = item.flags() | Qt.ItemIsUserCheckable
            if state == Qt.PartiallyChecked:
                flags |= Qt.ItemIsUserTristate
                item.setToolTip(item.toolTip() + "\nHalf-ticked: ticked in some of these sheets; left as is.")
            item.setFlags(flags)
            item.setCheckState(state)
            self.list.addItem(item)
        lay.addWidget(self.list)
        row = QHBoxLayout()
        all_btn, none_btn = QPushButton("All"), QPushButton("None")
        all_btn.setToolTip("Tick every column shown")
        none_btn.setToolTip("Untick every column shown")
        all_btn.clicked.connect(lambda: self._set_shown(Qt.Checked))
        none_btn.clicked.connect(lambda: self._set_shown(Qt.Unchecked))
        row.addWidget(all_btn)
        row.addWidget(none_btn)
        row.addStretch()
        lay.addLayout(row)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        lay.addWidget(buttons)
        self._lay = lay
        self.search.setFocus()

    def _items(self):
        return [self.list.item(i) for i in range(self.list.count())]

    def _filter(self, text: str):
        needle = text.strip().casefold()
        for item in self._items():
            item.setHidden(bool(needle) and needle not in item.data(Qt.UserRole).casefold())

    def _set_shown(self, state):
        for item in self._items():
            if not item.isHidden():
                item.setCheckState(state)

    def set_state(self, column: str, state):
        for item in self._items():
            if item.data(Qt.UserRole) == column:
                item.setCheckState(state)

    def states(self) -> dict[str, Qt.CheckState]:
        return {item.data(Qt.UserRole): item.checkState() for item in self._items()}

    def checked(self) -> list[str]:
        return [c for c, st in self.states().items() if st == Qt.Checked]


PLACE_SELECTED = "the selected group"
PLACE_OR = "a new OR group (any column matches)"
PLACE_AND = "a new AND group (every column matches)"
PLACEMENTS = (PLACE_SELECTED, PLACE_OR, PLACE_AND)


class SharedColumnPicker(ColumnPicker):
    """:class:`ColumnPicker` plus the operator and value every picked column's condition gets, and
    where the conditions go. ``values_for(columns)`` gives value suggestions for the ticked columns."""

    def __init__(self, rows, values_for, parent=None):
        super().__init__("Add for column", "Tick the columns to filter on. Each gets one condition that tests "
                         "every sheet with that column, using the operator and value below.", rows, parent)
        self._values_for = values_for
        box = QGroupBox("Condition for every ticked column")
        grid = QVBoxLayout(box)
        row = QHBoxLayout()
        self.op_box = _combo(OPERATORS, "=")
        self.value_edit = QLineEdit()
        self.value_edit.setPlaceholderText("value (lists: a, b, c) — can be left blank to fill in later")
        self.color_btn = ColorButton()
        row.addWidget(self.op_box)
        row.addWidget(self.value_edit, 1)
        row.addWidget(QLabel("Highlight"))
        row.addWidget(self.color_btn)
        grid.addLayout(row)
        place = QHBoxLayout()
        place.addWidget(QLabel("Put them in"))
        self.place_box = _combo(PLACEMENTS, PLACE_SELECTED)
        self.place_box.setToolTip("A new OR group matches rows where any of the columns matches; "
                                  "a new AND group, rows where all of them do.")
        place.addWidget(self.place_box, 1)
        grid.addLayout(place)
        self.error_label = QLabel()
        self.error_label.setWordWrap(True)
        self.error_label.setStyleSheet("color: #c0392b")
        self.error_label.hide()
        grid.addWidget(self.error_label)
        self._lay.insertWidget(self._lay.count() - 1, box)
        self.op_box.currentTextChanged.connect(self._op_changed)
        self.list.itemChanged.connect(lambda *_: self._update_values())
        self.value_edit.textChanged.connect(lambda *_: self.error_label.hide())
        self._update_values()

    def _op_changed(self, op: str):
        self.value_edit.setEnabled(op not in NO_VALUE_OPERATORS)
        self.error_label.hide()

    def _update_values(self):
        values = self._values_for(self.checked()) if self.checked() else []
        self.value_edit.setCompleter(_search_completer(self.value_edit, values))

    def expression(self) -> tuple[str, str]:
        op = self.op_box.currentText()
        return op, "" if op in NO_VALUE_OPERATORS else self.value_edit.text()

    def color(self) -> str:
        return self.color_btn.color()

    def group_logic(self) -> str | None:
        """None for the selected group, else the new group's AND / OR."""
        return {PLACE_OR: "OR", PLACE_AND: "AND"}.get(self.place_box.currentText())

    def problem(self) -> str:
        """Why OK can't go ahead, or ''. A blank value is allowed (filled in later in the tree)."""
        if not self.checked():
            return "Tick at least one column."
        op, value = self.expression()
        if not value.strip() and op not in NO_VALUE_OPERATORS:
            return ""
        try:
            Condition(ANY_SOURCE, self.checked()[0], op, value).check_value()
        except ValueError as exc:
            return str(exc).split(": ", 1)[-1]
        return ""

    def accept(self):
        msg = self.problem()
        if msg:
            self.error_label.setText(msg)
            self.error_label.show()
            return
        super().accept()


class ColorButton(QToolButton):
    """A swatch for a condition's highlight color: click to choose, right-click for "No color".
    ``mixed`` shows a group whose conditions have different colors."""

    colorChanged = Signal(str)

    def __init__(self, color: str = "", parent=None):
        super().__init__(parent)
        self._color = ""
        self._mixed = False
        self.setAutoRaise(True)
        self.setContextMenuPolicy(Qt.CustomContextMenu)
        self.customContextMenuRequested.connect(self._menu)
        self.clicked.connect(self.choose)
        self.set_color(color)

    def color(self) -> str:
        return self._color

    def set_color(self, color: str, mixed: bool = False):
        self._color, self._mixed = color or "", mixed and not color
        pix = QPixmap(16, 16)
        pix.fill(QColor(self._color) if self._color else Qt.transparent)
        self.setIcon(QIcon(pix))
        self.setText("" if self._color else ("…" if self._mixed else "–"))
        self.setToolButtonStyle(Qt.ToolButtonIconOnly if self._color else Qt.ToolButtonTextOnly)
        tip = (f"Highlight {self._color}" if self._color else
               "Colors differ" if self._mixed else "No highlight")
        self.setToolTip(tip + " — click to choose a color for the cells this condition matches, "
                              "right-click for no color")

    def _apply(self, color: str):
        if color != self._color or self._mixed:
            self.set_color(color)
            self.colorChanged.emit(color)

    def choose(self):
        picked = QColorDialog.getColor(QColor(self._color or "#ffd54f"), self, "Highlight color")
        if picked.isValid():
            self._apply(picked.name())

    def _menu(self, pos):
        menu = QMenu(self)
        menu.addAction("Choose color…", self.choose)
        menu.addAction("No color", lambda: self._apply(""))
        menu.exec(self.mapToGlobal(pos))


def _combo(items, current: str | None = None) -> QComboBox:
    box = QComboBox()
    box.addItems(list(items))
    if current is not None:
        if box.findText(current) < 0:
            box.addItem(current)
        box.setCurrentText(current)
    return box


class ConditionsEditor(QGroupBox):
    """A tree of AND/OR groups and conditions. The tree's widgets are the state; edits that change its
    shape go through :meth:`group` -> change -> :meth:`set_group` so the widgets are always rebuilt cleanly."""

    COLS = ("Group / Sheet", "Name / Column", "Operator", "Value", "Color", "On")

    def __init__(self, window: "MainWindow"):
        super().__init__("Conditions")
        self.win = window
        lay = QVBoxLayout(self)
        self.tree = QTreeWidget()
        self.tree.setColumnCount(len(self.COLS))
        self.tree.setHeaderLabels(self.COLS)
        self.tree.setSelectionMode(QAbstractItemView.SingleSelection)
        header = self.tree.header()
        header.setStretchLastSection(False)
        header.setSectionResizeMode(QHeaderView.Interactive)
        header.setSectionResizeMode(3, QHeaderView.Stretch)
        for col, width in ((0, 190), (1, 165), (2, 100), (4, 44), (5, 34)):
            self.tree.setColumnWidth(col, width)
        self.tree.setIndentation(14)
        self.tree.setMinimumWidth(560)
        self.tree.setMinimumHeight(180)
        lay.addWidget(self.tree)
        self._building = False
        self.tree.itemCollapsed.connect(self._expansion_changed)
        self.tree.itemExpanded.connect(self._expansion_changed)
        row = QHBoxLayout()
        self.add_cond_btn = QPushButton("Add condition")
        self.add_cond_btn.setToolTip("Add a condition to the selected group, on the selected condition's "
                                     "sheet and column")
        self.add_shared_btn = QPushButton("Add for column…")
        self.add_shared_btn.setToolTip("Pick columns that several sheets share; each gets one condition "
                                       "that tests every sheet with that column")
        self.add_group_btn = QPushButton("Add group")
        self.duplicate_btn = QPushButton("Duplicate")
        self.duplicate_btn.setToolTip("Copy the selected condition or group (Ctrl+D)")
        self.remove_btn = QPushButton("Remove")
        self.remove_btn.setToolTip("Remove the selected condition or group (Delete)")
        self.up_btn = QPushButton("Up")
        self.down_btn = QPushButton("Down")
        for b in (self.add_cond_btn, self.add_shared_btn, self.add_group_btn, self.duplicate_btn, self.remove_btn,
                  self.up_btn, self.down_btn):
            row.addWidget(b)
        row.addStretch()
        lay.addLayout(row)
        self.preview = QLabel()
        self.preview.setWordWrap(True)
        self.preview.setTextFormat(Qt.RichText)  # always: condition text is escaped, so `<>` shows
        self.preview.setTextInteractionFlags(Qt.TextSelectableByMouse)
        lay.addWidget(self.preview)
        self.add_cond_btn.clicked.connect(lambda: self.add_condition())
        self.add_group_btn.clicked.connect(lambda: self.add_group())
        self.add_shared_btn.clicked.connect(lambda: self.pick_shared())
        self.duplicate_btn.clicked.connect(lambda: self.duplicate())
        QShortcut(QKeySequence("Ctrl+D"), self, self.duplicate, context=Qt.WidgetWithChildrenShortcut)
        QShortcut(QKeySequence.Delete, self.tree, self.remove, context=Qt.WidgetShortcut)
        self.remove_btn.clicked.connect(lambda: self.remove())
        self.up_btn.clicked.connect(lambda: self.move(-1))
        self.down_btn.clicked.connect(lambda: self.move(1))
        self.set_group(Group())

    # --- building widgets ---------------------------------------------------
    def _source_labels(self) -> list[str]:
        return [ANY_LABEL] + self.win.aliases()

    def _make_group_item(self, parent, group: Group, root: bool = False) -> QTreeWidgetItem:
        item = QTreeWidgetItem(parent)
        item.setData(0, Qt.UserRole, "group")
        item._root = root
        item._collapsed = group.collapsed and not root
        box = QWidget()
        lay = QHBoxLayout(box)
        lay.setContentsMargins(0, 0, 0, 0)
        logic = _combo(LOGIC, group.logic)
        logic.setToolTip("AND: every item must match. OR: any item may match.")
        negate = QCheckBox("NOT")
        negate.setChecked(group.negate)
        lay.addWidget(QLabel("Match"))
        lay.addWidget(logic)
        lay.addWidget(negate)
        lay.addStretch()
        logic.currentTextChanged.connect(self.changed)
        negate.toggled.connect(self.changed)
        self.tree.setItemWidget(item, 0, box)
        item.setFirstColumnSpanned(False)
        item._logic, item._negate, item._box = logic, negate, box
        if not root:
            self._make_group_extras(item, group)
        for sub in group.items:
            if isinstance(sub, Group):
                self._make_group_item(item, sub)
            else:
                self._make_condition_item(item, sub)
        return item

    def _make_group_extras(self, item: QTreeWidgetItem, group: Group):
        """Name, On, and the collapsed group's bulk operator / value (shown only while collapsed)."""
        name = QLineEdit(group.name)
        name.setPlaceholderText("group name")
        name.setToolTip("A label for this group (optional)")
        name.textChanged.connect(self.changed)
        on = QCheckBox()
        on.setChecked(group.enabled)
        on.setToolTip("Untick to switch the whole group off without deleting it")
        on.toggled.connect(self._enabled_changed)
        op = _combo(OPERATORS, "=")
        op.setToolTip("Operator for every condition in this group")
        value = QLineEdit()
        value.setPlaceholderText("value for every condition")
        value.setToolTip("Value for every condition in this group")
        mixed = QLabel("<i>operators / values differ — expand to edit</i>")
        op_cell, value_cell = QWidget(), QWidget()
        for cell, widgets in ((op_cell, (op,)), (value_cell, (value, mixed))):
            lay = QHBoxLayout(cell)
            lay.setContentsMargins(0, 0, 0, 0)
            for w in widgets:
                lay.addWidget(w)
                w.hide()
        color = ColorButton()
        color.colorChanged.connect(lambda c: self._bulk_set(item, color=c))
        for col, w in ((1, name), (2, op_cell), (3, value_cell), (4, color), (5, on)):
            self.tree.setItemWidget(item, col, w)
        item._name, item._on, item._bulk_op, item._bulk_value, item._mixed = name, on, op, value, mixed
        item._color = color
        item._bulk_cells = (op_cell, value_cell)
        op.currentTextChanged.connect(lambda text: self._bulk_set(item, op=text))
        value.textChanged.connect(lambda text: self._bulk_set(item, value=text))

    # --- group state: collapse / bulk edit / on-off -------------------------
    def _group_items(self, item=None) -> list[QTreeWidgetItem]:
        """``item`` (default: the root) and every group under it."""
        item = item or self.tree.topLevelItem(0)
        out = [item]
        for i in range(item.childCount()):
            if item.child(i).data(0, Qt.UserRole) == "group":
                out += self._group_items(item.child(i))
        return out

    def _condition_items(self, item) -> list[QTreeWidgetItem]:
        out = []
        for i in range(item.childCount()):
            child = item.child(i)
            out += self._condition_items(child) if child.data(0, Qt.UserRole) == "group" else [child]
        return out

    def _expansion_changed(self, item):
        if not self._building and item.data(0, Qt.UserRole) == "group":
            self._sync_bulk(item)

    def _sync_bulk(self, item):
        """While a group is collapsed, show one operator / value for all its conditions when they share
        them, else a note that they differ."""
        if item._root:
            return
        conds = self._condition_items(item)
        shared = {(c._op.currentText(), c._value.text()) for c in conds}
        collapsed = not item.isExpanded() and bool(conds)
        uniform = collapsed and len(shared) == 1
        op, value = item._bulk_op, item._bulk_value
        if uniform:
            (op_text, value_text), = shared
            for w, setter, v in ((op, op.setCurrentText, op_text), (value, value.setText, value_text)):
                w.blockSignals(True)
                setter(v)
                w.blockSignals(False)
            value.setEnabled(op_text not in NO_VALUE_OPERATORS)
            columns = {c._column.currentText() for c in conds}
            sources = {c._source.currentText() for c in conds}
            suggestions = set()
            for c in conds:
                src = c._source.currentText()
                suggestions.update(self.win.values_for(ANY_SOURCE if src == ANY_LABEL else src,
                                                       c._column.currentText()))
            value.setCompleter(_search_completer(value, sorted(suggestions, key=_sort_key)[:MAX_SUGGESTIONS]))
            value.setToolTip(f"Value for all {len(conds)} conditions in this group "
                             f"({len(columns)} column{'s' if len(columns) != 1 else ''}, "
                             f"{len(sources)} sheet choice{'s' if len(sources) != 1 else ''})")
        op.setVisible(uniform)
        value.setVisible(uniform)
        item._mixed.setVisible(collapsed and not uniform)

    def _sync_colors(self, item=None):
        """Each group's swatch shows its conditions' color when they all share one."""
        for g in self._group_items(item):
            if g._root:
                continue
            colors = {c._color.color() for c in self._condition_items(g)}
            g._color.set_color(next(iter(colors)) if len(colors) == 1 else "", mixed=len(colors) > 1)

    def _bulk_set(self, item, op: str | None = None, value: str | None = None, color: str | None = None):
        """Apply a group's operator, value or highlight color to every condition in it."""
        for c in self._condition_items(item):
            if op is not None:
                c._op.setCurrentText(op)
            if value is not None:
                c._value.setText(value)
            if color is not None:
                c._color.set_color(color)
        if color is not None:
            self._sync_colors()
            self.changed()
        if op is not None:
            item._bulk_value.setEnabled(op not in NO_VALUE_OPERATORS)

    def _enabled_changed(self, *_):
        self._apply_enabled(self.tree.topLevelItem(0), True)
        self.changed()

    def _apply_enabled(self, item, on: bool):
        """Grey out everything inside a switched-off group (its own On box stays usable)."""
        if item.data(0, Qt.UserRole) == "group":
            if not item._root:
                item._on.setEnabled(on)
                on = on and item._on.isChecked()
                for w in (item._box, item._name, item._color, *item._bulk_cells):
                    w.setEnabled(on)
            for i in range(item.childCount()):
                self._apply_enabled(item.child(i), on)
            return
        for w in (item._source, item._column, item._op, item._color, item._on):
            w.setEnabled(on)
        item._value.setEnabled(on and item._op.currentText() not in NO_VALUE_OPERATORS)

    def _make_condition_item(self, parent, cond: Condition) -> QTreeWidgetItem:
        item = QTreeWidgetItem(parent)
        item.setData(0, Qt.UserRole, "condition")
        src_label = ANY_LABEL if cond.source == ANY_SOURCE else cond.source
        source = _combo(self._source_labels(), src_label)
        column = QComboBox()
        column.setEditable(True)
        column.setInsertPolicy(QComboBox.NoInsert)
        column.setCompleter(_search_completer(column))
        column.completer().setModel(column.model())
        column.setToolTip("Type to search the columns")
        self._fill_columns(column, cond.source, cond.column or None)
        op = _combo(OPERATORS, cond.op)
        value = QLineEdit(cond.value)
        value.setPlaceholderText("value (lists: a, b, c)")
        value.setToolTip("Suggestions come from the column's values. Enter runs the query.")
        value.returnPressed.connect(lambda: self.win.run())
        on = QCheckBox()
        on.setChecked(cond.enabled)
        color = ColorButton(cond.color)
        for col, w in enumerate((source, column, op, value, color, on)):
            self.tree.setItemWidget(item, col, w)
        item._source, item._column, item._op, item._value, item._on = source, column, op, value, on
        item._color = color
        value.setEnabled(cond.op not in NO_VALUE_OPERATORS)

        def alias():
            return ANY_SOURCE if source.currentText() == ANY_LABEL else source.currentText()

        def update_values():
            value.setCompleter(_search_completer(value, self.win.values_for(alias(), column.currentText())))

        def source_changed(_text):
            keep = column.currentText()
            column.blockSignals(True)
            self._fill_columns(column, alias(), None)
            if column.findText(keep) >= 0:
                column.setCurrentText(keep)
            column.blockSignals(False)
            update_values()
            self.changed()

        def column_changed(_text):
            update_values()
            self.changed()

        def op_changed(text):
            value.setEnabled(text not in NO_VALUE_OPERATORS and op.isEnabled())
            self.changed()

        update_values()
        source.currentTextChanged.connect(source_changed)
        op.currentTextChanged.connect(op_changed)
        column.currentTextChanged.connect(column_changed)
        value.textChanged.connect(self.changed)
        on.toggled.connect(self.changed)
        color.colorChanged.connect(lambda _c: (self._sync_colors(), self.changed()))
        return item

    def _fill_columns(self, box: QComboBox, alias: str, current: str | None):
        """Put ``alias``'s columns in ``box``; for any sheet, each one's tooltip names the sheets that have it."""
        box.clear()
        box.addItems(sort_columns(self.win.columns_for(alias)))
        if alias == ANY_SOURCE:
            for i in range(box.count()):
                box.setItemData(i, "In: " + ", ".join(self.win.sheets_with(box.itemText(i))), Qt.ToolTipRole)
        if current is not None:
            if box.findText(current) < 0:
                box.addItem(current)
            box.setCurrentText(current)

    def shared_picker(self) -> "SharedColumnPicker":
        rows = [(col, self.win.sheets_with(col), Qt.Unchecked) for col, _ in self.win.shared_columns()]
        return SharedColumnPicker(rows, self._values_of, self)

    def _values_of(self, columns: list[str]) -> list[str]:
        seen = {v for c in columns for v in self.win.values_for(ANY_SOURCE, c)}
        return sorted(seen, key=_sort_key)[:MAX_SUGGESTIONS]

    def pick_shared(self) -> list[Condition]:
        dlg = self.shared_picker()
        if dlg.exec() != QDialog.Accepted:
            return []
        op, value = dlg.expression()
        return self.add_shared_many(dlg.checked(), op, value, dlg.group_logic(), dlg.color())

    # --- model <-> widgets --------------------------------------------------
    def _read(self, item: QTreeWidgetItem):
        if item.data(0, Qt.UserRole) == "group":
            items = [self._read(item.child(i)) for i in range(item.childCount())]
            if item._root:
                return Group(item._logic.currentText(), items, item._negate.isChecked())
            return Group(item._logic.currentText(), items, item._negate.isChecked(), item._name.text(),
                         item._on.isChecked(), not item.isExpanded())
        src = item._source.currentText()
        return Condition(ANY_SOURCE if src == ANY_LABEL else src, item._column.currentText() or "?",
                         item._op.currentText(), item._value.text(), item._on.isChecked(), item._color.color())

    def group(self) -> Group:
        return self._read(self.tree.topLevelItem(0))

    def set_group(self, group: Group, select: list[int] | None = None, reveal: bool = True):
        """Rebuild the tree from ``group`` and select the item at path ``select``. With ``reveal``, groups
        around it open so it shows (after an edit); otherwise collapsed groups stay shut and the
        outermost one around it is selected instead."""
        self._building = True
        self.tree.clear()
        self._make_group_item(self.tree, group, root=True)
        groups = self._group_items()
        for g in groups:
            g.setExpanded(not g._collapsed)
        self._building = False
        if select is not None:
            item = self._item_at(select)
            if item is not None:
                parent = item.parent()
                while parent is not None:
                    if reveal:
                        parent.setExpanded(True)
                    elif not parent.isExpanded():
                        item = parent
                    parent = parent.parent()
                self.tree.setCurrentItem(item)
        for g in groups:
            self._sync_bulk(g)
        self._sync_colors()
        self._apply_enabled(self.tree.topLevelItem(0), True)
        self.add_shared_btn.setEnabled(bool(self.win.shared_columns()))
        self.changed()

    def refresh(self):
        """Rebuild after sources change (new sheet names / columns in the combos)."""
        self.set_group(self.group(), self._path(self.tree.currentItem()), reveal=False)

    def changed(self, *_):
        try:
            text = self.group().describe()
        except ValueError as exc:
            text = str(exc)
        self.preview.setText(f"<b>Where:</b> {_esc(text)}")

    # --- paths: list of child indexes from the root group -----------------
    def _path(self, item) -> list[int] | None:
        if item is None:
            return None
        path = []
        while item.parent() is not None:
            path.insert(0, item.parent().indexOfChild(item))
            item = item.parent()
        return path

    def _item_at(self, path: list[int]):
        item = self.tree.topLevelItem(0)
        for i in path:
            if item is None or i >= item.childCount():
                return None
            item = item.child(i)
        return item

    @staticmethod
    def _node(group: Group, path: list[int]):
        node = group
        for i in path:
            node = node.items[i]
        return node

    def _target(self) -> tuple[Group, list[int], list[int]]:
        """(model, path of the group to add into, path of the selection)."""
        model = self.group()
        sel = self._path(self.tree.currentItem()) or []
        node = self._node(model, sel)
        return model, (sel if isinstance(node, Group) else sel[:-1]), sel

    # --- edits --------------------------------------------------------------
    def _default_condition(self, model: Group, sel: list[int]) -> Condition:
        """Same sheet and column as the selected condition; else any sheet (with a column several sheets
        share, if there is one) when there are several sheets; else the only sheet."""
        node = self._node(model, sel)
        if isinstance(node, Condition):
            return Condition(node.source, node.column, "=", "")
        aliases = self.win.aliases()
        src = ANY_SOURCE if len(aliases) != 1 else aliases[0]
        shared = self.win.shared_columns()
        cols = [shared[0][0]] if src == ANY_SOURCE and shared else self.win.columns_for(src)
        return Condition(src, cols[0] if cols else "?", "=", "")

    def add_condition(self, cond: Condition | None = None) -> Condition:
        model, gpath, sel = self._target()
        if cond is None:
            cond = self._default_condition(model, sel)
        self._node(model, gpath).items.append(cond)
        path = gpath + [len(self._node(model, gpath).items) - 1]
        self.set_group(model, path)
        item = self._item_at(path)
        if item is not None and item._value.isEnabled():
            item._value.setFocus()
        return cond

    def add_shared(self, column: str) -> Condition:
        """A condition on ``column`` of every sheet that has it."""
        return self.add_condition(Condition(ANY_SOURCE, column, "=", ""))

    def add_shared_many(self, columns: list[str], op: str = "=", value: str = "",
                        new_group: str | None = None, color: str = "") -> list[Condition]:
        """One any-sheet condition per column, all with ``op`` and ``value``: in the selected group, or in a
        new AND / OR group (``new_group``) added to it. The first condition is selected; when its value is
        still blank its value box gets focus. ``color`` is the highlight each condition gets."""
        added = [Condition(ANY_SOURCE, col, op, value, color=color) for col in columns]
        if not added:
            return added
        model, gpath, _ = self._target()
        group = self._node(model, gpath)
        if new_group:
            group.items.append(Group(new_group, list(added)))
            first = gpath + [len(group.items) - 1, 0]
        else:
            group.items.extend(added)
            first = gpath + [len(group.items) - len(added)]
        self.set_group(model, first)
        item = self._item_at(first)
        if item is not None and item._value.isEnabled() and not value.strip():
            item._value.setFocus()
        return added

    def duplicate(self) -> bool:
        """Insert a copy of the selected condition or group right after it."""
        model, _, sel = self._target()
        if not sel:
            return False
        items = self._node(model, sel[:-1]).items
        items.insert(sel[-1] + 1, copy.deepcopy(items[sel[-1]]))
        self.set_group(model, sel[:-1] + [sel[-1] + 1])
        return True

    def add_group(self, logic: str = "OR") -> None:
        model, gpath, _ = self._target()
        self._node(model, gpath).items.append(Group(logic))
        self.set_group(model, gpath + [len(self._node(model, gpath).items) - 1])

    def remove(self) -> bool:
        model, _, sel = self._target()
        if not sel:
            return False
        del self._node(model, sel[:-1]).items[sel[-1]]
        parent = sel[:-1]
        n = len(self._node(model, parent).items)
        self.set_group(model, parent + [min(sel[-1], n - 1)] if n else parent)
        return True

    def move(self, step: int) -> bool:
        model, _, sel = self._target()
        if not sel:
            return False
        items = self._node(model, sel[:-1]).items
        i, j = sel[-1], sel[-1] + step
        if not 0 <= j < len(items):
            return False
        items[i], items[j] = items[j], items[i]
        self.set_group(model, sel[:-1] + [j])
        return True

    def select(self, path: list[int]):
        self.tree.setCurrentItem(self._item_at(path))

    def rename_source(self, old: str, new: str):
        def walk(g: Group):
            for it in g.items:
                if isinstance(it, Group):
                    walk(it)
                elif it.source == old:
                    it.source = new
        model = self.group()
        walk(model)
        self.set_group(model, self._path(self.tree.currentItem()), reveal=False)


class OutputsEditor(QGroupBox):
    """Per source: a checkable, reorderable list of its columns."""

    def __init__(self):
        super().__init__("Output columns")
        lay = QVBoxLayout(self)
        top = QHBoxLayout()
        self.all_sheets_btn = QPushButton("Choose for all sheets…")
        self.all_sheets_btn.setToolTip("Tick or untick a column in every sheet that has it, all at once")
        self.all_sheets_btn.clicked.connect(lambda: self.pick_all_sheets())
        self.all_sheets_btn.setEnabled(False)
        top.addWidget(self.all_sheets_btn)
        top.addStretch()
        lay.addLayout(top)
        self.toolbox = QToolBox()
        self.toolbox.setMinimumHeight(200)
        lay.addWidget(self.toolbox)
        self.lists: dict[str, QListWidget] = {}
        hint = QLabel("Tick the columns to output for each sheet; drag to reorder.")
        hint.setWordWrap(True)
        lay.addWidget(hint)

    def rebuild(self, entries: list[Entry], keep: dict[str, list[str]] | None = None):
        """One page per source. ``keep`` (alias -> checked columns, in order) restores choices;
        a source not in it gets every column ticked."""
        keep = self.outputs() if keep is None else keep
        while self.toolbox.count():
            w = self.toolbox.widget(0)
            self.toolbox.removeItem(0)
            w.deleteLater()
        self.lists = {}
        for e in entries:
            alias = e.source.alias
            page = QWidget()
            lay = QVBoxLayout(page)
            lst = QListWidget()
            lst.setDragDropMode(QAbstractItemView.InternalMove)
            chosen = keep.get(alias)
            ordered = list(chosen or []) + [c for c in e.columns if c not in (chosen or [])]
            for col in ordered:
                if col not in e.columns:
                    continue
                item = QListWidgetItem(col)
                item.setFlags(item.flags() | Qt.ItemIsUserCheckable)
                ticked = chosen is None or col in chosen
                item.setCheckState(Qt.Checked if ticked else Qt.Unchecked)
                lst.addItem(item)
            row = QHBoxLayout()
            all_btn, none_btn = QPushButton("All"), QPushButton("None")
            all_btn.clicked.connect(lambda _=False, l=lst: self._set_all(l, True))
            none_btn.clicked.connect(lambda _=False, l=lst: self._set_all(l, False))
            row.addWidget(all_btn)
            row.addWidget(none_btn)
            row.addStretch()
            lay.addWidget(lst)
            lay.addLayout(row)
            self.toolbox.addItem(page, alias)
            self.lists[alias] = lst
        self.all_sheets_btn.setEnabled(bool(self.lists))

    @staticmethod
    def _set_all(lst: QListWidget, on: bool):
        for i in range(lst.count()):
            lst.item(i).setCheckState(Qt.Checked if on else Qt.Unchecked)

    def set_checked(self, alias: str, columns: list[str]):
        lst = self.lists[alias]
        for i in range(lst.count()):
            lst.item(i).setCheckState(Qt.Checked if lst.item(i).text() in columns else Qt.Unchecked)

    def _item(self, alias: str, column: str) -> QListWidgetItem | None:
        lst = self.lists[alias]
        names = [lst.item(i).text() for i in range(lst.count())]
        name = resolve_column(names, column)
        return None if name is None else lst.item(names.index(name))

    def column_states(self) -> list[tuple[str, list[str], Qt.CheckState]]:
        """Every column of every sheet (once, ignoring case and spaces): the sheets that have it, and
        whether it's ticked in all of them, some or none."""
        rows, seen = [], set()
        for lst in self.lists.values():
            for i in range(lst.count()):
                col = lst.item(i).text()
                if column_key(col) in seen:
                    continue
                seen.add(column_key(col))
                sheets = [a for a in self.lists if self._item(a, col) is not None]
                ticked = sum(self._item(a, col).checkState() == Qt.Checked for a in sheets)
                state = Qt.Checked if ticked == len(sheets) else Qt.Unchecked if not ticked else Qt.PartiallyChecked
                rows.append((col, sheets, state))
        return rows

    def apply_states(self, states: dict[str, Qt.CheckState]):
        """Tick (or untick) each column in every sheet that has it; partly-checked columns stay as they are."""
        for col, state in states.items():
            if state == Qt.PartiallyChecked:
                continue
            for alias in self.lists:
                item = self._item(alias, col)
                if item is not None:
                    item.setCheckState(state)

    def all_sheets_picker(self) -> ColumnPicker:
        rows = self.column_states()
        order = {c: i for i, c in enumerate(sort_columns(r[0] for r in rows))}
        rows.sort(key=lambda r: order[r[0]])
        return ColumnPicker("Output columns for all sheets",
                            "Tick the columns to output. Each applies to every sheet that has it; "
                            "a half-ticked column is left as it is.", rows, self)

    def pick_all_sheets(self) -> bool:
        dlg = self.all_sheets_picker()
        if dlg.exec() != QDialog.Accepted:
            return False
        self.apply_states(dlg.states())
        return True

    def outputs(self) -> dict[str, list[str]]:
        return {alias: [lst.item(i).text() for i in range(lst.count())
                        if lst.item(i).checkState() == Qt.Checked]
                for alias, lst in self.lists.items()}


class MainWindow(QMainWindow):
    SRC_COLS = ("Name", "File", "Sheet", "Join key")

    def __init__(self):
        super().__init__()
        self.setWindowTitle(f"{APP} {__version__}")
        self.setAcceptDrops(True)
        self.entries: list[Entry] = []
        self.result: QueryResult | None = None
        self._updating = False
        self._values_cache: dict[tuple[str, str], list[str]] = {}

        # --- left: sources, mode, conditions, outputs ---
        left = QWidget()
        llay = QVBoxLayout(left)

        src_box = QGroupBox("Workbook sheets")
        slay = QVBoxLayout(src_box)
        self.src_table = QTableWidget(0, len(self.SRC_COLS))
        self.src_table.setHorizontalHeaderLabels(self.SRC_COLS)
        self.src_table.horizontalHeader().setSectionResizeMode(QHeaderView.Interactive)
        self.src_table.horizontalHeader().setStretchLastSection(True)
        self.src_table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.src_table.setMinimumHeight(110)
        self.src_table.itemChanged.connect(self._alias_edited)
        slay.addWidget(self.src_table)
        row = QHBoxLayout()
        self.add_btn = QPushButton("Add workbooks…")
        self.remove_src_btn = QPushButton("Remove sheet")
        self.add_btn.clicked.connect(lambda: self.browse_workbooks())
        self.remove_src_btn.clicked.connect(lambda: self.remove_source(self.src_table.currentRow()))
        row.addWidget(self.add_btn)
        row.addWidget(self.remove_src_btn)
        row.addStretch()
        slay.addLayout(row)
        llay.addWidget(src_box)

        mode_box = QGroupBox("How sheets combine")
        mlay = QHBoxLayout(mode_box)
        self.independent_radio = QRadioButton("Filter each sheet on its own")
        self.join_radio = QRadioButton("Join sheets on a key column")
        self.independent_radio.setChecked(True)
        self._mode_group = QButtonGroup(self)
        self._mode_group.addButton(self.independent_radio)
        self._mode_group.addButton(self.join_radio)
        self.join_how_box = _combo(JOIN_HOW, "inner")
        self.join_how_box.setToolTip("inner: keys found in every sheet. left: every row of the first sheet. "
                                     "outer: every key from any sheet.")
        self.provenance_check = QCheckBox("Add file / sheet / row columns")
        self.provenance_check.setChecked(True)
        mlay.addWidget(self.independent_radio)
        mlay.addWidget(self.join_radio)
        mlay.addWidget(self.join_how_box)
        mlay.addWidget(self.provenance_check)
        mlay.addStretch()
        self.join_radio.toggled.connect(self._mode_changed)
        llay.addWidget(mode_box)

        self.conditions = ConditionsEditor(self)
        llay.addWidget(self.conditions, 2)
        self.outputs = OutputsEditor()
        llay.addWidget(self.outputs, 1)

        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setWidget(left)

        # --- right: actions and results ---
        right = QWidget()
        rlay = QVBoxLayout(right)
        bar = QHBoxLayout()
        self.run_btn = QPushButton("Run query")
        self.run_btn.setDefault(True)
        self.export_xlsx_btn = QPushButton("Export Excel…")
        self.export_html_btn = QPushButton("Export HTML…")
        self.visualize_btn = QPushButton("Visualize")
        self.visualize_btn.setCheckable(True)
        self.visualize_btn.setToolTip("Chart the results (switch back to see the tables)")
        self.save_query_btn = QPushButton("Save query…")
        self.load_query_btn = QPushButton("Load query…")
        for b in (self.run_btn, self.export_xlsx_btn, self.export_html_btn, self.visualize_btn):
            bar.addWidget(b)
        bar.addStretch()
        bar.addWidget(self.save_query_btn)
        bar.addWidget(self.load_query_btn)
        rlay.addLayout(bar)
        self.status_label = QLabel("Add workbooks to start.")
        self.status_label.setWordWrap(True)
        self.status_label.setTextFormat(Qt.RichText)  # always: values from the data are escaped
        rlay.addWidget(self.status_label)
        self.tabs = QTabWidget()
        from .gui_charts import ChartPanel  # matplotlib loads with the window, not on import of this module

        self.charts = ChartPanel()
        self.view_tabs = QTabWidget()
        self.view_tabs.addTab(self.tabs, "Tables")
        self.view_tabs.addTab(self.charts, "Visualize")
        rlay.addWidget(self.view_tabs, 1)
        self.visualize_btn.toggled.connect(lambda on: self.view_tabs.setCurrentIndex(1 if on else 0))
        self.view_tabs.currentChanged.connect(lambda i: self.visualize_btn.setChecked(i == 1))
        self.run_btn.clicked.connect(lambda: self.run())
        self.export_xlsx_btn.clicked.connect(lambda: self.export("xlsx"))
        self.export_html_btn.clicked.connect(lambda: self.export("html"))
        self.save_query_btn.clicked.connect(lambda: self.save_query())
        self.load_query_btn.clicked.connect(lambda: self.load_query())

        split = QSplitter()
        split.addWidget(scroll)
        split.addWidget(right)
        split.setStretchFactor(0, 2)
        split.setStretchFactor(1, 3)
        self.setCentralWidget(split)
        self.resize(1500, 900)
        self._mode_changed()
        self._update_buttons()

    # --- helpers -------------------------------------------------------------
    def error(self, msg: str):
        # Always rich text, escaped: messages quote conditions such as `<> '0'` and values from the data.
        QMessageBox.critical(self, APP, "<p>" + _esc(msg).replace("\n", "<br>") + "</p>")

    def aliases(self) -> list[str]:
        return [e.source.alias for e in self.entries]

    def entry(self, alias: str) -> Entry | None:
        return next((e for e in self.entries if e.source.alias == alias), None)

    def columns_for(self, alias: str) -> list[str]:
        """A sheet's columns; for any sheet, every sheet's columns, once each (ignoring case and spaces)."""
        if alias == ANY_SOURCE:
            out, seen = [], set()
            for e in self.entries:
                for c in e.columns:
                    if column_key(c) not in seen:
                        seen.add(column_key(c))
                        out.append(c)
            return out
        e = self.entry(alias)
        return e.columns if e else []

    def sheets_with(self, column: str) -> list[str]:
        """Names of the sheets that have ``column`` (ignoring case and surrounding spaces)."""
        return [e.source.alias for e in self.entries if resolve_column(e.columns, column) is not None]

    def shared_columns(self) -> list[tuple[str, int]]:
        """(column, number of sheets) for each column found in more than one sheet."""
        if len(self.entries) < 2:
            return []
        out = []
        for c in sort_columns(self.columns_for(ANY_SOURCE)):
            n = len(self.sheets_with(c))
            if n > 1:
                out.append((c, n))
        return out

    def values_for(self, alias: str, column: str) -> list[str]:
        """Distinct non-blank values of ``column`` in sheet ``alias`` (or every sheet with it), sorted."""
        cache_key = (alias, column)
        if cache_key not in self._values_cache:
            entries = self.entries if alias == ANY_SOURCE else [e for e in self.entries if e.source.alias == alias]
            seen = set()
            for e in entries:
                col = resolve_column(e.columns, column)
                if col is None:
                    continue
                for v in e.frame[col]:
                    if not _is_blank(v):
                        text = _value_text(v)
                        if text:
                            seen.add(text)
            self._values_cache[cache_key] = sorted(seen, key=_sort_key)[:MAX_SUGGESTIONS]
        return self._values_cache[cache_key]

    def condition_target(self, column: str) -> tuple[str, str] | None:
        """(source, column) a condition on result column ``column`` should test, or None when it can't
        be tested (provenance and key columns). Shared columns test every sheet that has them."""
        if is_provenance(column):
            return None
        if self.result is not None and self.result.query.mode == "join":
            for a in sorted(self.aliases(), key=len, reverse=True):
                if column.startswith(f"{a}."):
                    return a, column[len(a) + 1:]
            return None
        sheets = self.sheets_with(column)
        if not sheets:
            return None
        return (ANY_SOURCE if len(sheets) > 1 else sheets[0]), column

    def cell_conditions(self, column: str, value) -> list[Condition]:
        """Conditions offered for a result cell: = and <> its value, or is (not) blank."""
        target = self.condition_target(column)
        if target is None:
            return []
        src, col = target
        text = _value_text(value)
        if not text:
            return [Condition(src, col, "is blank"), Condition(src, col, "is not blank")]
        return [Condition(src, col, "=", text), Condition(src, col, "<>", text)]

    def add_condition_from_result(self, cond: Condition):
        self.conditions.add_condition(cond)
        self.status_label.setText(f"Added condition: {_esc(cond.describe())}. Run the query to apply it.")

    def _result_menu(self, view: QTableView, pos):
        index = view.indexAt(pos)
        if not index.isValid():
            return
        proxy = view.model()
        src = proxy.mapToSource(index)
        df = proxy.sourceModel().df
        conds = self.cell_conditions(str(df.columns[src.column()]), df.iat[src.row(), src.column()])
        if not conds:
            return
        menu = QMenu(view)
        for cond in conds:
            act = menu.addAction(f"Add condition: {cond.describe()}")
            act.triggered.connect(lambda _=False, c=cond: self.add_condition_from_result(c))
        menu.exec(view.viewport().mapToGlobal(pos))

    def _update_buttons(self):
        has = bool(self.entries)
        self.run_btn.setEnabled(has)
        self.save_query_btn.setEnabled(has)
        self.export_xlsx_btn.setEnabled(self.result is not None)
        self.export_html_btn.setEnabled(self.result is not None)
        self.visualize_btn.setEnabled(self.result is not None)

    def _mode_changed(self, *_):
        join = self.join_radio.isChecked()
        self.join_how_box.setEnabled(join)
        self.src_table.setColumnHidden(3, not join)

    # --- sources ----------------------------------------------------------------
    def browse_workbooks(self) -> int:
        paths, _ = QFileDialog.getOpenFileNames(self, "Add workbooks", "", WORKBOOK_FILTER)
        return sum(self.add_workbook(p) for p in paths)

    def add_workbook(self, path: str | Path, sheets: list[str] | None = None) -> int:
        """Add sheets of ``path`` (asking which when it has several and ``sheets`` isn't given).
        Returns how many were added."""
        path = str(path)
        try:
            names = loader.list_sheets(path)
        except Exception as exc:  # noqa: BLE001 - anything unreadable is reported, not raised
            self.error(f"Can't read {path}:\n{exc}")
            return 0
        if sheets is None:
            if len(names) <= 1:
                sheets = names
            else:
                dlg = SheetPicker(path, names, self)
                if dlg.exec() != QDialog.Accepted:
                    return 0
                sheets = dlg.chosen()
        return sum(self.add_source(path, s) is not None for s in sheets)

    def add_source(self, path: str, sheet: str, alias: str | None = None, key: str = "",
                   refresh: bool = True) -> Entry | None:
        try:
            frame = loader.load_table(path, sheet)
        except Exception as exc:  # noqa: BLE001
            self.error(f"Can't read sheet '{sheet}' of {path}:\n{exc}")
            return None
        alias = unique_alias(alias or default_alias(path, sheet), self.aliases())
        e = Entry(Source(alias, str(path), sheet, key), frame)
        if not e.source.key or e.source.key not in e.columns:
            e.source.key = self._guess_key(e)
        self.entries.append(e)
        self._add_source_row(e)
        if refresh:
            self._sources_changed()
        return e

    def _guess_key(self, e: Entry) -> str:
        others = {c.strip().casefold() for o in self.entries for c in o.columns}
        for c in e.columns:
            if c.strip().casefold() in others:
                return c
        return e.columns[0] if e.columns else ""

    def _add_source_row(self, e: Entry):
        self._updating = True
        r = self.src_table.rowCount()
        self.src_table.insertRow(r)
        name = QTableWidgetItem(e.source.alias)
        name.setToolTip("Double-click to rename")
        self.src_table.setItem(r, 0, name)
        for col, text in ((1, Path(e.source.path).name), (2, e.source.sheet)):
            item = QTableWidgetItem(text)
            item.setFlags(item.flags() & ~Qt.ItemIsEditable)
            item.setToolTip(e.source.path)
            self.src_table.setItem(r, col, item)
        key = _combo(sort_columns(e.columns), e.source.key)
        key.currentTextChanged.connect(lambda text, e=e: setattr(e.source, "key", text))
        self.src_table.setCellWidget(r, 3, key)
        self.src_table.resizeColumnsToContents()
        self._updating = False

    def key_box(self, row: int) -> QComboBox:
        return self.src_table.cellWidget(row, 3)

    def remove_source(self, row: int) -> bool:
        if not 0 <= row < len(self.entries):
            return False
        del self.entries[row]
        self.src_table.removeRow(row)
        self._sources_changed()
        return True

    def rename_source(self, row: int, new: str) -> bool:
        self.src_table.item(row, 0).setText(new)
        return self.entries[row].source.alias == new.strip()

    def _alias_edited(self, item: QTableWidgetItem):
        if self._updating or item.column() != 0:
            return
        e = self.entries[item.row()]
        old, new = e.source.alias, item.text().strip()
        if new == old:
            return
        if not new or new in self.aliases():
            self._updating = True
            item.setText(old)
            self._updating = False
            self.error(f"'{new}' is blank or already used; each sheet needs its own name.")
            return
        keep = self.outputs.outputs()
        keep[new] = keep.pop(old, None)
        e.source.alias = new
        self._values_cache.clear()
        self.outputs.rebuild(self.entries, {k: v for k, v in keep.items() if v is not None})
        self.conditions.rename_source(old, new)

    def _sources_changed(self):
        self._values_cache.clear()
        self.outputs.rebuild(self.entries)
        self.conditions.refresh()
        n = len(self.entries)
        self.status_label.setText(f"{n} sheet{'s' if n != 1 else ''} loaded. Build conditions, tick output "
                                  "columns, then Run query." if n else "Add workbooks to start.")
        self._update_buttons()

    # --- query <-> widgets ------------------------------------------------------
    def build_query(self) -> Query:
        return Query(
            sources=[Source(e.source.alias, e.source.path, e.source.sheet, e.source.key) for e in self.entries],
            where=self.conditions.group(),
            outputs=self.outputs.outputs(),
            mode="join" if self.join_radio.isChecked() else "independent",
            join_how=self.join_how_box.currentText(),
            include_provenance=self.provenance_check.isChecked(),
        )

    def apply_query(self, query: Query) -> bool:
        """Replace everything with ``query``'s sources, conditions and outputs."""
        self.entries = []
        self._values_cache.clear()
        self.src_table.setRowCount(0)
        ok = True
        for s in query.sources:
            e = self.add_source(s.path, s.sheet, s.alias, s.key, refresh=False)
            ok &= e is not None and e.source.alias == s.alias
        (self.join_radio if query.mode == "join" else self.independent_radio).setChecked(True)
        self.join_how_box.setCurrentText(query.join_how)
        self.provenance_check.setChecked(query.include_provenance)
        self.outputs.rebuild(self.entries, {a: c for a, c in query.outputs.items()})
        for e in self.entries:  # a source missing from "outputs" outputs nothing, as when the query was saved
            if e.source.alias not in query.outputs:
                self.outputs.set_checked(e.source.alias, [])
        self.conditions.set_group(query.where)
        self._clear_results()
        self._sources_changed_status()
        return ok

    def _sources_changed_status(self):
        n = len(self.entries)
        self.status_label.setText(f"Query loaded with {n} sheet{'s' if n != 1 else ''}.")
        self._update_buttons()

    def save_query(self, path: str | None = None) -> Path | None:
        if path is None:
            path, _ = QFileDialog.getSaveFileName(self, "Save query", "query.json", "Query (*.json)")
            if not path:
                return None
        try:
            return save_query(self.build_query(), path)
        except (OSError, ValueError) as exc:
            self.error(exc)
            return None

    def load_query(self, path: str | None = None) -> bool:
        if path is None:
            path, _ = QFileDialog.getOpenFileName(self, "Load query", "", "Query (*.json)")
            if not path:
                return False
        try:
            query = load_query(path)
        except ValueError as exc:
            self.error(exc)
            return False
        return self.apply_query(query)

    # --- running and exporting --------------------------------------------------
    def _clear_results(self):
        self.result = None
        self.tabs.clear()
        self.charts.set_result(None)
        self._update_buttons()

    def run(self) -> bool:
        if not self.entries:
            self.error("Add workbooks first.")
            return False
        try:
            query = self.build_query()
            result = run_query(query, {e.source.alias: e.frame for e in self.entries})
        except (ValueError, KeyError) as exc:
            self._clear_results()
            self.error(exc.args[0] if isinstance(exc, KeyError) and exc.args else exc)
            return False
        self.result = result
        self.tabs.clear()
        for name, df in result.tables.items():
            view = QTableView()
            proxy = SortProxy(view)
            proxy.setSourceModel(FrameModel(df, view, result.colors.get(name)))
            view.setModel(proxy)
            view.setSortingEnabled(True)
            view.sortByColumn(-1, Qt.AscendingOrder)
            view.horizontalHeader().setSectionResizeMode(QHeaderView.Interactive)
            view.resizeColumnsToContents()
            view.setContextMenuPolicy(Qt.CustomContextMenu)
            view.setToolTip("Right-click a cell to add a condition on its value")
            view.customContextMenuRequested.connect(lambda pos, v=view: self._result_menu(v, pos))
            self.tabs.addTab(view, f"{name} ({len(df)})")
        self.charts.set_result(result)
        counts = ", ".join(f"{_esc(n)}: {len(df)} row{'s' if len(df) != 1 else ''}"
                           for n, df in result.tables.items())
        text = f"<b>Matched</b> — {counts}"
        if result.warnings:
            text += "".join(f"<br>⚠ {_esc(w)}" for w in result.warnings)
        self.status_label.setText(text)
        self._update_buttons()
        return True

    def export(self, fmt: str, path: str | None = None) -> Path | None:
        if self.result is None:
            return None
        if path is None:
            filt = "Excel workbook (*.xlsx)" if fmt == "xlsx" else "HTML report (*.html)"
            path, _ = QFileDialog.getSaveFileName(self, "Export results", f"query_results.{fmt}", filt)
            if not path:
                return None
        try:
            if fmt == "xlsx":
                from .export_xlsx import write_xlsx

                return write_xlsx(self.result, path)
            from .export_html import write_html

            return write_html(self.result, path)
        except OSError as exc:
            self.error(f"Couldn't write {path}:\n{exc}")
            return None

    # --- drag and drop ------------------------------------------------------------
    def dragEnterEvent(self, event):
        if event.mimeData().hasUrls():
            event.acceptProposedAction()

    def dropEvent(self, event):
        for url in event.mimeData().urls():
            if url.isLocalFile():
                self.add_workbook(url.toLocalFile())


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    app = QApplication.instance() or QApplication(sys.argv[:1])
    app.setApplicationName(APP)
    app.setDesktopFileName("wbquery")
    if ICON.exists():
        app.setWindowIcon(QIcon(str(ICON)))
    win = MainWindow()
    win.show()
    for path in argv:
        if path.lower().endswith(".json"):
            win.load_query(path)
        else:
            win.add_workbook(path)
    return app.exec()
