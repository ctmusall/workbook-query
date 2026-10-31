import os

import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
pytest.importorskip("PySide6")
pytest.importorskip("pytestqt")

from PySide6.QtCore import Qt  # noqa: E402
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
    assert first.source == ANY_SOURCE and first.column == "Lot"  # several sheets: any sheet by default
    item = win.conditions.tree.topLevelItem(0).child(0)
    item._source.setCurrentText("lots/Lots")
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


def test_shared_columns_and_quick_add(win, lots_xlsx, results_xlsx):
    assert win.add_workbook(lots_xlsx, ["Lots", "Owners"]) == 2
    assert win.add_workbook(results_xlsx) == 1
    assert win.shared_columns() == [("Lot", 2)]
    assert win.sheets_with(" lot ") == ["lots/Lots", "lots/Owners"]  # ignores case and spaces
    assert win.conditions.add_shared_btn.isEnabled()
    dlg = win.conditions.shared_picker()
    assert list(dlg.states()) == ["Lot"] and dlg.checked() == []
    dlg._set_shown(Qt.Checked)
    assert dlg.checked() == ["Lot"]
    dlg.deleteLater()
    cond = win.conditions.add_shared("Lot")
    assert cond.source == ANY_SOURCE and cond.column == "Lot"
    item = win.conditions.tree.topLevelItem(0).child(0)
    assert item._source.currentText() == ANY_LABEL
    assert item._column.itemData(item._column.findText("Lot"), 3) == "In: lots/Lots, lots/Owners"  # ToolTipRole


def test_smart_default_duplicate_and_values(win, lots_xlsx, results_xlsx):
    _load(win, lots_xlsx, results_xlsx)
    assert not win.conditions.add_shared_btn.isEnabled()  # no column in both sheets
    win.conditions.add_condition(Condition("results/Results", "Pct", "<", "20%"))
    second = win.conditions.add_condition()  # copies the selected condition's sheet and column
    assert (second.source, second.column, second.op, second.value) == ("results/Results", "Pct", "=", "")
    win.conditions.select([0])
    assert win.conditions.duplicate()
    assert [(c.column, c.op, c.value) for c in win.conditions.group().items] == \
        [("Pct", "<", "20%"), ("Pct", "<", "20%"), ("Pct", "=", "")]
    win.conditions.select([])
    assert not win.conditions.duplicate()  # the root group can't be copied

    assert win.values_for("lots/Lots", "Status") == ["Fail", "Pass", "pass"]
    assert win.values_for(ANY_SOURCE, "lot") == ["101", "A1", "A2", "A3"]
    assert win.values_for("results/Results", "Value")[:2] == ["10000000", "30000000"]  # numbers sort by value
    item = win.conditions.tree.topLevelItem(0).child(0)
    assert item._value.completer().model().stringList() == ["10%", "30%", "50%", "80%", "90%"]


def test_conditions_from_result_cells(win, lots_xlsx, results_xlsx):
    from wbquery.query import FILE_COL

    assert win.add_workbook(lots_xlsx, ["Lots", "Owners"]) == 2
    assert win.run(), win.errors
    assert win.condition_target(FILE_COL) is None
    assert win.condition_target("Lot") == (ANY_SOURCE, "Lot")
    assert win.condition_target("Status") == ("lots/Lots", "Status")
    conds = win.cell_conditions("Status", "Pass")
    assert [c.describe() for c in conds] == ["lots/Lots › Status = 'Pass'", "lots/Lots › Status <> 'Pass'"]
    assert [c.op for c in win.cell_conditions("Notes", None)] == ["is blank", "is not blank"]
    assert win.cell_conditions("Lot", 101.0)[0].value == "101"
    win.add_condition_from_result(conds[0])
    assert win.conditions.group().items == [conds[0]]
    assert win.run() and win.tabs.tabText(1) == "lots/Lots (3)"  # = ignores case: Pass, pass, Pass

    win.join_radio.setChecked(True)
    assert win.run(), win.errors
    assert win.condition_target("lots/Owners.Owner") == ("lots/Owners", "Owner")
    assert win.condition_target("Key") is None
    assert win.errors == []


def test_add_several_shared_conditions(win, lots_xlsx, results_xlsx):
    _load(win, lots_xlsx, results_xlsx)
    win.conditions.add_condition(Condition("lots/Lots", "Status", "=", "Pass"))
    win.conditions.select([])
    added = win.conditions.add_shared_many(["Lot", "Value"])
    assert [(c.source, c.column) for c in added] == [(ANY_SOURCE, "Lot"), (ANY_SOURCE, "Value")]
    assert [c.column for c in win.conditions.group().items] == ["Status", "Lot", "Value"]
    assert win.conditions.add_shared_many([]) == []


def test_outputs_for_all_sheets(win, lots_xlsx, results_xlsx):
    assert win.add_workbook(lots_xlsx, ["Lots", "Owners"]) == 2
    assert win.add_workbook(results_xlsx) == 1
    out = win.outputs
    assert out.all_sheets_btn.isEnabled()
    out.set_checked("lots/Owners", ["Owner"])  # Lot now ticked in Lots only
    states = {c: (sheets, st) for c, sheets, st in out.column_states()}
    assert states["Lot"] == (["lots/Lots", "lots/Owners"], Qt.PartiallyChecked)
    assert states["Owner"] == (["lots/Owners"], Qt.Checked)
    assert states["Value"] == (["results/Results"], Qt.Checked)

    dlg = out.all_sheets_picker()
    dlg._set_shown(Qt.Unchecked)
    dlg.search.setText("lot")  # filters to Lot and Lot ID
    assert [i.data(Qt.UserRole) for i in dlg._items() if not i.isHidden()] == ["Lot", "Lot ID"]
    dlg._set_shown(Qt.Checked)
    out.apply_states(dlg.states())
    dlg.deleteLater()
    assert out.outputs() == {"lots/Lots": ["Lot"], "lots/Owners": ["Lot"], "results/Results": ["Lot ID"]}

    out.apply_states({"lot": Qt.PartiallyChecked, "status": Qt.Checked})  # partial: left alone
    assert out.outputs()["lots/Lots"] == ["Lot", "Status"]


def test_add_for_column_with_expression(win, lots_xlsx, results_xlsx):
    assert win.add_workbook(lots_xlsx, ["Lots", "Owners"]) == 2
    assert win.add_workbook(results_xlsx) == 1
    dlg = win.conditions.shared_picker()
    assert dlg.problem() == "Tick at least one column."
    dlg.set_state("Lot", Qt.Checked)
    assert dlg.value_edit.completer().model().stringList() == ["101", "A1", "A2", "A3"]  # from the ticked columns
    dlg.op_box.setCurrentText(">")
    dlg.value_edit.setText("abc")
    dlg.accept()
    assert dlg.result() == 0 and "needs a number or date" in dlg.error_label.text()  # stays open
    dlg.op_box.setCurrentText("is blank")
    assert not dlg.value_edit.isEnabled() and dlg.expression() == ("is blank", "")
    dlg.op_box.setCurrentText("in")
    dlg.value_edit.setText("A1, A2")
    dlg.place_box.setCurrentText("a new OR group (any column matches)")
    assert dlg.problem() == "" and dlg.group_logic() == "OR"
    dlg.deleteLater()

    op, value = dlg.expression()
    added = win.conditions.add_shared_many(["Lot", "Owner"], op, value, dlg.group_logic())
    assert [(c.column, c.op, c.value) for c in added] == [("Lot", "in", "A1, A2"), ("Owner", "in", "A1, A2")]
    root = win.conditions.group()
    assert len(root.items) == 1 and root.items[0].logic == "OR" and root.items[0].items == added
    assert win.conditions._path(win.conditions.tree.currentItem()) == [0, 0]

    win.conditions.select([])
    win.conditions.add_shared_many(["Lot"], "is not blank")  # straight into the selected (root) group
    assert [type(i).__name__ for i in win.conditions.group().items] == ["Group", "Condition"]
    assert win.run(), win.errors


def test_group_name_toggle_and_bulk_edit(win, lots_xlsx, results_xlsx):
    _load(win, lots_xlsx, results_xlsx)
    ed = win.conditions
    ed.add_condition(Condition("lots/Lots", "Status", "=", "Pass"))
    ed.select([])
    ed.add_group("OR")
    ed.add_condition(Condition(ANY_SOURCE, "Lot", "=", "A1"))
    ed.add_condition(Condition("results/Results", "Lot ID", "=", "A1"))
    group = ed.tree.topLevelItem(0).child(1)
    group._name.setText("Lot A1")
    assert ed.group().items[1].name == "Lot A1"

    # Bulk edit shows only while collapsed and while every condition shares the operator and value.
    assert group._bulk_op.isHidden() and group._mixed.isHidden()
    group.setExpanded(False)
    assert not group._bulk_op.isHidden() and group._bulk_value.text() == "A1"
    group._bulk_op.setCurrentText("in")
    group._bulk_value.setText("A1, A2")
    assert [(c.op, c.value) for c in ed.group().items[1].items] == [("in", "A1, A2"), ("in", "A1, A2")]
    assert group._bulk_value.completer().model().stringList()[:3] == ["101", "A1", "a1"]
    group.setExpanded(True)
    assert group._bulk_op.isHidden()
    group.child(0)._value.setText("A2")
    group.setExpanded(False)
    assert group._bulk_op.isHidden() and not group._mixed.isHidden()

    # Collapsed state survives a rebuild and a save.
    assert ed.group().items[1].collapsed
    ed.refresh()
    assert not ed.tree.topLevelItem(0).child(1).isExpanded()
    group = ed.tree.topLevelItem(0).child(1)
    group.setExpanded(True)

    # Switching a group off greys its contents and leaves it out of the query.
    group._on.setChecked(False)
    assert not group.child(0)._value.isEnabled() and not group._name.isEnabled() and group._on.isEnabled()
    assert ed.group().items[1].enabled is False
    assert "A1" not in ed.preview.text()
    assert win.run(), win.errors
    assert win.tabs.tabText(1) == "lots/Lots (3)"  # only Status = Pass applies
    group._on.setChecked(True)
    assert group.child(0)._value.isEnabled()

    # Adding into a collapsed group opens it to show the new condition.
    group.setExpanded(False)
    ed.select([1])
    ed.add_condition()
    assert ed.tree.topLevelItem(0).child(1).isExpanded()
    ed.tree.topLevelItem(0).child(1).setExpanded(False)
    ed.select([1, 2])  # selection inside a collapsed group: a rebuild keeps it shut and selects the group
    ed.refresh()
    assert not ed.tree.topLevelItem(0).child(1).isExpanded()
    assert ed._path(ed.tree.currentItem()) == [1]


def test_app_icon_is_packaged():
    from PySide6.QtGui import QIcon

    from wbquery.gui import ICON

    assert ICON.exists() and ICON.with_suffix(".ico").exists()
    assert not QIcon(str(ICON)).isNull()
