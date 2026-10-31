import openpyxl

from wbquery.conditions import Condition, Group
from wbquery.export_html import render_html, write_html
from wbquery.export_xlsx import sheet_name, write_xlsx
from wbquery.query import Query, run_query


def _result(two_sources, mode="independent"):
    return run_query(Query(sources=two_sources, mode=mode,
                           where=Group("AND", [Condition("L", "Status", "=", "Pass")]),
                           outputs={"L": ["Lot", "Made"], "R": ["Value"]}))


def test_xlsx(tmp_path, two_sources):
    path = write_xlsx(_result(two_sources), tmp_path / "out")
    assert path.suffix == ".xlsx"
    wb = openpyxl.load_workbook(path)
    assert wb.sheetnames == ["Query", "L", "R"]
    lots = wb["L"]
    assert [c.value for c in lots[1]] == ["File", "Sheet", "Excel row", "Lot", "Made"]
    assert lots.max_row == 4 and lots.freeze_panes == "A2"
    query = [[c.value for c in r] for r in wb["Query"].iter_rows()]
    assert ["Conditions", "L › Status = 'Pass'"] in [r[:2] for r in query]
    assert ["Rows out: L", 3] in [r[:2] for r in query]
    assert any(r[0] == "L" and len(r[5] or "") == 64 for r in query)


def test_xlsx_join_and_warnings(tmp_path, two_sources):
    path = write_xlsx(_result(two_sources, "join"), tmp_path / "join.xlsx")
    wb = openpyxl.load_workbook(path)
    assert wb.sheetnames == ["Query", "Joined"]
    assert any(r[0].value == "Warning" for r in wb["Query"].iter_rows())


def test_sheet_name():
    taken = {"query"}
    assert sheet_name("book/Sheet1", taken) == "book_Sheet1"
    assert sheet_name("BOOK/Sheet1", taken) == "BOOK_Sheet1 (2)"
    assert len(sheet_name("x" * 40, taken)) == 31
    assert sheet_name("Query", taken) == "Query (2)"


def test_html(tmp_path, two_sources):
    res = _result(two_sources)
    text = render_html(res)
    assert "L › Status = &#x27;Pass&#x27;" in text
    assert "2024-01-05" in text and "<h2>L" in text and "(3 rows)" in text
    path = write_html(res, tmp_path / "r")
    assert path.suffix == ".html" and path.stat().st_size > 1000
