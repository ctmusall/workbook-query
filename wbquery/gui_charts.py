"""Visualize tab: chart a result table, choosing the chart type and columns."""

from __future__ import annotations

import html
from pathlib import Path

import matplotlib

matplotlib.use("QtAgg")
from matplotlib.backends.backend_qtagg import FigureCanvasQTAgg, NavigationToolbar2QT  # noqa: E402
from matplotlib.figure import Figure  # noqa: E402
import numpy as np  # noqa: E402
from PySide6.QtCore import Qt, QTimer  # noqa: E402
from PySide6.QtGui import QCursor  # noqa: E402
from PySide6.QtWidgets import (  # noqa: E402
    QCheckBox, QComboBox, QDialog, QDialogButtonBox, QFileDialog, QGridLayout, QHBoxLayout, QLabel, QLineEdit,
    QMessageBox, QPushButton, QSpinBox, QToolTip, QVBoxLayout, QWidget,
)

from .charts import (  # noqa: E402
    AGGREGATES, KINDS, MULTI_Y, ChartError, ChartSpec, Dataset, draw, is_date, numeric_columns,
)
from .conditions import sort_columns  # noqa: E402
from .query import QueryResult, is_provenance  # noqa: E402,F401 - is_provenance re-exported

NONE = "(none)"


IMAGE_FILTER = "PNG image (*.png);;SVG image (*.svg);;PDF document (*.pdf)"
DATASETS_TIP = "Overlay more datasets, each with its own X column and Y columns (e.g. Sample Date -> Hand)"


def pick_columns(parent, title: str, prompt: str, columns: list[str], current) -> list[str] | None:
    """Tick columns in a :class:`ColumnPicker`; None when cancelled."""
    from .gui import ColumnPicker  # the app window module imports this one

    rows = [(c, [], Qt.Checked if c in current else Qt.Unchecked) for c in columns]
    dlg = ColumnPicker(title, prompt, rows, parent, show_sheets=False)
    return dlg.checked() if dlg.exec() == QDialog.Accepted else None


class DatasetsDialog(QDialog):
    """Edit the extra datasets of a line / scatter chart: one row per dataset with its X column,
    its Y columns and a Remove button."""

    def __init__(self, datasets: list[Dataset], x_columns: list[str], y_columns: list[str], parent=None):
        super().__init__(parent)
        self.setWindowTitle("More datasets")
        self.resize(520, 300)
        self.x_columns, self.y_columns = x_columns, y_columns
        self.rows: list[tuple[QWidget, QComboBox, QPushButton, list[str]]] = []
        lay = QVBoxLayout(self)
        intro = QLabel("Each dataset is drawn over the chart's own X and Y with its own X column, "
                       "e.g. Sample Date with the Hand columns.")
        intro.setWordWrap(True)
        lay.addWidget(intro)
        self.rows_layout = QVBoxLayout()
        lay.addLayout(self.rows_layout)
        add = QPushButton("Add dataset")
        add.clicked.connect(lambda: self.add_row(Dataset()))
        lay.addWidget(add, 0, Qt.AlignLeft)
        lay.addStretch()
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        lay.addWidget(buttons)
        for ds in datasets:
            self.add_row(ds)
        if not datasets:
            self.add_row(Dataset())

    def add_row(self, ds: Dataset):
        row = QWidget()
        h = QHBoxLayout(row)
        h.setContentsMargins(0, 0, 0, 0)
        x_box = QComboBox()
        x_box.addItem(NONE)
        x_box.addItems(self.x_columns)
        x_box.setCurrentText(ds.x or NONE)
        y_btn = QPushButton()
        remove = QPushButton("Remove")
        h.addWidget(QLabel("X"))
        h.addWidget(x_box, 1)
        h.addWidget(y_btn, 1)
        h.addWidget(remove)
        entry = (row, x_box, y_btn, list(ds.ys))
        self.rows.append(entry)
        self._show_ys(entry)
        y_btn.clicked.connect(lambda: self._pick_ys(entry))
        remove.clicked.connect(lambda: self._remove(entry))
        self.rows_layout.addWidget(row)

    @staticmethod
    def _show_ys(entry):
        _, _, btn, ys = entry
        btn.setText("Y columns…" if not ys else f"Y: {', '.join(ys)}" if len(ys) <= 2 else
                    f"Y: {ys[0]} +{len(ys) - 1}")
        btn.setToolTip(", ".join(ys) or "Choose the Y columns of this dataset")

    def _pick_ys(self, entry):
        chosen = pick_columns(self, "Y columns", "Tick the columns to plot against this dataset's X.",
                              self.y_columns, set(entry[3]))
        if chosen is not None:
            entry[3][:] = chosen
            self._show_ys(entry)

    def _remove(self, entry):
        self.rows.remove(entry)
        entry[0].deleteLater()

    def datasets(self) -> list[Dataset]:
        return [Dataset("" if x.currentText() == NONE else x.currentText(), list(ys))
                for _, x, _, ys in self.rows if ys]


class ChartPanel(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.result: QueryResult | None = None
        self.default_dir = lambda: str(Path.home())  # where the save dialog starts (the window sets this)
        self.saved = lambda path: None  # told about each chart saved
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
        self.extra_y: list[str] = []  # line / scatter: Y columns after the Y box's
        self.more_y_btn = QPushButton("More Y…")
        self.more_y_btn.setToolTip("Plot several Y columns (e.g. every Chest, Sleeve and Hand column)")
        self.datasets: list[Dataset] = []  # line / scatter: overlays, each with its own X
        self.datasets_btn = QPushButton("More datasets…")
        self.datasets_btn.setToolTip(DATASETS_TIP)
        self.merge_check = QCheckBox("Combine numbered columns")
        self.merge_check.setChecked(True)
        self.merge_check.setToolTip("Chest 1, Chest 2 and Chest 1 (2) become one 'Chest' series")
        self.hits_check = QCheckBox("Only cells that met a condition")
        self.labels_check = QCheckBox("Label points with their text")
        self.labels_check.setToolTip("Write the text after each number next to its point "
                                     "(1 Micrococcus luteus -> 'Micrococcus luteus'), up to 60 labels")
        self.hits_check.setToolTip("Plot only the cells a condition counted for (the ones highlighted in "
                                   "the results), e.g. just the non-zero readings")
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
            (None, self.more_y_btn), (None, self.datasets_btn), (None, self.merge_check),
            (None, self.hits_check), (None, self.labels_check),
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
        self.canvas.mpl_connect("motion_notify_event", self._hover)
        # Which point labels fit depends on the canvas size, so redraw (re-picking them) after a resize.
        self.canvas.mpl_connect("resize_event", lambda _e: self.schedule())
        bar = QHBoxLayout()
        bar.addWidget(self.toolbar)
        bar.addStretch()
        bar.addWidget(self.save_btn)
        self.message = QLabel()
        self.message.setWordWrap(True)
        self.message.setTextFormat(Qt.RichText)
        self.message.setOpenExternalLinks(True)

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
        self.merge_check.toggled.connect(self.schedule)
        self.hits_check.toggled.connect(self.schedule)
        self.labels_check.toggled.connect(self.schedule)
        self.more_y_btn.clicked.connect(lambda: self.pick_y())
        self.datasets_btn.clicked.connect(lambda: self.edit_datasets())
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
            ys=list(self.extra_y) if self.kind() in MULTI_Y else [],
            merge_numbered=self.merge_check.isChecked(),
            only_hits=self.hits_check.isChecked() and self.hits_check.isEnabled(),
            point_labels=self.labels_check.isChecked(),
            datasets=[Dataset(d.x, list(d.ys)) for d in self.datasets] if self.kind() in MULTI_Y else [],
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
        self.set_extra_y(spec.ys)
        self.set_datasets(spec.datasets)
        self.merge_check.setChecked(spec.merge_numbered)
        self.hits_check.setChecked(spec.only_hits)
        self.labels_check.setChecked(spec.point_labels)
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
        self.hits_check.setEnabled(bool(result and result.query.where.conditions(active=True)))
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
        self.set_extra_y([c for c in self.extra_y if c in cols + extra])
        self.set_datasets([Dataset(d.x if d.x in cols + extra else "", [c for c in d.ys if c in cols + extra])
                           for d in self.datasets])
        self._fill_required()
        self.schedule()

    def set_extra_y(self, columns: list[str]):
        y = self._value(self.y_box)
        self.extra_y = [c for c in dict.fromkeys(columns) if c and c != y]
        n = len(self.extra_y)
        self.more_y_btn.setText(f"More Y… (+{n})" if n else "More Y…")
        self.more_y_btn.setToolTip("Also plotting: " + ", ".join(self.extra_y) if n else
                                   "Plot several Y columns (e.g. every Chest, Sleeve and Hand column)")
        self.schedule()

    def set_datasets(self, datasets: list[Dataset]):
        """Use ``datasets`` as the overlays (ones without a Y column are dropped)."""
        self.datasets = [Dataset(d.x, list(dict.fromkeys(c for c in d.ys if c))) for d in datasets if any(d.ys)]
        n = len(self.datasets)
        self.datasets_btn.setText(f"More datasets… (+{n})" if n else "More datasets…")
        self.datasets_btn.setToolTip("\n".join(f"{d.x or 'row order'} → {', '.join(d.ys)}" for d in self.datasets)
                                     if n else DATASETS_TIP)
        self.schedule()

    def edit_datasets(self) -> bool:
        k = self._kinds
        cols = k["date"] + k["numeric"] + k["category"]
        dlg = DatasetsDialog(self.datasets, sort_columns(cols) + k["extra"], k["numeric"], self)
        if dlg.exec() != QDialog.Accepted:
            return False
        self.set_datasets(dlg.datasets())
        return True

    def pick_y(self, columns: list[str] | None = None) -> bool:
        """Choose every Y column at once (``columns`` skips the dialog): the first goes in the Y box."""
        if columns is None:
            columns = pick_columns(self, "Y columns", "Tick the columns to plot. With 'Combine numbered columns', "
                                   "Chest 1, Chest 2, Chest 1 (2)… make one 'Chest' line.", self._kinds["numeric"],
                                   set(self.spec().y_columns()))
            if columns is None:
                return False
        if not columns:
            return False
        self._loading = True
        self.y_box.setCurrentText(columns[0])
        self._loading = False
        self.set_extra_y(columns[1:])
        return True

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
                           (None, self.stacked_check, info.stacked),
                           (None, self.more_y_btn, self.kind() in MULTI_Y),
                           (None, self.datasets_btn, self.kind() in MULTI_Y),
                           (None, self.merge_check, self.kind() in MULTI_Y),
                           (None, self.hits_check, self.kind() in MULTI_Y),
                           (None, self.labels_check, self.kind() in MULTI_Y)):
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
            hits = self.result.hits.get(self.table_box.currentText()) if self.result else None
            notes = draw(self.figure, df, self.spec(), hits)
        except ChartError as exc:
            self.canvas.draw_idle()
            self.message.setText(f"<span style='color:#b05000'>⚠ {html.escape(str(exc))}</span>")
            return False
        self.canvas.draw_idle()
        self.message.setText(html.escape(" · ".join(notes)))
        return True

    def hover_text(self, x: float, y: float, radius: float = 8.0) -> str:
        """Tooltip for canvas pixel (``x``, ``y``): every point within ``radius`` pixels (several readings can
        share a date and value), nearest first; '' when there's none."""
        found = []
        for ax in self.figure.axes:
            for xy, tips in getattr(ax, "wbq_points", []):
                if not len(xy):
                    continue
                pix = ax.transData.transform(xy)
                dist = np.hypot(pix[:, 0] - x, pix[:, 1] - y)
                found += [(d, tips[i]) for i, d in enumerate(dist) if d <= radius]
        found.sort(key=lambda f: f[0])
        tips = [t for _, t in found]
        more = len(tips) - 8
        return "\n\n".join(tips[:8]) + (f"\n\n… and {more} more" if more > 0 else "")

    def _hover(self, event):
        text = self.hover_text(event.x, event.y) if event.inaxes is not None else ""
        if text:
            QToolTip.showText(QCursor.pos(), text, self.canvas)
        else:
            QToolTip.hideText()

    def has_error(self) -> bool:
        return self.message.text().startswith("<span")

    def save(self, path: str | None = None) -> Path | None:
        if self.frame() is None:
            return None
        if path is None:
            path, chosen = QFileDialog.getSaveFileName(self, "Save chart", str(Path(self.default_dir()) / "chart.png"),
                                                       IMAGE_FILTER)
            if not path:
                return None
            if Path(path).suffix.lower() not in (".png", ".svg", ".pdf"):  # typed without one: use the filter's
                path = str(Path(path).with_suffix("." + ("svg" if "svg" in chosen else "pdf" if "pdf" in chosen
                                                         else "png")))
        path = Path(path)
        if path.suffix.lower() not in (".png", ".svg", ".pdf"):
            path = path.with_suffix(".png")
        if not self.redraw():
            return None
        try:
            self.figure.savefig(path, dpi=150, bbox_inches="tight")
        except Exception as exc:  # noqa: BLE001 - a windowed app has no console: always say what went wrong
            QMessageBox.critical(self, "Save chart", f"Couldn't write {path}:\n{type(exc).__name__}: {exc}")
            return None
        self.saved(path)
        full = path.resolve()
        self.message.setText(f"Chart saved to <a href=\"{full.parent.as_uri()}\">{html.escape(str(full))}</a>")
        return path
