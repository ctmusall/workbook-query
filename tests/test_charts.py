import matplotlib

matplotlib.use("Agg")

import numpy as np  # noqa: E402
import pandas as pd  # noqa: E402
import pytest  # noqa: E402
from matplotlib.figure import Figure  # noqa: E402

from wbquery.charts import KINDS, ChartError, ChartSpec, aggregate, draw, imr_stats, is_numeric, numeric, \
    numeric_columns  # noqa: E402

from conftest import LOTS, RESULTS  # noqa: E402

DF = pd.concat([LOTS, RESULTS], axis=1)

SPECS = {
    "line": ChartSpec("line", x="Made", y="Value"),
    "scatter": ChartSpec("scatter", x="Pct", y="Value", group="Status"),
    "bar": ChartSpec("bar", x="Status", y="Value", agg="sum"),
    "grouped_bar": ChartSpec("grouped_bar", x="Status", y="", group="Notes", stacked=True),
    "histogram": ChartSpec("histogram", y="Value", group="Status", bins=5),
    "box": ChartSpec("box", y="Value", group="Status"),
    "pie": ChartSpec("pie", x="Status", agg="count"),
    "heatmap": ChartSpec("heatmap", x="Status", y="Value", group="Notes", agg="mean"),
    "imr": ChartSpec("imr", x="Made", y="Value", title="Value I-MR"),
}


def test_every_kind_has_a_spec():
    assert set(SPECS) == set(KINDS)


@pytest.mark.parametrize("kind", list(KINDS))
def test_every_kind_draws(kind):
    fig = Figure()
    draw(fig, DF, SPECS[kind])
    expected = {"imr": 2, "heatmap": 2}.get(kind, 1)  # heatmap: plot + colorbar
    assert len(fig.axes) == expected


def test_line_with_row_order_and_text_x():
    fig = Figure()
    draw(fig, DF, ChartSpec("line", y="Pct"))
    draw(fig, DF, ChartSpec("line", x="Status", y="Pct", group="Notes"))
    assert fig.axes[0].get_legend() is not None


def test_numeric_parsing():
    s = pd.Series(["50%", "1,000", "5e7", "abc", None, 3])
    assert numeric(s).tolist()[:3] == [0.5, 1000.0, 5e7]
    assert np.isnan(numeric(s)[3]) and np.isnan(numeric(s)[4])
    assert is_numeric(RESULTS["Pct"]) and not is_numeric(LOTS["Status"]) and not is_numeric(LOTS["Made"])
    assert numeric_columns(DF) == ["Value", "Pct"]


def test_aggregate():
    assert aggregate(DF, "Status", "Value", agg="sum")["value"].to_dict() == {"Pass": 6e7, "Fail": 1e8, "pass": 6e7}
    assert aggregate(DF, "Status")["value"].to_dict() == {"Pass": 2, "Fail": 2, "pass": 1}
    t = aggregate(DF, "Status", "Value", group="Notes", agg="mean")
    assert t.loc["Fail", "retest"] == 7e7 and np.isnan(t.loc["Pass", "retest"])
    assert list(aggregate(DF, "Made").index)[0] == "2024-01-05"  # dates sort by date


def test_imr_limits():
    st = imr_stats([1, 2, 3, 2, 1])
    assert st.mean == pytest.approx(1.8) and st.mr_bar == pytest.approx(1.0)
    assert st.ucl == pytest.approx(1.8 + 3 / 1.128)
    assert st.mr_ucl == pytest.approx(3.267, abs=1e-3)
    assert not st.out_of_limits.any()
    with pytest.raises(ChartError):
        imr_stats([1])


def test_notes_and_errors():
    notes = draw(Figure(), pd.DataFrame({"v": [1, 2, "x", None]}), ChartSpec("histogram", y="v"))
    assert notes == ["1 non-numeric value in 'v' skipped"]
    with pytest.raises(ChartError, match="no numeric values"):
        draw(Figure(), DF, ChartSpec("histogram", y="Status"))
    with pytest.raises(ChartError, match="group"):
        draw(Figure(), DF, ChartSpec("grouped_bar", x="Status"))
    with pytest.raises(ChartError, match="rows"):
        draw(Figure(), DF, ChartSpec("heatmap", x="Status"))
    with pytest.raises(ChartError, match="Y"):
        draw(Figure(), DF, ChartSpec("box"))
    with pytest.raises(ChartError, match="no rows"):
        draw(Figure(), DF.iloc[:0], ChartSpec("line", y="Value"))


def test_many_categories_are_capped():
    df = pd.DataFrame({"c": [f"k{i}" for i in range(80)], "v": range(80)})
    assert "largest" in draw(Figure(), df, ChartSpec("bar", x="c", y="v", agg="sum"))[0]
    assert "Other" in draw(Figure(), df, ChartSpec("pie", x="c", y="v", agg="sum"))[-1]
    with pytest.raises(ChartError, match="distinct values"):
        draw(Figure(), df, ChartSpec("box", y="v", group="c"))
