"""Write two small workbooks and a join query for the CI smoke test of the built CLI.

Usage: python tests/data/make_smoke.py OUT_DIR
"""

import json
import sys
from pathlib import Path

import pandas as pd

out = Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=True)
pd.DataFrame({"Lot": ["A1", "A2", "A3"], "Status": ["Pass", "Fail", "Pass"]}).to_excel(
    out / "lots.xlsx", sheet_name="Lots", index=False)
pd.DataFrame({"Lot ID": ["a1", "A2", "A3"], "Value": [5e7, 7e7, 1e7]}).to_excel(
    out / "results.xlsx", sheet_name="Results", index=False)
query = {
    "version": 1, "mode": "join", "join_how": "inner",
    "sources": [{"alias": "L", "path": str(out / "lots.xlsx"), "sheet": "Lots", "key": "Lot"},
                {"alias": "R", "path": str(out / "results.xlsx"), "sheet": "Results", "key": "Lot ID"}],
    "where": {"logic": "AND", "items": [{"source": "L", "column": "Status", "op": "=", "value": "Pass"},
                                        {"source": "R", "column": "Value", "op": ">", "value": "2e7"}]},
    "outputs": {"L": ["Status"], "R": ["Value"]},
}
(out / "query.json").write_text(json.dumps(query, indent=2))
