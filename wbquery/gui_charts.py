"""Visualize tab: chart a result table, choosing the chart type and columns."""

from __future__ import annotations

from pathlib import Path

import matplotlib

matplotlib.use("QtAgg")
from matplotlib.backends.backend_qtagg import FigureCanvasQTAgg, NavigationToolbar2QT  # noqa: E402
from matplotlib.figure import Figure  # noqa: E402
from PySide6.QtCore import QTimer  # noqa: E402
from PySide6.QtWidgets import (  # noqa: E402
    QCheckBox, QComboBox, QFileDialog, QGridLayout, QHBoxLayout, QLabel, QLineEdit, QMessageBox, QPushButton,
    QSpinBox, QVBoxLayout, QWidget,
)

from .charts import AGGREGATES, KINDS, ChartError, ChartSpec, draw, is_date, numeric_columns  # noqa: E402
from .conditions import sort_columns  # noqa: E402
from .query import QueryResult, is_provenance  # noqa: E402,F401 - is_provenance re-exported

NONE = "(none)"


IMAGE_FILTER = "PNG image (*.png);;SVG image (*.svg);;PDF document (*.pdf)"


class ChartPanel(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.result: QueryResult | None = None
        self._loading = False
        self._kinds: dict[str, list[str]] = {"numeric": [], "date": [], "category": [], "extra": []}
        self._redraw_timer = QTimer(self)
        self._redraw_timer.setSingleShot(True)
        self._redraw_timer.setInterval(0)
        self._redraw_timer.timeout.connect(self.redraw)

        self.table_box = QComboBox()
        self.kind_box = QComboBox()
        for kind, info in KINDS.items():
            self.kind_box.addItem(info.label, kind)
        self.x_box, self.y_box, self.group_box = QComboBox(), QComboBox(), QComboBox()
        self.agg_box = QComboBox()
        self.agg_box.addItems(AGGREGATES)
        self.agg_box.setCurrentText("mean")
        self.bins_spin = QSpinBox()
        self.bins_spin.setRange(1, 500)
        self.bins_spin.setValue(20)
        self.stacked_check = QCheckBox("Stacked")
        self.title_edit = QLineEdit()
        self.title_edit.setPlaceholderText("optional")
        self.save_btn = QPushButton("Save chart…")

        grid = QGridLayout()
        self.x_label, self.group_label = QLabel("X"), QLabel("Group by")
        self.y_label, self.agg_label, self.bins_label = QLabel("Y"), QLabel("Aggregate"), QLabel("Bins")
        cells = [
            (QLabel("Table"), self.table_box), (QLabel("Chart"), self.kind_box), (QLabel("Title"), self.title_edit),
            (self.x_label, self.x_box), (self.y_label, self.y_box), (self.group_label, self.group_box),
            (self.agg_label, self.agg_box), (self.bins_label, self.bins_spin), (None, self.stacked_check),
        ]
        for i, (lab, w) in enumerate(cells):
            r, c = divmod(i, 3)
            if lab is not None:
                grid.addWidget(lab, r, c * 2)
            grid.addWidget(w, r, c * 2 + 1)
        for c in (1, 3, 5):
            grid.setColumnStretch(c, 1)

        self.figure = Figure(figsize=(8, 5), dpi=100)
        self.canvas = FigureCanvasQTAgg(self.figure)
        self.toolbar = NavigationToolbar2QT(self.canvas, self)
        bar = QHBoxLayout()
        bar.addWidget(self.toolbar)
        bar.addStretch()
        bar.addWidget(self.save_btn)
        self.message = QLabel()
        self.message.setWordWrap(True)

        lay = QVBoxLayout(self)
        lay.addLayout(grid)
        lay.addLayout(bar)
        lay.addWidget(self.canvas, 1)
        lay.addWidget(self.message)

        self.table_box.currentTextChanged.connect(self._table_changed)
        self.kind_box.currentIndexChanged.connect(self._kind_changed)
        for box in (self.x_box, self.y_box, self.group_box, self.agg_box):
            box.currentTextChanged.connect(self.schedule)
        self.bins_spin.valueChanged.connect(self.schedule)
        self.stacked_check.toggled.connect(self.schedule)
        self.title_edit.textChanged.connect(self.schedule)
        self.save_btn.clicked.connect(lambda: self.save())
        self._kind_changed()
        self.set_result(None)

    # --- state -------------------------------------------------------------------
    def kind(self) -> str:
        return self.kind_box.currentData()

    def set_kind(self, kind: str):
        self.kind_box.setCurrentIndex(list(KINDS).index(kind))

    def frame(self):
        if self.result is None:
            return None
        return self.result.tables.get(self.table_box.currentText())

    @staticmethod
    def _value(box: QComboBox) -> str:
        return "" if box.currentText() == NONE else box.currentText()

    def spec(self) -> ChartSpec:
        info = KINDS[self.kind()]
        return ChartSpec(
            kind=self.kind(),
            x=self._value(self.x_box) if info.x != "none" else "",
            y=self._value(self.y_box),
            group=self._value(self.group_box) if info.group != "none" else "",
            agg=self.agg_box.currentText(),
            bins=self.bins_spin.value(),
            stacked=self.stacked_check.isChecked(),
            title=self.title_edit.text().strip(),
        )

    def set_spec(self, spec: ChartSpec):
        self._loading = True
        self.set_kind(spec.kind)
        for box, val in ((self.x_box, spec.x), (self.y_box, spec.y), (self.group_box, spec.group)):
            box.setCurrentText(val or NONE)
        self.agg_box.setCurrentText(spec.agg)
        self.bins_spin.setValue(spec.bins)
        self.stacked_check.setChecked(spec.stacked)
        self.title_edit.setText(spec.title)
        self._loading = False
        self.redraw()

    def set_result(self, result: QueryResult | None):
        """Show ``result``'s tables, keeping the chosen table and columns where they still exist."""
        self.result = result
        keep = self.table_box.currentText()
        self._loading = True
        self.table_box.clear()
        if result is not None:
            self.table_box.addItems(list(result.tables))
            if keep in result.tables:
                self.table_box.setCurrentText(keep)
        self._loading = False
        self._table_changed()
        has = bool(result and result.tables)
        for w in (self.save_btn, self.toolbar):
            w.setEnabled(has)

    # --- reacting to edits -----------------------------------------------------------
    def _fill(self, box: QComboBox, items: list[str], optional: bool, default: str):
        keep = box.currentText()
        box.blockSignals(True)
        box.clear()
        if optional:
            box.addItem(NONE)
        extra = [c for c in items if is_provenance(c)]  # file / sheet / row stay last
        box.addItems(sort_columns(c for c in items if c not in extra) + extra)
        if keep and keep != NONE and box.findText(keep) >= 0:
            box.setCurrentText(keep)
        elif default and box.findText(default) >= 0:
            box.setCurrentText(default)
        box.blockSignals(False)

    def _table_changed(self, *_):
        if self._loading:
            return
        df = self.frame()
        cols = [] if df is None else [str(c) for c in df.columns]
        extra = [c for c in cols if is_provenance(c)]
        cols = [c for c in cols if c not in extra]
        nums = [] if df is None else [c for c in numeric_columns(df) if c in cols]
        others = [c for c in cols if c not in nums]
        dates = [c for c in others if is_date(df[c])]
        categories = [c for c in others if c not in dates]
        self._kinds = {"numeric": nums, "date": dates, "category": categories, "extra": extra}
        info = KINDS[self.kind()]
        self._fill(self.y_box, nums + others + extra, info.y == "optional", nums[0] if nums else "")
        self._fill(self.x_box, cols + extra, True, (dates or categories or [""])[0])
        self._fill(self.group_box, categories + dates + nums + extra, True, "")
        self._fill_required()
        self.schedule()

    def _fill_required(self):
        """Pick a column for an X or Group the chart type needs but that is still '(none)'."""
        info, k = KINDS[self.kind()], self._kinds
        y = self._value(self.y_box)
        if info.x == "required" and not self._value(self.x_box):
            order = k["date"] + k["numeric"] if self.kind() == "scatter" else k["category"] + k["date"]
            choice = next((c for c in order if c != y), "")
            if choice:
                self.x_box.setCurrentText(choice)
        if info.group == "required" and not self._value(self.group_box):
            x = self._value(self.x_box)
            choice = next((c for c in k["category"] + k["date"] + k["extra"] + k["numeric"] if c not in (x, y)), "")
            if choice:
                self.group_box.setCurrentText(choice)

    def _kind_changed(self, *_):
        info = KINDS[self.kind()]
        self.x_label.setText(info.x_label)
        self.group_label.setText(info.group_label)
        for lab, w, on in ((self.x_label, self.x_box, info.x != "none"),
                           (self.group_label, self.group_box, info.group != "none"),
                           (self.agg_label, self.agg_box, info.agg), (self.bins_label, self.bins_spin, info.bins),
                           (None, self.stacked_check, info.stacked)):
            w.setVisible(on)
            if lab is not None:
                lab.setVisible(on)
        self.y_label.setText("Y (blank: count rows)" if info.y == "optional" else "Y")
        # Y may be left blank only where rows can be counted.
        if info.y == "optional" and self.y_box.findText(NONE) < 0:
            self.y_box.insertItem(0, NONE)
        elif info.y == "required" and (i := self.y_box.findText(NONE)) >= 0:
            self.y_box.removeItem(i)
        self._fill_required()
        self.schedule()

    def schedule(self, *_):
        if not self._loading:
            self._redraw_timer.start()

    # --- drawing and saving ----------------------------------------------------------
    def redraw(self) -> bool:
        """Draw the chart; returns False (and shows why) when it can't be drawn."""
        self._redraw_timer.stop()
        df = self.frame()
        if df is None:
            self.figure.clear()
            self.canvas.draw_idle()
            self.message.setText("Run a query to chart its results.")
            return False
        try:
            notes = draw(self.figure, df, self.spec())
        except ChartError as exc:
            self.canvas.draw_idle()
            self.message.setText(f"<span style='color:#b05000'>⚠ {exc}</span>")
            return False
        self.canvas.draw_idle()
        self.message.setText(" · ".join(notes))
        return True

    def has_error(self) -> bool:
        return self.message.text().startswith("<span")

    def save(self, path: str | None = None) -> Path | None:
        if self.frame() is None:
            return None
        if path is None:
            path, _ = QFileDialog.getSaveFileName(self, "Save chart", "chart.png", IMAGE_FILTER)
            if not path:
                return None
        path = Path(path)
        if path.suffix.lower() not in (".png", ".svg", ".pdf"):
            path = path.with_suffix(".png")
        if not self.redraw():
            return None
        try:
            self.figure.savefig(path, dpi=150, bbox_inches="tight")
        except (OSError, ValueError) as exc:
            QMessageBox.critical(self, "Save chart", f"Couldn't write {path}:\n{exc}")
            return None
        return path
