import pytest

from wbquery.conditions import Condition, Group
from wbquery.query import FILE_COL, ROW_COL, SHEET_COL, Query, Source, run_query, validate


def test_each_source_filtered_by_its_own_conditions(two_sources):
    q = Query(
        sources=two_sources,
        where=Group("OR", [Condition("L", "Status", "=", "pass"), Condition("R", "Value", ">=", "6e7")]),
        outputs={"L": ["Lot", "Status"], "R": ["Lot ID", "Value"]},
    )
    res = run_query(q)
    lots, results = res.tables["L"], res.tables["R"]
    assert lots["Lot"].tolist() == ["A1", "A3", 101]
    assert lots[ROW_COL].tolist() == [2, 4, 5]  # Excel rows (header is row 1)
    assert set(lots[FILE_COL]) == {"lots.xlsx"} and set(lots[SHEET_COL]) == {"Lots"}
    assert results["Value"].tolist() == [7e7, 6e7]
    assert [s["matched"] for s in res.sources] == [3, 2]
    assert all(len(s["sha256"]) == 64 for s in res.sources)


def test_any_sheet_condition_and_unfiltered_source(two_sources, lots_xlsx):
    owners = Source("O", str(lots_xlsx), "Owners")
    q = Query(
        sources=two_sources + [owners],
        where=Group("AND", [Condition("*", "Lot", "=", "A2"), Condition("R", "Pct", ">", "85%")]),
        outputs={"L": ["Notes"], "O": ["Owner"], "R": ["Pct"]},
        include_provenance=False,
    )
    res = run_query(q)
    assert res.tables["L"].columns.tolist() == ["Notes"]
    assert res.tables["L"]["Notes"].tolist() == ["retest"]
    assert res.tables["O"]["Owner"].tolist() == ["Lee"]
    assert res.tables["R"]["Pct"].tolist() == ["90%"]  # R has no "Lot" column, only its own condition applies


def test_no_conditions_keeps_all_rows_and_source_without_outputs_is_skipped(two_sources):
    res = run_query(Query(sources=two_sources, outputs={"L": ["Lot"]}))
    assert list(res.tables) == ["L"]
    assert len(res.tables["L"]) == 5


def test_validation_messages(two_sources):
    cols = {"L": ["Lot", "Status"], "R": ["Lot ID"]}
    q = Query(
        sources=two_sources,
        where=Group("AND", [Condition("L", "Nope", "=", "1"), Condition("Q", "Lot", "=", "1"),
                            Condition("*", "Ghost", "=", "1"), Condition("L", "Status", ">", "abc")]),
        outputs={"L": ["Missing"]},
    )
    errors = "\n".join(validate(q, cols))
    assert "column 'Nope' not found in L" in errors
    assert "no source named 'Q'" in errors
    assert "no sheet has a column 'Ghost'" in errors
    assert "needs a number or date" in errors
    assert "output column(s) not found: Missing" in errors
    assert "Choose at least one output column" in "\n".join(validate(Query(sources=two_sources), cols))
    with pytest.raises(ValueError, match="Nope"):
        run_query(q)
