import json

import pytest

from wbquery.conditions import Condition, Group
from wbquery.query import Query, Source, default_alias, load_query, save_query, unique_alias


def test_round_trip(tmp_path, two_sources):
    q = Query(sources=two_sources, mode="join", join_how="left",
              where=Group("OR", [Condition("L", "Status", "=", "Pass"), Group("AND", [], negate=True)]),
              outputs={"L": ["Lot"], "R": ["Value", "Pct"]}, include_provenance=False,
              name="Lots vs results", description="Line one\nline two")
    path = save_query(q, tmp_path / "q.json")
    assert json.loads(path.read_text())["version"] == 1
    assert load_query(path) == q


def test_unknown_keys_ignored_and_errors(tmp_path):
    p = tmp_path / "q.json"
    p.write_text(json.dumps({"extra": 1, "sources": [{"alias": "A", "path": "a.xlsx", "sheet": "S", "x": 2}]}))
    assert load_query(p).sources == [Source("A", "a.xlsx", "S")]
    assert (load_query(p).name, load_query(p).description) == ("", "")  # older files have neither
    p.write_text("{nope")
    with pytest.raises(ValueError, match="not valid JSON"):
        load_query(p)
    p.write_text(json.dumps({"sources": [{"alias": "A"}]}))
    with pytest.raises(ValueError, match="needs"):
        load_query(p)
    with pytest.raises(ValueError, match="not found"):
        load_query(tmp_path / "missing.json")


def test_aliases():
    assert default_alias("/x/book.xlsx", "Sheet1") == "book/Sheet1"
    assert default_alias("/x/data.csv", "data") == "data"
    assert unique_alias("a", ["a", "a (2)"]) == "a (3)"
    assert unique_alias("b", ["a"]) == "b"
