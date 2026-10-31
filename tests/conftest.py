"""Shared fixtures: two small workbooks whose sheets link by lot number."""

from datetime import datetime

import pandas as pd
import pytest

LOTS = pd.DataFrame({
    "Lot": ["A1", "A2", "A3", 101, None],
    "Status": ["Pass", "Fail", "pass", "Pass", "Fail"],
    "Made": [datetime(2024, 1, 5), datetime(2024, 2, 10), datetime(2024, 3, 1), datetime(2024, 3, 15),
             datetime(2024, 4, 1)],
    "Notes": [None, "retest", "ok", None, "orphan"],
})
OWNERS = pd.DataFrame({"Lot": ["A1", "A2"], "Owner": ["Kim", "Lee"]})
RESULTS = pd.DataFrame({
    "Lot ID": ["a1", "A2", "A2", 101.0, "Z9"],
    "Value": [5e7, 7e7, 6e7, 1e7, 3e7],
    "Pct": ["50%", "80%", "90%", "10%", "30%"],
})


@pytest.fixture
def lots_xlsx(tmp_path):
    path = tmp_path / "lots.xlsx"
    with pd.ExcelWriter(path) as xw:
        LOTS.to_excel(xw, sheet_name="Lots", index=False)
        OWNERS.to_excel(xw, sheet_name="Owners", index=False)
    return path


@pytest.fixture
def results_xlsx(tmp_path):
    path = tmp_path / "results.xlsx"
    with pd.ExcelWriter(path) as xw:
        RESULTS.to_excel(xw, sheet_name="Results", index=False)
    return path


@pytest.fixture
def two_sources(lots_xlsx, results_xlsx):
    from wbquery.query import Source

    return [Source("L", str(lots_xlsx), "Lots", key="Lot"), Source("R", str(results_xlsx), "Results", key="Lot ID")]
