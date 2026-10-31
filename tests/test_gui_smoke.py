import os

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
pytest.importorskip("PySide6")
pytest.importorskip("pytestqt")

from PySide6.QtWidgets import QMessageBox  # noqa: E402

from wbquery.conditions import ANY_SOURCE, Condition, Group  # noqa: E402
from wbquery.gui import ANY_LABEL, MainWindow  # noqa: E402


@pytest.fixture
def win(qtbot, monkeypatch):
    errors = []
    monkeypatch.setattr(QMessageBox, "critical", lambda *a, **k: errors.append(a[2]))
    w = MainWindow()
    w.errors = errors
    qtbot.addWidget(w)
    return w


def _load(win, lots_xlsx, results_xlsx):
    assert win.add_workbook(lots_xlsx, ["Lots"]) == 1
    assert win.add_workbook(results_xlsx) == 1  # single sheet: no dialog
    assert win.aliases() == ["lots/Lots", "results/Results"]


def test_initial_state(win):
    assert not win.run_btn.isEnabled() and not win.export_xlsx_btn.isEnabled()
    assert win.run() is False and win.export("xlsx", "x") is None
    assert win.conditions.group() == Group()


def test_independent_query_and_export(win, lots_xlsx, results_xlsx, tmp_path):
    _load(win, lots_xlsx, results_xlsx)
    assert win.outputs.outputs()["lots/Lots"] == ["Lot", "Status", "Made", "Notes"]  # all ticked by default

    # (lots.Status = pass) AND (results.Value > 6e7 OR results.Pct < 20%)
    first = win.conditions.add_condition()
    assert first.source == "lots/Lots" and first.column == "Lot"
    item = win.conditions.tree.topLevelItem(0).child(0)
    item._column.setCurrentText("Status")
    item._value.setText("pass")
    win.conditions.select([])
    win.conditions.add_group("OR")
    win.conditions.add_condition(Condition("results/Results", "Value", ">", "6e7"))
    win.conditions.add_condition(Condition("results/Results", "Pct", "<", "20%"))
    tree = win.conditions.group()
    assert tree.describe() == ("lots/Lots › Status = 'pass' AND (results/Results › Value > '6e7' "
                               "OR results/Results › Pct < '20%')")
    assert "Status = 'pass'" in win.conditions.preview.text()

    win.outputs.set_checked("lots/Lots", ["Lot"])
    assert win.run(), win.errors
    assert [win.tabs.tabText(i) for i in range(win.tabs.count())] == ["All results (5)", "lots/Lots (3)",
                                                                     "results/Results (2)"]
    assert win.export_xlsx_btn.isEnabled()
    for fmt in ("xlsx", "html"):
        out = win.export(fmt, str(tmp_path / f"res.{fmt}"))
        assert out is not None and out.stat().st_size > 0
    assert win.errors == []


def test_join_query_save_and_reload(win, lots_xlsx, results_xlsx, tmp_path):
    _load(win, lots_xlsx, results_xlsx)
    assert win.entries[0].source.key == "Lot"  # first column
    win.key_box(1).setCurrentText("Lot ID")
    win.join_radio.setChecked(True)
    assert not win.src_table.isColumnHidden(3)
    win.conditions.add_condition(Condition(ANY_SOURCE, "Status", "=", "Pass"))
    assert win.conditions.tree.topLevelItem(0).child(0)._source.currentText() == ANY_LABEL
    assert win.run(), win.errors
    assert win.tabs.tabText(0) == "Joined (2)"  # A1 and 101

    path = win.save_query(str(tmp_path / "q.json"))
    saved = win.build_query()
    other = MainWindow()
    assert other.load_query(str(path))
    assert other.build_query() == saved
    other.deleteLater()


def test_rename_remove_move(win, lots_xlsx, results_xlsx):
    _load(win, lots_xlsx, results_xlsx)
    win.conditions.add_condition(Condition("lots/Lots", "Status", "=", "Pass"))
    win.conditions.select([])
    win.conditions.add_condition(Condition("results/Results", "Value", ">", "1"))
    assert win.rename_source(0, "L")
    assert win.conditions.group().items[0].source == "L"
    assert "L" in win.outputs.lists
    assert not win.rename_source(1, "L") and win.errors  # duplicate name refused
    assert win.aliases() == ["L", "results/Results"]

    win.conditions.select([1])
    assert win.conditions.move(-1)
    assert [c.column for c in win.conditions.group().items] == ["Value", "Status"]
    assert not win.conditions.move(-1)
    assert win.conditions.remove()
    assert [c.column for c in win.conditions.group().items] == ["Status"]

    assert win.remove_source(1)
    assert win.aliases() == ["L"] and list(win.outputs.lists) == ["L"]


def test_errors_are_reported(win, lots_xlsx, tmp_path):
    bad = tmp_path / "bad.xlsx"
    bad.write_text("not a workbook")
    assert win.add_workbook(bad) == 0 and win.errors
    win.add_workbook(lots_xlsx, ["Lots"])
    win.conditions.add_condition(Condition("lots/Lots", "Status", ">", "abc"))
    assert win.run() is False
    assert "needs a number or date" in win.errors[-1]
    assert win.load_query(str(tmp_path / "missing.json")) is False


def test_visualize(win, lots_xlsx, results_xlsx, tmp_path):
    from wbquery.charts import KINDS

    _load(win, lots_xlsx, results_xlsx)
    assert not win.visualize_btn.isEnabled()
    assert win.run(), win.errors
    assert win.visualize_btn.isEnabled()
    win.visualize_btn.setChecked(True)
    assert win.view_tabs.currentIndex() == 1
    charts = win.charts
    assert [charts.table_box.itemText(i) for i in range(charts.table_box.count())] == \
        ["All results", "lots/Lots", "results/Results"]
    charts.table_box.setCurrentText("results/Results")
    assert charts.spec().y == "Value"  # first numeric column
    for kind in KINDS:
        charts.set_kind(kind)  # a needed X or Group column is picked automatically
        assert charts.redraw(), (kind, charts.message.text())
        assert not charts.has_error()
    charts.set_kind("box")
    charts.y_box.setCurrentText("File")  # not numeric
    assert not charts.redraw() and charts.has_error()
    charts.y_box.setCurrentText("Value")
    for ext in ("png", "svg"):
        out = charts.save(str(tmp_path / f"chart.{ext}"))
        assert out is not None and out.stat().st_size > 0
    assert charts.save(str(tmp_path / "chart")).suffix == ".png"

    win.view_tabs.setCurrentIndex(0)
    assert not win.visualize_btn.isChecked()
    assert win.run()  # re-running keeps the chosen table and chart
    assert charts.table_box.currentText() == "results/Results" and charts.kind() == "box"
    win._clear_results()
    assert not win.visualize_btn.isEnabled() and charts.frame() is None and charts.save("x.png") is None
    assert win.errors == []
