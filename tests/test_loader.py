import pandas as pd
import pytest

from wbquery import loader
from wbquery.conditions import Condition, Group
from wbquery.query import Query, Source, run_query


@pytest.fixture
def header_tab_xlsx(tmp_path):
    """Data tab without a header row, names on a 'Column Headers (Data)' tab, plus hidden helper tabs."""
    path = tmp_path / "tabbed.xlsx"
    with pd.ExcelWriter(path) as xw:
        pd.DataFrame([["Q1", 45292, "12%"], [None, None, None], ["Q2", 45293, "7%"]]).to_excel(
            xw, sheet_name="Data", index=False, header=False)
        pd.DataFrame([["Lot", "Date Processed", "Rate"]]).to_excel(
            xw, sheet_name="Column Headers (Data)", index=False, header=False)
        pd.DataFrame([[1]]).to_excel(xw, sheet_name="MC Tags", index=False, header=False)
        pd.DataFrame([[1, 2]]).to_excel(xw, sheet_name="Summary", index=False, header=False)
    return path


def test_header_tab_and_sheet_list(header_tab_xlsx):
    assert loader.list_sheets(header_tab_xlsx) == ["Summary", "Data"]
    df = loader.load_table(header_tab_xlsx, "Data")
    assert list(df.columns) == ["Lot", "Date Processed", "Rate"]
    assert list(df.index) == [1, 3]  # blank Excel row 2 dropped, row numbers kept
    assert list(loader.load_table(header_tab_xlsx, "Summary").columns) == ["Column 1", "Column 2"]
    with pytest.raises(ValueError, match="not found"):
        loader.load_table(header_tab_xlsx, "Nope")


def test_serial_dates_and_percent_compare(header_tab_xlsx):
    q = Query(sources=[Source("D", str(header_tab_xlsx), "Data")],
              where=Group("AND", [Condition("D", "Date Processed", ">", "2024-01-01"),
                                  Condition("D", "Rate", "<", "10%")]),
              outputs={"D": ["Lot"]})
    assert run_query(q).tables["D"]["Lot"].tolist() == ["Q2"]


def test_csv(tmp_path):
    path = tmp_path / "data.csv"
    path.write_text("Lot,Value\nA,1\nB,2\n")
    assert loader.list_sheets(path) == ["data"]
    df = loader.load_table(path)
    assert list(df.index) == [2, 3] and list(df.columns) == ["Lot", "Value"]
    assert loader.numeric_columns(df) == ["Value"]
    with pytest.raises(FileNotFoundError):
        loader.load_table(tmp_path / "none.csv")
