//! Parity with the Python chart code: what each chart plots (tests/golden/charts.json, recorded from
//! matplotlib by tests/golden/generate_charts.py).

use std::path::PathBuf;

use chrono::NaiveDateTime;
use serde_json::Value;
use wbquery_charts::scene::{Axis, Chart, Mark, Panel};
use wbquery_charts::spec::{ChartSpec, Dataset};
use wbquery_charts::stats::{aggregate, imr_stats};
use wbquery_charts::values::{is_numeric, numeric, numeric_columns, series_name};
use wbquery_core::cell::{Cell, Column, Frame};

fn golden() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/charts.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn cell(v: &Value) -> Cell {
    match v {
        Value::Null => Cell::Empty,
        Value::Bool(b) => Cell::Bool(*b),
        Value::Number(n) if n.is_f64() => Cell::Float(n.as_f64().unwrap()),
        Value::Number(n) => Cell::Int(n.as_i64().unwrap()),
        Value::String(s) => Cell::Text(s.clone()),
        Value::Object(o) => {
            Cell::Date(NaiveDateTime::parse_from_str(o["d"].as_str().unwrap(), "%Y-%m-%dT%H:%M:%S").unwrap())
        }
        other => panic!("{other}"),
    }
}

fn frame(doc: &Value) -> Frame {
    let columns: Vec<Column> = doc["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let name = c.as_str().unwrap();
            Column::inferred(name, doc["cells"][name].as_array().unwrap().iter().map(cell).collect())
        })
        .collect();
    let n = columns.first().map_or(0, Column::len);
    Frame { columns, index: (0..n as i64).collect() }
}

fn spec(doc: &Value) -> ChartSpec {
    let s = |k: &str| doc[k].as_str().unwrap_or("").to_string();
    let list =
        |v: &Value| v.as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
    ChartSpec {
        kind: s("kind"),
        x: s("x"),
        y: s("y"),
        group: s("group"),
        agg: s("agg"),
        bins: doc["bins"].as_u64().unwrap() as usize,
        stacked: doc["stacked"].as_bool().unwrap(),
        title: s("title"),
        ys: list(&doc["ys"]),
        merge_numbered: doc["merge_numbered"].as_bool().unwrap(),
        only_hits: doc["only_hits"].as_bool().unwrap(),
        point_labels: doc["point_labels"].as_bool().unwrap(),
        datasets: doc["datasets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| Dataset { x: d["x"].as_str().unwrap().into(), ys: list(&d["ys"]) })
            .collect(),
    }
}

/// What matplotlib would report for one axes of the chart.
#[derive(Default, Debug)]
struct Axes {
    xlabel: String,
    ylabel: String,
    title: String,
    lines: Vec<(Option<String>, Vec<[f64; 2]>)>,
    scatter: Vec<(Option<String>, Vec<[f64; 2]>)>,
    bars: Vec<[f64; 4]>,
    wedges: Vec<[f64; 2]>,
    texts: Vec<String>,
    legend: Option<(Vec<String>, String)>,
    xticks: Option<Vec<String>>,
    yticks: Option<Vec<String>>,
    image: Option<Vec<Vec<Option<f64>>>>,
}

fn ticks(axis: &Axis) -> Option<Vec<String>> {
    match axis {
        Axis::Category(c) => Some(c.clone()),
        Axis::Ticks(t) => Some(t.iter().map(|(_, s)| s.clone()).collect()),
        _ => None,
    }
}

fn axes_of(panel: &Panel) -> Vec<Axes> {
    let mut a = Axes {
        xlabel: panel.xlabel.clone(),
        ylabel: panel.ylabel.clone(),
        title: panel.title.clone(),
        xticks: ticks(&panel.x),
        yticks: ticks(&panel.y),
        ..Axes::default()
    };
    let mut pcts = Vec::new();
    for m in &panel.marks {
        match m {
            Mark::Line { label, points, .. } => a.lines.push((label.clone(), points.clone())),
            Mark::HLine { y, text, .. } => {
                a.lines.push((None, vec![[0.0, *y], [1.0, *y]]));
                a.texts.extend(text.clone());
            }
            Mark::Scatter { label, points, .. } => a.scatter.push((label.clone(), points.clone())),
            Mark::Bars { rects, .. } => a.bars.extend(rects.iter().map(|r| [r.x, r.y, r.w, r.h])),
            Mark::Boxes { boxes, .. } => {
                for b in boxes {
                    let (p, w, s) = (b.pos, b.width, &b.stats);
                    a.lines.push((None, vec![[p, s.q1], [p, s.whisker_lo]]));
                    a.lines.push((None, vec![[p, s.q3], [p, s.whisker_hi]]));
                    a.lines.push((None, vec![[p - w / 4.0, s.whisker_lo], [p + w / 4.0, s.whisker_lo]]));
                    a.lines.push((None, vec![[p - w / 4.0, s.whisker_hi], [p + w / 4.0, s.whisker_hi]]));
                    a.lines.push((None, vec![[p - w / 2.0, s.median], [p + w / 2.0, s.median]]));
                    a.lines.push((None, s.fliers.iter().map(|f| [p, *f]).collect()));
                }
            }
            Mark::Pie { slices } => {
                for s in slices {
                    a.wedges.push([s.theta1, s.theta2]);
                    a.texts.push(s.label.clone());
                    pcts.push(s.pct.clone());
                }
            }
            Mark::Heatmap { values, texts, .. } => {
                a.image = Some(values.clone());
                a.texts.extend(texts.iter().map(|t| t.text.clone()));
            }
        }
    }
    a.texts.extend(pcts);
    a.texts.extend(panel.labels.iter().map(|l| l.text.clone()));
    if let Some(title) = &panel.legend {
        a.legend =
            Some((panel.legend_entries().iter().map(|(l, _, _)| l.to_string()).collect(), title.clone()));
    }
    let mut out = vec![a];
    if let Some(cb) = &panel.colorbar {
        out.push(Axes { ylabel: cb.label.clone(), ..Axes::default() });
    }
    out
}

fn close(a: f64, b: Option<f64>) -> bool {
    match b {
        None => a.is_nan(),
        Some(b) => (a - b).abs() <= 1e-9 * b.abs().max(1.0),
    }
}

fn points_match(got: &[[f64; 2]], want: &Value) -> bool {
    let want = want.as_array().unwrap();
    got.len() == want.len()
        && got.iter().zip(want).all(|(g, w)| close(g[0], w[0].as_f64()) && close(g[1], w[1].as_f64()))
}

fn label_match(got: &Option<String>, want: &Value) -> bool {
    let w = want.as_str().unwrap();
    match got {
        None => w.starts_with('_'),
        Some(g) => g == w,
    }
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
}

fn compare(name: &str, chart: &Chart, case: &Value, problems: &mut Vec<String>) {
    let mut say =
        |what: &str, got: String, want: &Value| problems.push(format!("{name}: {what}: {got} != {want}"));
    let notes = strings(&case["notes"]);
    if chart.notes != notes {
        say("notes", format!("{:?}", chart.notes), &case["notes"]);
    }
    if chart.title != case["suptitle"].as_str().unwrap() {
        say("title", chart.title.clone(), &case["suptitle"]);
    }
    let axes: Vec<Axes> = chart.panels.iter().flat_map(axes_of).collect();
    let want_axes = case["axes"].as_array().unwrap();
    if axes.len() != want_axes.len() {
        say("axes", axes.len().to_string(), &Value::from(want_axes.len()));
        return;
    }
    for (i, (a, w)) in axes.iter().zip(want_axes).enumerate() {
        let at = |what: &str| format!("axes {i} {what}");
        for (what, got) in [("xlabel", &a.xlabel), ("ylabel", &a.ylabel), ("title", &a.title)] {
            if got != w[what].as_str().unwrap() {
                say(&at(what), got.clone(), &w[what]);
            }
        }
        let wl = w["lines"].as_array().unwrap();
        if a.lines.len() != wl.len()
            || !a
                .lines
                .iter()
                .zip(wl)
                .all(|((l, p), w)| label_match(l, &w["label"]) && points_match(p, &w["xy"]))
        {
            say(&at("lines"), format!("{:?}", a.lines), &w["lines"]);
        }
        let ws = w["scatter"].as_array().unwrap();
        if a.scatter.len() != ws.len()
            || !a
                .scatter
                .iter()
                .zip(ws)
                .all(|((l, p), w)| label_match(l, &w["label"]) && points_match(p, &w["xy"]))
        {
            say(&at("scatter"), format!("{:?}", a.scatter), &w["scatter"]);
        }
        let wb = w["bars"].as_array().unwrap();
        if a.bars.len() != wb.len()
            || !a.bars.iter().zip(wb).all(|(g, w)| (0..4).all(|k| close(g[k], w[k].as_f64())))
        {
            say(&at("bars"), format!("{:?}", a.bars), &w["bars"]);
        }
        let ww = w["wedges"].as_array().unwrap();
        if a.wedges.len() != ww.len()
            || !a.wedges.iter().zip(ww).all(|(g, w)| (0..2).all(|k| close(g[k], w[k].as_f64())))
        {
            say(&at("wedges"), format!("{:?}", a.wedges), &w["wedges"]);
        }
        if a.texts != strings(&w["texts"]) {
            say(&at("texts"), format!("{:?}", a.texts), &w["texts"]);
        }
        let legend = match &w["legend"] {
            Value::Null => None,
            l => Some((strings(l), w["legend_title"].as_str().unwrap().to_string())),
        };
        if a.legend != legend {
            say(&at("legend"), format!("{:?}", a.legend), &w["legend"]);
        }
        if let Some(t) = &a.xticks
            && *t != strings(&w["xticklabels"])
        {
            say(&at("xticks"), format!("{t:?}"), &w["xticklabels"]);
        }
        if let Some(t) = &a.yticks
            && *t != strings(&w["yticklabels"])
        {
            say(&at("yticks"), format!("{t:?}"), &w["yticklabels"]);
        }
        let images = w["images"].as_array().unwrap();
        let want_image: Option<Vec<Vec<Option<f64>>>> = images.first().map(|im| {
            im.as_array()
                .unwrap()
                .iter()
                .map(|r| r.as_array().unwrap().iter().map(Value::as_f64).collect())
                .collect()
        });
        if a.image != want_image {
            say(&at("image"), format!("{:?}", a.image), &w["images"]);
        }
    }
}

/// The chart a fixture case draws (None for cases that are errors).
pub fn chart_for(doc: &Value, case: &Value) -> Option<Chart> {
    if case.get("error").is_some() {
        return None;
    }
    let f = frame(&doc["frames"][case["frame"].as_str().unwrap()]);
    let hits: Option<Vec<Vec<bool>>> = case["hits"].as_str().map(|h| {
        let h = &doc["hits"][h];
        f.columns
            .iter()
            .map(|c| h[&c.name].as_array().unwrap().iter().map(|b| b.as_bool().unwrap()).collect())
            .collect()
    });
    wbquery_charts::build(&f, hits.as_deref(), &spec(&case["spec"])).ok()
}

#[test]
fn charts_match_python() {
    let doc = golden();
    let frames = doc["frames"].as_object().unwrap();
    let mut problems = Vec::new();
    let mut drawn = 0;
    for case in doc["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let f = frame(&frames[case["frame"].as_str().unwrap()]);
        let hits: Option<Vec<Vec<bool>>> = case["hits"].as_str().map(|h| {
            let h = &doc["hits"][h];
            f.columns
                .iter()
                .map(|c| h[&c.name].as_array().unwrap().iter().map(|b| b.as_bool().unwrap()).collect())
                .collect()
        });
        let result = wbquery_charts::build(&f, hits.as_deref(), &spec(&case["spec"]));
        match (result, case.get("error")) {
            (Err(e), Some(want)) if e.0 == want.as_str().unwrap() => {}
            (Err(e), want) => problems.push(format!("{name}: error {e}, want {want:?}")),
            (Ok(_), Some(want)) => problems.push(format!("{name}: drew, but Python said {want}")),
            (Ok(chart), None) => {
                drawn += 1;
                compare(name, &chart, case, &mut problems);
            }
        }
    }
    assert!(drawn >= 30, "only {drawn} charts compared");
    assert!(problems.is_empty(), "{} mismatch(es):\n{}", problems.len(), problems.join("\n"));
}

#[test]
fn helpers_match_python() {
    let doc = golden();
    for (col, want) in doc["series_name"].as_object().unwrap() {
        assert_eq!(series_name(col), want.as_str().unwrap(), "{col}");
    }
    let s = Column::inferred(
        "s",
        ["50%", "1,000", "5e7", "abc"]
            .iter()
            .map(|t| Cell::Text(t.to_string()))
            .chain([Cell::Empty, Cell::Int(3), Cell::Bool(true), Cell::Text(" ".into())])
            .collect(),
    );
    let got: Vec<Option<f64>> = numeric(&s).iter().map(|v| (!v.is_nan()).then_some(*v)).collect();
    let want: Vec<Option<f64>> = doc["numeric"].as_array().unwrap().iter().map(Value::as_f64).collect();
    assert_eq!(got, want);
    let df = frame(&doc["frames"]["DF"]);
    for c in &df.columns {
        assert_eq!(is_numeric(c, 0.8), doc["is_numeric"][&c.name].as_bool().unwrap(), "{}", c.name);
    }
    assert_eq!(numeric_columns(&df.columns), strings(&doc["numeric_columns"]));
    let st = imr_stats(&[1.0, 2.0, 3.0, 2.0, 1.0, 9.0, 2.0], 3.0).unwrap();
    let imr = &doc["imr"];
    for (got, key) in
        [(st.mean, "mean"), (st.mr_bar, "mr_bar"), (st.ucl, "ucl"), (st.lcl, "lcl"), (st.mr_ucl, "mr_ucl")]
    {
        assert!(close(got, imr[key].as_f64()), "{key}: {got} vs {}", imr[key]);
    }
    assert!(imr_stats(&[1.0], 3.0).is_err());
    for a in doc["aggregate"].as_array().unwrap() {
        let col = |k: &str| {
            let name = a[k].as_str().unwrap();
            (!name.is_empty()).then(|| df.column(name).unwrap())
        };
        let t = aggregate(col("x").unwrap(), col("y"), col("group"), a["agg"].as_str().unwrap()).unwrap();
        assert_eq!(t.rows, strings(&a["rows"]), "{a}");
        assert_eq!(t.columns, strings(&a["columns"]), "{a}");
        let want: Vec<Vec<Option<f64>>> = a["values"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_array().unwrap().iter().map(Value::as_f64).collect())
            .collect();
        assert_eq!(t.values, want, "{a}");
    }
}
