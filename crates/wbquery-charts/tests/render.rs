//! Every chart from the parity fixtures renders to SVG, PNG and PDF.
//! Set WBQ_CHART_DIR to keep the files for a look.

use std::path::PathBuf;

use serde_json::Value;
use wbquery_charts::render::{self, DEFAULT_SIZE};

#[path = "golden.rs"]
#[allow(dead_code)]
mod golden;

#[test]
fn every_chart_saves_in_every_format() {
    let doc: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/charts.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let keep = std::env::var_os("WBQ_CHART_DIR").map(PathBuf::from);
    let tmp = tempfile::tempdir().unwrap();
    let dir = keep.clone().unwrap_or_else(|| tmp.path().to_path_buf());
    std::fs::create_dir_all(&dir).unwrap();
    let mut saved = 0;
    for case in doc["cases"].as_array().unwrap() {
        let Some(chart) = golden::chart_for(&doc, case) else { continue };
        let name = case["name"].as_str().unwrap();
        for ext in ["svg", "png", "pdf"] {
            let out = render::save(&chart, dir.join(format!("{name}.{ext}")), DEFAULT_SIZE).unwrap();
            let bytes = std::fs::read(&out.path).unwrap();
            let magic: &[u8] = match ext {
                "svg" => b"<svg",
                "png" => b"\x89PNG",
                _ => b"%PDF",
            };
            assert!(bytes.starts_with(magic), "{name}.{ext}");
            saved += 1;
        }
    }
    assert!(saved > 90);
    // No extension, or an unknown one: PNG with .png added.
    let chart = golden::chart_for(&doc, &doc["cases"][0]).unwrap();
    let out = render::save(&chart, dir.join("plain"), DEFAULT_SIZE).unwrap();
    assert_eq!(out.path.file_name().unwrap(), "plain.png");
}

#[test]
fn crowded_labels_are_thinned() {
    use wbquery_charts::{ChartSpec, build};
    use wbquery_core::cell::{Cell, Column, Frame};
    let n = 60;
    let day = |i: i64| {
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()
            + chrono::Duration::days(i)
    };
    let frame = Frame {
        columns: vec![
            Column::inferred("Date", (0..n).map(|i| Cell::Date(day(i))).collect()),
            Column::inferred(
                "Hand 1",
                (0..n).map(|i| Cell::Text(format!("1 Staphylococcus species{i}"))).collect(),
            ),
            Column::inferred(
                "Hand 2",
                std::iter::once(Cell::Text("4 Micrococcus luteus".into()))
                    .chain((1..n).map(|_| Cell::Int(0)))
                    .collect(),
            ),
        ],
        index: (0..n).collect(),
    };
    let spec = ChartSpec {
        x: "Date".into(),
        y: "Hand 1".into(),
        ys: vec!["Hand 2".into()],
        point_labels: true,
        ..ChartSpec::new("scatter")
    };
    let chart = build(&frame, None, &spec).unwrap();
    assert_eq!(chart.panels[0].labels[0].text, "Micrococcus luteus"); // highest value first
    let out = render::render_svg(&chart, 800.0, 400.0);
    assert!(out.hidden_labels > 0 && out.hidden_labels < n as usize);
    assert!(out.svg.contains("Micrococcus luteus"));
    assert!(render::hidden_note(out.hidden_labels).unwrap().contains("hidden so labels don't overlap"));
}
