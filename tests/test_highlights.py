"""Leading-number cells, per-condition highlight colors (cell hits) and multi-Y charts."""

import os

import matplotlib

matplotlib.use("Agg")

import openpyxl  # noqa: E402
import pandas as pd  # noqa: E402
import pytest  # noqa: E402
from matplotlib.figure import Figure  # noqa: E402

from wbquery.charts import ChartSpec, draw, series_name  # noqa: E402
from wbquery.conditions import Condition, Group, cell_hits, normalize_color  # noqa: E402
from wbquery.export_html import render_html  # noqa: E402
from wbquery.export_xlsx import write_xlsx  # noqa: E402
from wbquery.loader import cell_number, leading_number  # noqa: E402
from wbquery.query import COMBINED, JOINED, Query, Source, load_query, run_query, save_query  # noqa: E402

RED, BLUE = "#ff0000", "#0000ff"

# Two people per row, as on the EM data sheets: Person N then their readings.
EM = pd.DataFrame({
    "Monitor Date": pd.to_datetime(["2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08"]),
    "Person 1": ["KSG", "ZEN", "KSG", "LST"],
    "Chest 1": [0, 5, "1 Micrococcus luteus", 0],
    "Hand 1": [2, 0, 0, 0],
    "Person 2": ["TJH", "KSG", "ZEN", "KSG"],
    "Chest 1 (2)": [4, 0, 0, 0],
    "Hand 1 (2)": [0, 3, "TNTC", 0],
}, index=[5, 6, 7, 8])  # Excel rows


def _person(n: int, suffix: str, color: str = RED) -> Group:
    return Group("AND", [Condition("*", f"Person {n}", "=", "KSG"),
                         Group("OR", [Condition("*", f"Chest 1{suffix}", "<>", "0", color=color),
                                      Condition("*", f"Hand 1{suffix}", "<>", "0", color=color)])])


KSG = Group("OR", [_person(1, ""), _person(2, " (2)", BLUE)])


def _run(where=KSG, mode="independent", frames=None, outputs=None):
    frames = frames or {"EM": EM}
    sources = [Source(a, f"{a}.xlsx", "Sheet", key="Monitor Date") for a in frames]
    q = Query(sources, where, outputs or {a: list(EM.columns) for a in frames}, mode=mode)
    return run_query(q, frames)


@pytest.fixture(autouse=True)
def _no_hash(monkeypatch):
    monkeypatch.setattr("wbquery.query.file_sha256", lambda p: "")


# --- numbers followed by text -----------------------------------------------------

@pytest.mark.parametrize("text,value", [
    ("1 Micrococcus luteus", 1), ("3 CFU", 3), ("1,200 colonies", 1200), ("5% coverage", 0.05),
    (" 4 mold - see note", 4), ("1.5", 1.5), ("2", 2),
])
def test_cell_number_reads_leading_numbers(text, value):
    assert cell_number(text) == pytest.approx(value)


@pytest.mark.parametrize("text", ["TNTC", "<1", ">300", "approx 2", "2026-01-05", "5/2", "", "n/a"])
def test_cell_number_leaves_other_text(text):
    assert pd.isna(cell_number(text))


def test_leading_number_keeps_the_label():
    assert leading_number("1 Micrococcus luteus") == (1.0, "Micrococcus luteus")
    assert leading_number("12") is None


def test_conditions_order_leading_numbers_but_not_typed_values():
    s = pd.Series(["1 Micrococcus luteus", "1 Bacillus", "TNTC", 0])
    assert Condition("*", "x", ">", "0").matches(s).tolist() == [True, True, False, False]
    assert Condition("*", "x", "=", "1").matches(s).tolist() == [True, True, False, False]
    # A typed value is read strictly: this compares text, not the number 1.
    assert Condition("*", "x", "=", "1 Micrococcus luteus").matches(s).tolist() == [True, False, False, False]
    assert Condition("*", "x", "<>", "0").matches(s).tolist() == [True, True, True, False]


# --- cell hits ------------------------------------------------------------------------

def test_cell_hits_count_only_where_every_group_holds():
    def column_for(c):
        return [EM[c.column]] if c.column in EM.columns else []

    hits = {(cond.column, col): mask.tolist() for cond, col, mask in cell_hits(KSG, column_for)}
    # Row 6: Chest 1 = 5 but Person 1 is ZEN, so it doesn't count; Person 2 KSG with Hand 1 (2) = 3 does.
    assert hits[("Chest 1", "Chest 1")] == [False, False, True, False]
    assert hits[("Hand 1", "Hand 1")] == [True, False, False, False]
    assert hits[("Hand 1 (2)", "Hand 1 (2)")] == [False, True, False, False]
    assert hits[("Chest 1 (2)", "Chest 1 (2)")] == [False, False, False, False]  # row 5's 4 is TJH's
    assert hits[("Person 1", "Person 1")] == [True, False, True, False]


def test_cell_hits_skip_negated_and_disabled():
    def column_for(c):
        return [EM[c.column]]

    negated = Group("AND", [Group("OR", [Condition("*", "Hand 1", "<>", "0")], negate=True)])
    assert cell_hits(negated, column_for) == []
    off = Group("AND", [Condition("*", "Hand 1", "<>", "0", enabled=False), Condition("*", "Person 1", "=", "KSG")])
    assert [c.column for c, _, _ in cell_hits(off, column_for)] == ["Person 1"]


def test_result_hits_and_colors_match_the_table():
    res = _run()
    table, hits, colors = res.tables["EM"], res.hits["EM"], res.colors["EM"]
    assert table["Excel row"].tolist() == [5, 6, 7]  # row 8: Person 2 KSG but every reading 0
    assert list(hits.columns) == list(table.columns) == list(colors.columns)
    assert colors["Chest 1"].tolist() == ["", "", RED]
    assert colors["Hand 1"].tolist() == [RED, "", ""]
    assert colors["Hand 1 (2)"].tolist() == ["", BLUE, ""]
    assert colors["Chest 1 (2)"].tolist() == ["", "", ""]
    assert hits["Person 1"].tolist() == [True, False, True] and colors["Person 1"].tolist() == ["", "", ""]
    assert not hits["Excel row"].any()


def test_first_colored_condition_wins():
    where = Group("AND", [Condition("*", "Hand 1", ">", "1", color=BLUE), Condition("*", "Hand 1", "<>", "0",
                                                                                     color=RED)])
    assert _run(where).colors["EM"]["Hand 1"].tolist() == [BLUE]


def test_combined_and_join_tables_carry_colors():
    res = _run(frames={"A": EM, "B": EM.iloc[:2]})
    assert res.colors[COMBINED]["Hand 1"].tolist() == [RED, "", "", RED, ""]
    assert res.hits[COMBINED].shape == res.tables[COMBINED].shape
    res = _run(mode="join")
    assert res.colors[JOINED]["EM.Hand 1 (2)"].tolist() == ["", BLUE, ""]


def test_color_saved_only_when_set(tmp_path):
    q = Query([Source("EM", "em.xlsx", "Sheet")], KSG, {"EM": ["Hand 1"]})
    back = load_query(save_query(q, tmp_path / "q.json"))
    assert back.where.conditions()[1].color == RED
    plain = Group("AND", [Condition("*", "Hand 1", "<>", "0")])
    assert "color" not in str(plain.to_dict())
    assert normalize_color("FF00AA") == "#ff00aa"
    with pytest.raises(ValueError):
        Condition("*", "x", "=", "1", color="red")


def test_exports_fill_colored_cells(tmp_path):
    res = _run()
    wb = openpyxl.load_workbook(write_xlsx(res, tmp_path / "out.xlsx"))
    ws = wb["EM"]
    header = [c.value for c in ws[1]]
    hand = header.index("Hand 1") + 1
    assert ws.cell(2, hand).fill.fgColor.rgb.endswith("FF0000")
    assert ws.cell(3, hand).fill.fill_type is None
    html = render_html(res)
    assert 'style="background:#ff0000;color:#ffffff">2<' in html
    assert 'style="background:#0000ff' in html


# --- charts -------------------------------------------------------------------------------

def test_series_name():
    assert [series_name(c) for c in ("Chest 1", "Chest 1 (2)", "Hand 2 (6)", "Sleeve", "P10")] == \
        ["Chest", "Chest", "Hand", "Sleeve", "P"]


def _lines(fig):
    return {line.get_label(): list(line.get_ydata()) for line in fig.axes[0].get_lines()}


def test_merged_series_and_only_hits():
    res = _run()
    df, hits = res.tables["EM"], res.hits["EM"]
    spec = ChartSpec("line", x="Monitor Date", y="Chest 1", ys=["Hand 1", "Chest 1 (2)", "Hand 1 (2)"])
    fig = Figure()
    draw(fig, df, spec, hits)
    assert not fig.axes[0].texts  # labels are opt-in
    spec.point_labels = True
    notes = draw(fig, df, spec, hits)
    assert set(_lines(fig)) == {"Chest", "Hand"}
    assert "1 non-numeric value in 'Hand 1 (2)' skipped" in notes  # TNTC
    assert [t.get_text() for t in fig.axes[0].texts] == ["Micrococcus luteus"]

    spec.only_hits = True
    notes = draw(fig, df, spec, hits)
    assert _lines(fig) == {"Chest": [1.0], "Hand": [2.0, 3.0]}
    assert not any("skipped" in n for n in notes)  # TNTC wasn't a hit

    spec.merge_numbered = False
    draw(fig, df, spec, hits)
    assert set(_lines(fig)) == {"Chest 1", "Hand 1", "Chest 1 (2)", "Hand 1 (2)"}


def test_only_hits_with_nothing_left_says_so():
    df = EM.reset_index(drop=True)
    hits = pd.DataFrame(False, index=df.index, columns=df.columns)
    with pytest.raises(Exception, match="cells the conditions matched"):
        draw(Figure(), df, ChartSpec("line", y="Hand 1", only_hits=True), hits)


def test_single_y_line_unchanged():
    fig = Figure()
    draw(fig, EM.reset_index(drop=True), ChartSpec("line", x="Monitor Date", y="Hand 1"))
    assert [line.get_label() for line in fig.axes[0].get_lines()][0].startswith("_")  # no legend label


# --- GUI ------------------------------------------------------------------------------------

def test_gui_colors_and_multi_y(qtbot, tmp_path, monkeypatch):
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    pytest.importorskip("PySide6")
    from PySide6.QtCore import Qt
    from PySide6.QtWidgets import QMessageBox

    from wbquery.gui import MainWindow

    monkeypatch.setattr(QMessageBox, "critical", lambda *a, **k: None)
    path = tmp_path / "em.xlsx"
    EM.to_excel(path, sheet_name="Cleanroom A", index=False)
    win = MainWindow()
    qtbot.addWidget(win)
    assert win.add_workbook(path) == 1
    win.conditions.set_group(KSG)
    group = win.conditions.tree.topLevelItem(0).child(0)
    assert group._color.color() == ""  # Person = KSG has no color, the readings do
    inner = group.child(1)
    assert inner._color.color() == RED
    inner._color._apply("#00ff00")
    assert [c.color for c in win.conditions.group().items[0].items[1].items] == ["#00ff00", "#00ff00"]
    assert win.run()

    view = win.tabs.widget(0)
    model = view.model().sourceModel()
    col = list(model.df.columns).index("Hand 1")
    assert model.data(model.index(0, col), Qt.BackgroundRole).name() == "#00ff00"
    assert model.data(model.index(1, col), Qt.BackgroundRole) is None

    charts = win.charts
    charts.set_kind("line")
    charts.x_box.setCurrentText("Monitor Date")
    assert charts.pick_y(["Chest 1", "Hand 1", "Chest 1 (2)", "Hand 1 (2)"])
    assert charts.spec().y_columns() == ["Chest 1", "Hand 1", "Chest 1 (2)", "Hand 1 (2)"]
    assert charts.more_y_btn.text() == "More Y… (+3)"
    charts.hits_check.setChecked(True)
    assert charts.redraw(), charts.message.text()
    assert _lines(charts.figure) == {"Chest": [1.0], "Hand": [2.0, 3.0]}


def test_operators_and_tags_show_literally(qtbot, monkeypatch):
    """The Where line, status line and error dialogs are rich text, so condition text is escaped."""
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    pytest.importorskip("PySide6")
    from PySide6.QtGui import QTextDocument
    from PySide6.QtWidgets import QMessageBox

    from wbquery.gui import MainWindow

    shown = []
    monkeypatch.setattr(QMessageBox, "critical", lambda *a, **k: shown.append(a[2]))
    win = MainWindow()
    qtbot.addWidget(win)

    def plain(rich: str) -> str:
        doc = QTextDocument()
        doc.setHtml(rich)
        return doc.toPlainText()

    win.conditions.set_group(Group("AND", [Condition("*", "Chest 1", "<>", "0"),
                                           Condition("*", "Note", "=", "<b>x</b>")]))
    assert "Chest 1 <> '0'" in plain(win.conditions.preview.text())
    assert "'<b>x</b>'" in plain(win.conditions.preview.text())
    win.error("Any sheet › Chest 1 <> 'x': needs a value\nsecond line")
    assert plain(shown[-1]) == "Any sheet › Chest 1 <> 'x': needs a value\nsecond line"


def test_crowded_labels_never_overlap():
    n = 60
    df = pd.DataFrame({"Date": pd.date_range("2026-01-01", periods=n),
                       "Hand 1": [f"1 Staphylococcus species{i}" for i in range(n)],
                       "Hand 2": ["4 Micrococcus luteus"] + [0] * (n - 1)})
    fig = Figure(figsize=(8, 4))
    notes = draw(fig, df, ChartSpec("scatter", x="Date", y="Hand 1", ys=["Hand 2"], point_labels=True))
    ax = fig.axes[0]
    renderer = fig._get_renderer()
    boxes = [t.get_window_extent(renderer) for t in ax.texts]
    assert 0 < len(boxes) < n
    assert not any(a.overlaps(b) for i, a in enumerate(boxes) for b in boxes[i + 1:])
    assert ax.texts[0].get_text() == "Micrococcus luteus"  # highest value labeled first
    assert any("hidden so labels don't overlap" in note for note in notes)


def test_hover_lists_points_under_the_cursor(qtbot):
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    pytest.importorskip("PySide6")
    from wbquery.gui_charts import ChartPanel

    panel = ChartPanel()
    qtbot.addWidget(panel)
    df = EM.reset_index(drop=True)
    draw(panel.figure, df, ChartSpec("scatter", x="Monitor Date", y="Chest 1", ys=["Hand 1"]))
    panel.canvas.draw()
    ax = panel.figure.axes[0]
    xy = ax.wbq_points[0][0][2]  # Chest 1 on 2026-01-07: "1 Micrococcus luteus"
    x, y = ax.transData.transform(xy)
    text = panel.hover_text(x, y)
    assert "1 Micrococcus luteus" in text and "Monitor Date: 2026-01-07" in text and "Chest" in text
    assert panel.hover_text(x + 200, y + 200) == ""
