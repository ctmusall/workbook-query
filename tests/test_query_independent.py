import pytest

from wbquery.conditions import Condition, Group
from wbquery.query import COMBINED, FILE_COL, ROW_COL, SHEET_COL, SOURCE_COL, Query, Source, run_query, validate


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


def test_all_results_table_stacks_sources(two_sources):
    q = Query(
        sources=two_sources,
        where=Group("OR", [Condition("L", "Status", "=", "pass"), Condition("R", "Value", ">=", "6e7")]),
        outputs={"L": ["Lot", "Status"], "R": ["Lot ID", "Value"]},
    )
    res = run_query(q)
    assert list(res.tables) == [COMBINED, "L", "R"]
    allr = res.tables[COMBINED]
    assert allr.columns.tolist() == [SOURCE_COL, FILE_COL, SHEET_COL, ROW_COL, "Lot", "Status", "Lot ID", "Value"]
    assert allr[SOURCE_COL].tolist() == ["L", "L", "L", "R", "R"]
    assert allr["Lot"].iloc[:3].tolist() == ["A1", "A3", 101] and allr["Lot"].iloc[3:].isna().all()
    assert allr["Value"].iloc[3:].tolist() == [7e7, 6e7] and allr["Value"].iloc[:3].isna().all()
    assert SOURCE_COL not in res.tables["L"].columns
    assert [s["matched"] for s in res.sources] == [3, 2]


def test_all_results_without_provenance(two_sources):
    res = run_query(Query(sources=two_sources, outputs={"L": ["Lot"], "R": ["Value"]}, include_provenance=False))
    allr = res.tables[COMBINED]
    assert allr.columns.tolist() == ["Lot", "Value"]
    assert len(allr) == len(res.tables["L"]) + len(res.tables["R"])


def test_condition_columns_match_ignoring_case_and_spaces(two_sources):
    q = Query(
        sources=two_sources,
        where=Group("AND", [Condition("*", "lot id", "=", "A2"), Condition("L", " status ", "=", "fail")]),
        outputs={"L": ["Lot"], "R": ["Value"]},
        include_provenance=False,
    )
    res = run_query(q)
    assert res.tables["L"]["Lot"].tolist()[0] == "A2" and len(res.tables["L"]) == 2  # A2 and the blank lot
    assert res.tables["R"]["Value"].tolist() == [7e7, 6e7]


def test_disabled_group_is_not_validated(two_sources):
    bad = Group("AND", [Condition("L", "Status", ">", "abc"), Condition("*", "Ghost", "=", "1")], enabled=False)
    q = Query(sources=two_sources, where=Group("AND", [bad]), outputs={"L": ["Lot"]})
    assert len(run_query(q).tables["L"]) == 5


def test_sheet_no_condition_applies_to_matches_nothing(two_sources, lots_xlsx):
    owners = Source("O", str(lots_xlsx), "Owners")
    q = Query(sources=two_sources + [owners], where=Group("AND", [Condition("*", "Status", "=", "pass")]),
              outputs={"L": ["Lot"], "O": ["Lot"], "R": ["Value"]})
    res = run_query(q)
    assert len(res.tables["L"]) == 3  # Pass, pass, Pass
    assert len(res.tables["O"]) == 0 and len(res.tables["R"]) == 0  # neither has a Status column
    assert len(res.tables[COMBINED]) == 3
    assert [s["matched"] for s in res.sources] == [3, 0, 0]
    assert res.warnings == ["2 sheets lack every column the conditions test, so they match no rows: R, O"]

    q.where.items[0].enabled = False  # no active conditions: everything is kept, no warning
    res = run_query(q)
    assert len(res.tables["O"]) == 2 and len(res.tables["R"]) == 5 and res.warnings == []
