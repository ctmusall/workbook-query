import pandas as pd
import pytest

from wbquery.conditions import Condition, Group, evaluate, parse_number

S = pd.Series(["Pass", " fail ", None, "", "50%", "1,000", "2024-03-01", "Retest needed"], index=range(2, 10))


@pytest.mark.parametrize("op, value, expected", [
    ("=", "pass", [2]),
    ("=", "", [4, 5]),
    ("<>", "pass", [3, 4, 5, 6, 7, 8, 9]),
    (">", "0.4", [6, 7]),
    (">=", "1000", [7]),
    ("<", "60%", [6]),
    (">", "2024-02-01", [8]),
    ("contains", "TEST", [9]),
    ("not contains", "a", [4, 5, 6, 7, 8, 9]),
    ("starts with", "re", [9]),
    ("ends with", "IL", [3]),
    ("in", "pass, FAIL", [2, 3]),
    ("in", "0.5, 1000", [6, 7]),
    ("not in", "pass, fail", [6, 7, 8, 9]),
    ("is blank", "", [4, 5]),
    ("is not blank", "", [2, 3, 6, 7, 8, 9]),
])
def test_operators(op, value, expected):
    hit = Condition("X", "c", op, value).matches(S)
    assert list(S.index[hit.to_numpy(dtype=bool)]) == expected


def test_bad_operator_and_value():
    with pytest.raises(ValueError, match="unknown operator"):
        Condition("X", "c", "~", "1")
    with pytest.raises(ValueError, match="needs a number or date"):
        Condition("X", "c", ">", "abc").check_value()
    with pytest.raises(ValueError, match="needs a value"):
        Condition("X", "c", "contains", "").check_value()
    with pytest.raises(ValueError, match="separated by commas"):
        Condition("X", "c", "in", " , ").check_value()
    Condition("X", "c", "is blank").check_value()


def test_parse_number():
    assert parse_number("2e7") == 2e7
    assert parse_number("1,000") == 1000
    assert parse_number("50%") == 0.5
    assert parse_number(" ") is None
    with pytest.raises(ValueError):
        parse_number("x")


def _frame_resolver(df):
    return lambda c: [df[c.column]] if c.column in df.columns else []


def test_nested_groups_and_not():
    df = pd.DataFrame({"a": [1, 2, 3, 4], "b": ["x", "y", "x", "y"]})
    tree = Group("OR", [
        Condition("*", "a", ">", "3"),
        Group("AND", [Condition("*", "a", "<", "3"), Condition("*", "b", "=", "y")]),
    ])
    assert evaluate(tree, _frame_resolver(df)).tolist() == [False, True, False, True]
    tree.negate = True
    assert evaluate(tree, _frame_resolver(df)).tolist() == [True, False, True, False]


def test_disabled_and_inapplicable_are_left_out():
    df = pd.DataFrame({"a": [1, 2]})
    tree = Group("AND", [Condition("*", "a", "=", "1"), Condition("*", "missing", "=", "1"),
                         Condition("*", "a", "=", "2", enabled=False)])
    assert evaluate(tree, _frame_resolver(df)).tolist() == [True, False]
    assert evaluate(Group("OR", [Condition("*", "missing", "=", "1")]), _frame_resolver(df)) is None
    assert evaluate(Group(), _frame_resolver(df)) is None


def test_describe():
    tree = Group("AND", [Condition("A", "x", ">", "5"),
                         Group("OR", [Condition("B", "s", "=", "Pass"), Condition("*", "s", "is blank")])])
    assert tree.describe() == "A › x > '5' AND (B › s = 'Pass' OR Any sheet › s is blank)"
    tree.negate = True
    assert tree.describe().startswith("NOT (")
    assert Group().describe() == "(all rows)"


def test_dict_round_trip():
    tree = Group("OR", [Condition("A", "x", "in", "1,2"), Group("AND", [Condition("*", "y", "is blank")], True)])
    again = Group.from_dict(tree.to_dict())
    assert again == tree
    with pytest.raises(ValueError, match="column"):
        Group.from_dict({"items": [{"source": "A"}]})
