# Workbook Query

Query rows across any number of Excel workbooks and sheets, then export the matches.

- Load many workbooks and pick sheets from each (`.xlsx`, `.xlsm`, `.xls`, `.csv`, `.tsv`).
- Build a query from **AND / OR groups** of conditions, nested as deep as you like and each one optionally negated with **NOT**. A condition tests one column of one sheet, or of **any sheet** that has that column.
- Choose **output columns for each sheet**.
- Combine sheets in one of two ways:
  - **Filter each sheet on its own:** results are listed per sheet.
  - **Join on a key column:** rows from different sheets are linked by a value such as a lot number, giving one merged row per key.
- See results in the app, sort them by any column, and export them to **Excel** or a self-contained **HTML** report.
- Every result row shows its file, sheet and Excel row number. The export records each input file's SHA-256 so you can prove which data was used.
- Save a query to JSON and run it again later, from the app or from the command line.

## Download

From the [latest release](../../releases/latest):

| File | What it is |
|---|---|
| **wbquery.exe** | Windows app: double-click it, or drop workbooks onto it |
| wbquery-cli.exe | Windows command-line version |
| **wbquery-linux** | Linux app (`chmod +x wbquery-linux` first) |
| wbquery-cli-linux | Linux command-line version |
| SHA256SUMS.txt | Fingerprints of the files above |

Windows may show a SmartScreen warning because the exe isn't code-signed. Click *More info* → *Run anyway*.

## Input files

Each sheet is read as a table. Column names come from, in order of priority:

1. a sibling tab named `Column Headers (<sheet>)`, if the workbook has one;
2. otherwise the first row, if it looks like a header;
3. otherwise generic names: `Column 1`, `Column 2`, …

Fully blank rows are skipped. Each row keeps its **Excel row number** so you can find it in the source file.

## Using the app

1. **Add workbooks…** For a workbook with several sheets, tick the sheets you want. Each sheet gets a name such as `lots/Lots`; double-click a name to rename it.
2. **How sheets combine:** choose *Filter each sheet on its own* or *Join sheets on a key column*. In join mode, pick each sheet's **Join key** column and a join type:
   - **inner:** keys found in every sheet;
   - **left:** every row of the first sheet;
   - **outer:** every key from any sheet.
3. **Conditions:**
   - *Add condition* adds to the selected group.
   - *Add group* nests a new AND/OR group.
   - Set each group's *Match* to AND (all items must match) or OR (any item may match). Tick *NOT* to invert a group.
   - Untick *On* to switch a condition off without deleting it.
   - The *Where:* line shows the whole query in words.
4. **Output columns:** tick the columns you want from each sheet, and drag to reorder them.
5. **Run query**, then **Export Excel…** or **Export HTML…**. **Save query…** stores everything above as JSON.

### Operators

| Operator | Matches cells that… |
|---|---|
| `=` / `<>` | equal / differ from the value. Numbers and dates compare by value; text ignores case and surrounding spaces |
| `<` `<=` `>` `>=` | compare as numbers (`5e7`, `1,000`, `50%`) or dates (`2024-03-01`) |
| `contains` / `not contains` | contain the text (ignoring case) / don't |
| `starts with` / `ends with` | start / end with the text |
| `in` / `not in` | equal one of a comma-separated list (`Pass, Retest`) / none of them |
| `is blank` / `is not blank` | are empty / have a value |

Excel date serial numbers and `50%`-style text are understood. A cell that can't be compared, such as text in a `>` test, never matches.

### Filter each sheet on its own vs. join

**Filter each sheet on its own:**
- Each sheet uses only the conditions that apply to it: conditions naming that sheet, plus *Any sheet* conditions on a column the sheet has.
- So `A › x > 5 OR B › y < 3` filters A by `x > 5` and B by `y < 3`.
- A sheet with no conditions that apply keeps all its rows.
- Each sheet with ticked output columns gives one result table.

**Join on a key column:**
- Keys match regardless of case and surrounding spaces.
- `101`, `101.0` and `" 101 "` all count as the same key, and dates match by day.
- The whole condition tree is tested against each joined row, so conditions can mix sheets freely: `Lots › Status = Pass AND Results › Value > 5e7`.
- If a key appears on several rows of one sheet, each of those rows is paired with every match. The app shows a warning when this happens.
- Rows with a blank key never match another sheet.

### Saved query format

```json
{
  "version": 1,
  "mode": "join",
  "join_how": "inner",
  "include_provenance": true,
  "sources": [
    {"alias": "L", "path": "lots.xlsx", "sheet": "Lots", "key": "Lot"},
    {"alias": "R", "path": "results.xlsx", "sheet": "Results", "key": "Lot ID"}
  ],
  "where": {"logic": "AND", "negate": false, "items": [
    {"source": "L", "column": "Status", "op": "=", "value": "Pass"},
    {"logic": "OR", "negate": false, "items": [
      {"source": "R", "column": "Value", "op": ">", "value": "5e7"},
      {"source": "*", "column": "Notes", "op": "contains", "value": "retest"}
    ]}
  ]},
  "outputs": {"L": ["Lot", "Status"], "R": ["Value"]}
}
```

`"source": "*"` means any sheet.

## Command line

```
wbquery-cli --list book1.xlsx book2.xlsx           # show each file's sheets and columns
wbquery-cli -q query.json -o out/results           # writes out/results.xlsx and out/results.html
wbquery-cli -q query.json --source R=new.xlsx      # run a saved query against a different file
wbquery-cli -q query.json --formats html --print   # HTML only, and print the tables
```

Running `python -m wbquery` (or `wbquery.exe`) with no options opens the app. Any file paths you pass are opened, and `.json` files are loaded as queries. Passing any `-option` runs the CLI instead.

## Development

```bash
python3 -m venv .venv
.venv/bin/pip install -e '.[dev]'
QT_QPA_PLATFORM=offscreen .venv/bin/pytest --cov=wbquery
.venv/bin/python -m wbquery              # run the app
```

## Building executables

| Where | How |
|---|---|
| Linux | `./build.sh` → `dist/wbquery`, `dist/wbquery-cli` |
| Windows | `powershell -ExecutionPolicy Bypass -File build.ps1` → `dist\wbquery.exe`, `dist\wbquery-cli.exe` |
| GitHub Actions | Every push runs the tests and builds both platforms. On `main`, a release `v<version>` is published (version from `wbquery/__init__.py`; bump it there and in `pyproject.toml`) |

PyInstaller can't cross-compile, so each platform's executables are built on that platform. The build scripts run the test suite first.
