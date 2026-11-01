# Workbook Query

Query rows across any number of Excel workbooks and sheets, then export the matches.

- Load many workbooks and pick sheets from each (`.xlsx`, `.xlsm`, `.xls`, `.csv`, `.tsv`).
- Build a query from **AND / OR groups** of conditions, nested as deep as you like and each one optionally negated with **NOT**. A condition tests one column of one sheet, or of **any sheet** that has that column.
- Choose **output columns for each sheet**.
- Combine sheets in one of two ways:
  - **Filter each sheet on its own:** results are listed per sheet, plus an **All results** table with every match in one place.
  - **Join on a key column:** rows from different sheets are linked by a value such as a lot number, giving one merged row per key.
- See results in the app, sort them by any column, and export them to **Excel** or a self-contained **HTML** report.
- Every result row shows its file, sheet and Excel row number. The export records each input file's SHA-256 so you can prove which data was used.
- Save a query to JSON and run it again later, from the app or from the command line.
- **Visualize** the results as line, scatter, bar, histogram, box, pie, heatmap or I-MR control charts, and save a chart as PNG, SVG or PDF.

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
   - *Add condition* adds to the selected group. It copies the selected condition's sheet and column. With nothing selected and several sheets loaded, it starts on *Any sheet*.
   - **Any sheet** conditions test every sheet that has the column. A sheet without it skips that condition. A sheet that has none of the columns your conditions test matches no rows, and the status line names it.
   - *Add for column…* lists the columns that more than one sheet has. Tick as many as you like and set the operator and value they all get. You can leave the value blank to fill it in later; the Value box suggests values from the ticked columns.
     - Choose where the conditions go: into the selected group, or into a new OR group (a row matches if *any* of the columns matches) or a new AND group (*every* column must match).
     - OK adds one *Any sheet* condition per column. If the operator and value don't fit together, the dialog says so and stays open.
   - Column names match even when their case or surrounding spaces differ, so `Lot ID`, `lot id` and `Lot ID ` count as the same column.
   - Type in the *Column* box to search it. The *Value* box suggests values that are actually in that column. Press Enter in it to run the query.
   - *Duplicate* (Ctrl+D) copies the selected condition or group. *Remove* (or Delete) deletes it.
   - After a run, right-click a cell in a result table to add a condition `= that value` (or `<>`, or *is blank*).
   - *Add group* nests a new AND/OR group.
   - Set each group's *Match* to AND (all items must match) or OR (any item may match). Tick *NOT* to invert a group.
   - Untick *On* to switch a condition, or a whole group, off without deleting it. Everything inside a switched-off group is greyed out and left out of the query.
   - Give a group a name in its *Name / Column* box. The name shows in the *Where:* line and in exported reports as `[name] (…)`.
   - Collapse a group with its arrow to tidy the tree. While it's collapsed, its row shows one *Operator* and *Value* for every condition inside, when they all share the same ones. Editing those changes all of them at once. If they differ, the row says so; expand the group to edit them one by one. Collapsed groups stay collapsed when you save and reload a query.
   - **Highlight color:** click a condition's *Color* swatch to pick a color; right-click it for *No color*. After a run, the cells that condition counted for are filled with its color in the result tables and in both exports. A cell counts only when its condition is true *and* every group above it is true for that row. So in `(Person 1 = KSG AND Chest 1 <> 0) OR (Person 2 = KSG AND …)`, Chest 1 is colored only on rows where Person 1 is KSG. Conditions inside a *NOT* group never color cells. When several colored conditions count for a cell, the first one in the tree wins. A group's swatch colors every condition inside it. *Add for column…* can give all its conditions one color.
   - The *Where:* line shows the whole query in words.
   - Column pick lists are alphabetical, with numbers in order (`P2` before `P10`). Each sheet's output list keeps the output order instead.
4. **Output columns:** tick the columns you want from each sheet, and drag to reorder them. **Choose for all sheets…** lists every column once. Ticking or unticking one applies to every sheet that has it. A half-ticked column is ticked in some sheets only, and is left as it is unless you change it.
5. **Run query**, then **Export Excel…** or **Export HTML…**. **Save query…** stores everything above as JSON.
6. **Visualize** (or the *Visualize* tab) charts a result table. Choose the table, the chart type and its columns; the chart redraws as you change them. Use the toolbar to zoom and pan, and **Save chart…** for a PNG, SVG or PDF.

### Chart types

| Chart | Uses |
|---|---|
| Line / run chart | Y over X (dates, numbers or text; blank X = row order), one line per Y series and *Group by* value |
| Scatter | Y against X, colored by Y series and an optional column |
| Bar | One bar per X category: count of rows, or count / sum / mean / median / min / max of Y |
| Grouped / stacked bar | As Bar, with bars split by *Group by*; tick *Stacked* to stack them |
| Histogram | Distribution of Y in *Bins* bins, overlaid per optional group |
| Box plot | Y's median, quartiles and outliers, one box per optional group |
| Pie | Share of each X category (count, or an aggregate of Y) |
| Heatmap / pivot | X categories across, *Rows* down, each cell the aggregate of Y (or a count) |
| I-MR control chart | Individuals and moving range of Y with X̄ ± 2.66·MR̄ and MR UCL = 3.267·MR̄ limits; points outside are red. *Order by* sorts by a date or number column |

Y values are read like conditions read them (`1,000`, `5e7` and `50%` are numbers, and `1 Micrococcus luteus` is 1). Values that aren't numbers are skipped, and the note under the chart says how many. Tick **Label points with their text** to write the text after each number next to its point, such as *Micrococcus luteus*. Labels never overlap: the highest values are labeled first, each label takes the first free corner around its point, and labels with no room are left off. The note under the chart says how many were left off. In the app, hover over any point to see its column, full cell text and X value. When several readings share a spot, the tooltip lists them all.

Line and scatter charts can plot several Y columns: click **More Y…** and tick them (search `chest`, `sleeve`, `hand`). With **Combine numbered columns**, columns that differ only by a number make one series: `Chest 1`, `Chest 2` and `Chest 1 (2)` all go on the *Chest* line. **Only cells that met a condition** plots just the cells the query counted for (the highlighted ones, whether or not they have a color). For example, `<> 0` conditions give only the non-zero readings. To plot over time, add a date column such as *Monitor Date* to the output columns and choose it as X. To overlay datasets that use a different X column, click **More datasets…** and add one row per dataset, each with its own X and Y columns (e.g. *Sample Date* with the Hand columns). All the X columns must be the same kind: all dates, all numbers or all text. Charts show at most 50 categories (pie charts group the rest as *Other*).

### Operators

| Operator | Matches cells that… |
|---|---|
| `=` / `<>` | equal / differ from the value. Numbers and dates compare by value; text ignores case and surrounding spaces |
| `<` `<=` `>` `>=` | compare as numbers (`5e7`, `1,000`, `50%`) or dates (`2024-03-01`) |
| `contains` / `not contains` | contain the text (ignoring case) / don't |
| `starts with` / `ends with` | start / end with the text |
| `in` / `not in` | equal one of a comma-separated list (`Pass, Retest`) / none of them |
| `is blank` / `is not blank` | are empty / have a value |

Excel date serial numbers and `50%`-style text are understood. A cell that starts with a number, then a space, then any text reads as that number: `1 Micrococcus luteus` and `3 CFU` match `> 0` and `= 1` / `= 3`. Only the first number counts. `2026-01-05` and `5/2` aren't read this way, because there is no space after the number. The value you type in a condition is never read this way: `= 1 Micrococcus luteus` compares text. A cell that can't be compared, such as `TNTC` in a `>` test, never matches, except with `<>`: a non-blank cell that isn't a number differs from one, so `TNTC` matches `<> 0`. Blank cells never match `<>`; use *is blank* for those.

### Filter each sheet on its own vs. join

**Filter each sheet on its own:**
- Each sheet uses only the conditions that apply to it: conditions naming that sheet, plus *Any sheet* conditions on a column the sheet has.
- So `A › x > 5 OR B › y < 3` filters A by `x > 5` and B by `y < 3`.
- With no conditions (or all switched off), every sheet keeps all its rows. Otherwise a sheet that none of the conditions apply to matches no rows, and a warning names it.
- Each sheet with ticked output columns gives one result table.
- When there are several, an **All results** table stacks them all, first in the results and exports. Columns are matched by name; a column a sheet doesn't have is left blank for its rows. A **Source** column says which sheet each row came from.

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
      {"source": "*", "column": "Notes", "op": "contains", "value": "retest", "color": "#ffd54f"}
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

The app icon (`wbquery/assets/icon.png` and `icon.ico`) is drawn by `scripts/make_icon.py`; rerun it after changing the design.

## Rust port (in progress)

The app has been ported to Rust (`crates/`): the engine, the command line, the charts and the desktop app (egui).
The Python version stays the released one until the switch-over.

| Crate | What it is |
|---|---|
| `wbquery-core` | loading sheets, conditions, independent / join queries, Excel and HTML export |
| `wbquery-charts` | the nine chart types: what to plot (series, bars, limits, notes), drawn to SVG, PNG (resvg) or PDF (svg2pdf) |
| `wbquery-cli` | `wbquery-cli`, with the same options, messages and exit codes as the Python CLI |
| `wbquery-gui` | `wbquery`, the desktop app: the same window as the Python app; started with options, it runs the command line |

```bash
cargo test --workspace                       # unit, parity and CLI tests
cargo run -p wbquery-cli -- -q query.json    # run the CLI
cargo run -p wbquery-gui                     # run the app (cargo run -p wbquery-gui -- book.xlsx opens a workbook)
just ci                                      # everything CI checks: fmt, clippy, cargo-deny, tests, smoke test
```

The toolchain version is pinned in `rust-toolchain.toml`; `rustup` installs it on first use.

**Parity tests.** `tests/golden/generate.py` ran the Python implementation over fixture workbooks and recorded
its results (loading, every operator on many kinds of columns, query results with highlight colors, the HTML
report and the Excel layout); `tests/golden/generate_charts.py` recorded what matplotlib plotted for 46 charts
(series and their points, bars, box-plot whiskers, pie wedges, heatmap cells, control limits, legends, notes).
The Rust tests check against those records, so the two give the same answers.

Charts are drawn with the app's font (Ubuntu Light, embedded; licence in `licenses/`).

**App tests.** `crates/wbquery-gui/tests/state.rs` drives the window's logic the way the Python GUI tests did
(adding sheets, editing conditions, running, exporting, saving and loading, charts); `tests/ui.rs` clicks the real
window headless with egui_kittest. `WBQ_UI_DIR=some/dir cargo test -p wbquery-gui --test ui` also saves
screenshots (needs a GPU or a software Vulkan / GL driver).

**Known differences from the Python version:**

- `.xls` files open (the Python version needed `xlrd`, which wasn't installed).
- CSV headers that are blank or repeated are named like Excel headers (`Column 3`, `Name (2)`), not
  `Unnamed: 2` / `Name.1`.
- Dates in conditions and cells must be written out (`2024-01-05`, `1/5/2024`, `Jan 5, 2024`, with an optional
  time); words such as `today` and dates without a year aren't read as dates.
- A blank cell in a date column never matches a numeric condition (the Python version matched it with `<>` and
  `<`).
- `--print` lays the tables out in its own format.
- In the app, charts zoom and pan with the mouse (scroll, drag, double-click to reset) instead of matplotlib's
  toolbar, and the conditions editor is drawn by egui; the controls and what they do are the same.
- Charts plot the same data, but their axes are laid out by the Rust renderer: tick spacing differs a little, and
  large numbers read `20M` rather than matplotlib's `2` with a `1e7` offset.
