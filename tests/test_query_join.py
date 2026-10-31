import pandas as pd

from wbquery.conditions import Condition, Group
from wbquery.query import JOINED, KEY_COL, Query, normalize_key, run_query


def test_normalize_key():
    assert normalize_key(101, "b") == normalize_key(101.0, "b") == normalize_key(" 101 ", "b") == "101"
    assert normalize_key("A1 ", "b") == normalize_key("a1", "b")
    assert normalize_key(pd.Timestamp("2024-01-05 13:00"), "b") == "2024-01-05"
    assert normalize_key(None, "b") == normalize_key(float("nan"), "b") == normalize_key("  ", "b") == "b"


def _query(two_sources, how="inner", where=None):
    return Query(sources=two_sources, mode="join", join_how=how, where=where or Group(),
                 outputs={"L": ["Status"], "R": ["Value"]})


def test_inner_join_with_cross_sheet_conditions(two_sources):
    where = Group("AND", [Condition("L", "Status", "=", "Pass"), Condition("R", "Value", ">", "2e7")])
    res = run_query(_query(two_sources, where=where))
    out = res.tables[JOINED]
    assert out.columns.tolist() == [KEY_COL, "L Excel row", "L.Status", "R Excel row", "R.Value"]
    assert out[KEY_COL].tolist() == ["A1"]
    assert out["R.Value"].tolist() == [5e7]


def test_inner_join_pairs_keys_and_warns_on_duplicates(two_sources):
    res = run_query(_query(two_sources))
    out = res.tables[JOINED]
    assert sorted(map(str, out[KEY_COL])) == ["101", "A1", "A2", "A2"]
    assert any("appear on more than one row" in w for w in res.warnings)
    assert any("blank key" in w for w in res.warnings)


def test_left_and_outer_join(two_sources):
    left = run_query(_query(two_sources, "left")).tables[JOINED]
    assert len(left) == 6  # 5 lots, A2 doubled
    assert left["R.Value"].isna().sum() == 2  # A3 and the blank-key lot
    outer = run_query(_query(two_sources, "outer")).tables[JOINED]
    assert "Z9" in outer[KEY_COL].tolist()
    # A condition on R drops rows R didn't match.
    only_r = run_query(_query(two_sources, "outer", Group("AND", [Condition("R", "Value", "is not blank")])))
    assert len(only_r.tables[JOINED]) == 5


def test_any_sheet_condition_matches_either_side(two_sources):
    where = Group("AND", [Condition("*", "Status", "=", "fail")])
    out = run_query(_query(two_sources, where=where)).tables[JOINED]
    assert out[KEY_COL].tolist() == ["A2", "A2"]


def test_join_requires_keys(two_sources):
    two_sources[1].key = ""
    try:
        run_query(_query(two_sources))
    except ValueError as exc:
        assert "choose a key column" in str(exc)
    else:
        raise AssertionError("expected a ValueError")
