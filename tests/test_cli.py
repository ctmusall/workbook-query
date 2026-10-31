import openpyxl
import pytest

from wbquery import __version__
from wbquery.__main__ import wants_gui
from wbquery.cli import main
from wbquery.conditions import Condition, Group
from wbquery.query import Query, save_query


@pytest.fixture
def query_file(tmp_path, two_sources):
    q = Query(sources=two_sources, mode="join",
              where=Group("AND", [Condition("R", "Value", ">=", "5e7")]),
              outputs={"L": ["Status"], "R": ["Value"]})
    return save_query(q, tmp_path / "q.json")


def test_run(tmp_path, query_file, capsys):
    assert main(["-q", str(query_file), "-o", str(tmp_path / "res"), "--print"]) == 0
    out = capsys.readouterr()
    assert "Joined: 3 row(s)" in out.out and "warning:" in out.err
    assert (tmp_path / "res.xlsx").stat().st_size > 0 and (tmp_path / "res.html").stat().st_size > 0
    assert openpyxl.load_workbook(tmp_path / "res.xlsx")["Joined"].max_row == 4


def test_out_dir_and_formats(tmp_path, query_file):
    assert main(["-q", str(query_file), "-o", str(tmp_path), "--formats", "html"]) == 0
    assert (tmp_path / "query_results.html").exists() and not (tmp_path / "query_results.xlsx").exists()
    assert main(["-q", str(query_file), "--formats", "pdf"]) == 2


def test_source_override(tmp_path, query_file, results_xlsx, capsys):
    assert main(["-q", str(query_file), "--source", "R=missing.xlsx", "-o", str(tmp_path / "x")]) == 1
    assert "R:" in capsys.readouterr().err
    assert main(["-q", str(query_file), "--source", f"R={results_xlsx}", "-o", str(tmp_path / "x")]) == 0
    assert main(["-q", str(query_file), "--source", "Nope=x.xlsx"]) == 1
    assert main(["-q", str(query_file), "--source", "bad"]) == 2


def test_list_and_errors(lots_xlsx, capsys):
    assert main(["--list", str(lots_xlsx)]) == 0
    assert "[Lots] Lot, Status, Made, Notes" in capsys.readouterr().out
    assert main([]) == 2
    with pytest.raises(SystemExit):
        main(["--version"])
    assert __version__ in capsys.readouterr().out


def test_wants_gui():
    assert wants_gui([]) and wants_gui(["a.xlsx", "b.xlsx"])
    assert not wants_gui(["-q", "x.json"])
