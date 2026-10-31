"""Desktop app: pick workbook sheets, build an AND/OR query, choose output columns, run and export."""

from __future__ import annotations

import sys
from dataclasses import dataclass
from pathlib import Path

import pandas as pd
from PySide6.QtCore import QAbstractTableModel, QModelIndex, QSortFilterProxyModel, Qt
from PySide6.QtWidgets import (
    QAbstractItemView, QApplication, QButtonGroup, QCheckBox, QComboBox, QDialog, QDialogButtonBox, QFileDialog,
    QGroupBox, QHBoxLayout, QHeaderView, QLabel, QLineEdit, QListWidget, QListWidgetItem, QMainWindow,
    QMessageBox, QPushButton, QRadioButton, QScrollArea, QSplitter, QTableView, QTableWidget, QTableWidgetItem,
    QTabWidget, QToolBox, QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget,
)

from . import __version__, loader
from .conditions import ANY_SOURCE, LOGIC, NO_VALUE_OPERATORS, OPERATORS, Condition, Group
from .query import JOIN_HOW, Query, QueryResult, Source, default_alias, load_query, run_query, save_query, \
    unique_alias

APP = "Workbook Query"
ANY_LABEL = "Any sheet"
WORKBOOK_FILTER = "Workbooks (*.xlsx *.xlsm *.xls *.csv *.tsv *.txt);;All files (*)"


@dataclass
class Entry:
    source: Source
    frame: pd.DataFrame

    @property
    def columns(self) -> list[str]:
        return [str(c) for c in self.frame.columns]


class FrameModel(QAbstractTableModel):
    """Read-only view of a DataFrame."""

    def __init__(self, df: pd.DataFrame, parent=None):
        super().__init__(parent)
        self.df = df

    def rowCount(self, parent=QModelIndex()):
        return 0 if parent.isValid() else len(self.df)

    def columnCount(self, parent=QModelIndex()):
        return 0 if parent.isValid() else self.df.shape[1]

    def data(self, index, role=Qt.DisplayRole):
        if not index.isValid():
            return None
        v = self.df.iat[index.row(), index.column()]
        if role == Qt.DisplayRole:
            if v is None or (not isinstance(v, str) and pd.isna(v)):
                return ""
            if isinstance(v, pd.Timestamp):
                return v.strftime("%Y-%m-%d") if v == v.normalize() else str(v)
            return str(v)
        if role == Qt.UserRole:  # sort key
            if v is None or (not isinstance(v, str) and pd.isna(v)):
                return None
            return v if isinstance(v, (int, float)) else str(v)
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

    COLS = ("Group / Sheet", "Column", "Operator", "Value", "On")

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
        for col, width in ((0, 190), (1, 165), (2, 100), (4, 34)):
            self.tree.setColumnWidth(col, width)
        self.tree.setIndentation(14)
        self.tree.setMinimumWidth(560)
        self.tree.setMinimumHeight(180)
        lay.addWidget(self.tree)
        row = QHBoxLayout()
        self.add_cond_btn = QPushButton("Add condition")
        self.add_group_btn = QPushButton("Add group")
        self.remove_btn = QPushButton("Remove")
        self.up_btn = QPushButton("Up")
        self.down_btn = QPushButton("Down")
        for b in (self.add_cond_btn, self.add_group_btn, self.remove_btn, self.up_btn, self.down_btn):
            row.addWidget(b)
        row.addStretch()
        lay.addLayout(row)
        self.preview = QLabel()
        self.preview.setWordWrap(True)
        self.preview.setTextInteractionFlags(Qt.TextSelectableByMouse)
        lay.addWidget(self.preview)
        self.add_cond_btn.clicked.connect(lambda: self.add_condition())
        self.add_group_btn.clicked.connect(lambda: self.add_group())
        self.remove_btn.clicked.connect(lambda: self.remove())
        self.up_btn.clicked.connect(lambda: self.move(-1))
        self.down_btn.clicked.connect(lambda: self.move(1))
        self.set_group(Group())

    # --- building widgets ---------------------------------------------------
    def _source_labels(self) -> list[str]:
        return [ANY_LABEL] + self.win.aliases()

    def _make_group_item(self, parent, group: Group) -> QTreeWidgetItem:
        item = QTreeWidgetItem(parent)
        item.setData(0, Qt.UserRole, "group")
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
        item._logic, item._negate = logic, negate
        for sub in group.items:
            if isinstance(sub, Group):
                self._make_group_item(item, sub)
            else:
                self._make_condition_item(item, sub)
        item.setExpanded(True)
        return item

    def _make_condition_item(self, parent, cond: Condition) -> QTreeWidgetItem:
        item = QTreeWidgetItem(parent)
        item.setData(0, Qt.UserRole, "condition")
        src_label = ANY_LABEL if cond.source == ANY_SOURCE else cond.source
        source = _combo(self._source_labels(), src_label)
        column = _combo(self.win.columns_for(cond.source), cond.column or None)
        op = _combo(OPERATORS, cond.op)
        value = QLineEdit(cond.value)
        value.setPlaceholderText("value (lists: a, b, c)")
        on = QCheckBox()
        on.setChecked(cond.enabled)
        for col, w in enumerate((source, column, op, value, on)):
            self.tree.setItemWidget(item, col, w)
        item._source, item._column, item._op, item._value, item._on = source, column, op, value, on
        value.setEnabled(cond.op not in NO_VALUE_OPERATORS)

        def source_changed(text):
            alias = ANY_SOURCE if text == ANY_LABEL else text
            keep = column.currentText()
            column.blockSignals(True)
            column.clear()
            column.addItems(self.win.columns_for(alias))
            if column.findText(keep) >= 0:
                column.setCurrentText(keep)
            column.blockSignals(False)
            self.changed()

        def op_changed(text):
            value.setEnabled(text not in NO_VALUE_OPERATORS)
            self.changed()

        source.currentTextChanged.connect(source_changed)
        op.currentTextChanged.connect(op_changed)
        column.currentTextChanged.connect(self.changed)
        value.textChanged.connect(self.changed)
        on.toggled.connect(self.changed)
        return item

    # --- model <-> widgets --------------------------------------------------
    def _read(self, item: QTreeWidgetItem):
        if item.data(0, Qt.UserRole) == "group":
            return Group(item._logic.currentText(), [self._read(item.child(i)) for i in range(item.childCount())],
                         item._negate.isChecked())
        src = item._source.currentText()
        return Condition(ANY_SOURCE if src == ANY_LABEL else src, item._column.currentText() or "?",
                         item._op.currentText(), item._value.text(), item._on.isChecked())

    def group(self) -> Group:
        return self._read(self.tree.topLevelItem(0))

    def set_group(self, group: Group, select: list[int] | None = None):
        self.tree.clear()
        root = self._make_group_item(self.tree, group)
        root._root = True
        self.tree.expandAll()
        if select is not None:
            item = self._item_at(select)
            if item is not None:
                self.tree.setCurrentItem(item)
        self.changed()

    def refresh(self):
        """Rebuild after sources change (new sheet names / columns in the combos)."""
        self.set_group(self.group(), self._path(self.tree.currentItem()))

    def changed(self, *_):
        try:
            text = self.group().describe()
        except ValueError as exc:
            text = str(exc)
        self.preview.setText(f"<b>Where:</b> {text}")

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
    def add_condition(self, cond: Condition | None = None) -> Condition:
        if cond is None:
            aliases = self.win.aliases()
            src = aliases[0] if aliases else ANY_SOURCE
            cols = self.win.columns_for(src)
            cond = Condition(src, cols[0] if cols else "?", "=", "")
        model, gpath, _ = self._target()
        self._node(model, gpath).items.append(cond)
        self.set_group(model, gpath + [len(self._node(model, gpath).items) - 1])
        return cond

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
        self.set_group(model, self._path(self.tree.currentItem()))


class OutputsEditor(QGroupBox):
    """Per source: a checkable, reorderable list of its columns."""

    def __init__(self):
        super().__init__("Output columns")
        lay = QVBoxLayout(self)
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

    @staticmethod
    def _set_all(lst: QListWidget, on: bool):
        for i in range(lst.count()):
            lst.item(i).setCheckState(Qt.Checked if on else Qt.Unchecked)

    def set_checked(self, alias: str, columns: list[str]):
        lst = self.lists[alias]
        for i in range(lst.count()):
            lst.item(i).setCheckState(Qt.Checked if lst.item(i).text() in columns else Qt.Unchecked)

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
        QMessageBox.critical(self, APP, str(msg))

    def aliases(self) -> list[str]:
        return [e.source.alias for e in self.entries]

    def entry(self, alias: str) -> Entry | None:
        return next((e for e in self.entries if e.source.alias == alias), None)

    def columns_for(self, alias: str) -> list[str]:
        if alias == ANY_SOURCE:
            out = []
            for e in self.entries:
                out += [c for c in e.columns if c not in out]
            return out
        e = self.entry(alias)
        return e.columns if e else []

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
        key = _combo(e.columns, e.source.key)
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
        self.outputs.rebuild(self.entries, {k: v for k, v in keep.items() if v is not None})
        self.conditions.rename_source(old, new)

    def _sources_changed(self):
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
            proxy.setSourceModel(FrameModel(df, view))
            view.setModel(proxy)
            view.setSortingEnabled(True)
            view.sortByColumn(-1, Qt.AscendingOrder)
            view.horizontalHeader().setSectionResizeMode(QHeaderView.Interactive)
            view.resizeColumnsToContents()
            self.tabs.addTab(view, f"{name} ({len(df)})")
        self.charts.set_result(result)
        counts = ", ".join(f"{n}: {len(df)} row{'s' if len(df) != 1 else ''}" for n, df in result.tables.items())
        text = f"<b>Matched</b> — {counts}"
        if result.warnings:
            text += "".join(f"<br>⚠ {w}" for w in result.warnings)
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
    win = MainWindow()
    win.show()
    for path in argv:
        if path.lower().endswith(".json"):
            win.load_query(path)
        else:
            win.add_workbook(path)
    return app.exec()
