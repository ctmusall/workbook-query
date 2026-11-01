//! Drawing a [`Chart`] interactively with egui_plot: drag to pan, scroll to zoom, double-click to
//! reset; hovering lists the points under the pointer.

use egui::{Align2, Color32, Id, Pos2, Ui};
use egui_plot::{
    Bar, BarChart, BoxElem, BoxPlot, BoxSpread, GridInput, GridMark, HLine, Legend, Line, LineStyle,
    MarkerShape, Plot, PlotPoint, PlotPoints, Points, Polygon, Text,
};
use wbquery_charts::render::font::text_width;
use wbquery_charts::render::ticks::date_ticks;
use wbquery_charts::scene::{Axis, Chart, Color, Mark, Panel, viridis};
use wbquery_charts::values::format_g;

fn c32(c: Color) -> Color32 {
    Color32::from_rgb(c.0, c.1, c.2)
}

fn with_alpha(c: Color, a: f64) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0, c.1, c.2, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn days_label(v: f64, step: f64) -> String {
    let t = chrono::DateTime::UNIX_EPOCH.naive_utc()
        + chrono::Duration::milliseconds((v * 86_400_000.0).round() as i64);
    if step < 1.0 { t.format("%m-%d %H:%M").to_string() } else { t.format("%Y-%m-%d").to_string() }
}

/// Axis labels for category / tick axes: the label at each whole position.
fn label_at(labels: &[(f64, String)], v: f64) -> String {
    labels.iter().find(|(p, _)| (p - v).abs() < 1e-6).map(|(_, l)| l.clone()).unwrap_or_default()
}

fn axis_labels(axis: &Axis) -> Option<Vec<(f64, String)>> {
    match axis {
        Axis::Category(c) => Some(c.iter().enumerate().map(|(i, s)| (i as f64, s.clone())).collect()),
        Axis::Ticks(t) => Some(t.clone()),
        _ => None,
    }
}

fn fixed_marks(labels: &[(f64, String)], input: GridInput, sign: f64) -> Vec<GridMark> {
    let (lo, hi) = input.bounds;
    labels
        .iter()
        .map(|(p, _)| p * sign)
        .filter(|v| *v >= lo && *v <= hi)
        .map(|value| GridMark { value, step_size: 1.0 })
        .collect()
}

/// Draw `chart` filling the available space; returns how many point labels were left out.
pub fn chart_view(ui: &mut Ui, chart: &Chart, id: Id) -> usize {
    if !chart.title.is_empty() {
        ui.vertical_centered(|ui| ui.heading(&chart.title));
    }
    let total: f64 = chart.panels.iter().map(|p| p.height).sum::<f64>().max(1e-9);
    let height = ui.available_height() - 6.0 * chart.panels.len() as f32;
    let link = id.with("x-link");
    let multi = chart.panels.len() > 1;
    let mut hidden = 0;
    for (k, p) in chart.panels.iter().enumerate() {
        if !p.title.is_empty() {
            ui.label(egui::RichText::new(&p.title).strong());
        }
        let h = (height * (p.height / total) as f32 - if p.title.is_empty() { 0.0 } else { 20.0 }).max(80.0);
        let shared_x = multi.then(|| {
            let lo = chart.panels.iter().map(data_left).fold(f64::INFINITY, f64::min);
            let hi = chart.panels.iter().map(data_right).fold(f64::NEG_INFINITY, f64::max);
            (link, lo, hi)
        });
        hidden += panel_view(ui, p, id.with(k), h, shared_x);
    }
    hidden
}

fn panel_view(ui: &mut Ui, p: &Panel, id: Id, height: f32, shared_x: Option<(Id, f64, f64)>) -> usize {
    let pie = p.marks.iter().any(|m| matches!(m, Mark::Pie { .. }));
    let heat = p.marks.iter().any(|m| matches!(m, Mark::Heatmap { .. }));
    let ysign = if p.y_inverted { -1.0 } else { 1.0 };
    let mut plot = Plot::new(id)
        .height(height)
        .x_axis_label(p.xlabel.clone())
        .y_axis_label(p.ylabel.clone())
        .show_x(false)
        .show_y(false)
        .show_grid(p.grid);
    if p.legend.is_some() {
        plot = plot.legend(Legend::default());
    }
    if let Some((link, lo, hi)) = shared_x {
        // Stacked panels (I-MR) show the same observations.
        plot =
            plot.link_axis(link, [true, false]).link_cursor(link, [true, false]).include_x(lo).include_x(hi);
    }
    if pie {
        plot = plot
            .data_aspect(1.0)
            .show_axes(false)
            .show_grid(false)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false);
    }
    if p.sticky_zero {
        plot = plot.include_y(0.0);
    }
    match &p.x {
        Axis::Date => {
            plot = plot
                .x_grid_spacer(|input| {
                    let (lo, hi) = input.bounds;
                    let ticks = date_ticks(lo, hi, 8);
                    let step = if ticks.len() > 1 { ticks[1].0 - ticks[0].0 } else { 1.0 };
                    ticks.into_iter().map(|(value, _)| GridMark { value, step_size: step }).collect()
                })
                .x_axis_formatter(|mark, _| days_label(mark.value, mark.step_size));
        }
        axis => {
            if let Some(labels) = axis_labels(axis) {
                let l2 = labels.clone();
                plot = plot
                    .x_grid_spacer(move |input| fixed_marks(&labels, input, 1.0))
                    .x_axis_formatter(move |mark, _| label_at(&l2, mark.value));
            }
        }
    }
    if let Some(labels) = axis_labels(&p.y) {
        let l2 = labels.clone();
        plot = plot
            .y_grid_spacer(move |input| fixed_marks(&labels, input, ysign))
            .y_axis_formatter(move |mark, _| label_at(&l2, mark.value * ysign));
    }

    // Limit-line labels sit at the data's right edge (placing them at the plot's edge would widen the
    // automatic bounds every frame).
    let right = data_right(p);
    let mut hidden = 0;
    let resp = plot.show(ui, |pu| {
        for m in &p.marks {
            add_mark(pu, m, ysign, right);
        }
        if heat {
            pu.set_auto_bounds([true, true]);
        }
        hidden = place_labels(pu, p);
    });
    // Hover: every point within 8 px, nearest first.
    if let Some(pos) = resp.response.hover_pos() {
        let mut found: Vec<(f32, &str)> = Vec::new();
        for m in &p.marks {
            let (Mark::Line { points, tooltips, .. } | Mark::Scatter { points, tooltips, .. }) = m else {
                continue;
            };
            for (pt, tip) in points.iter().zip(tooltips) {
                let s = resp.transform.position_from_point(&PlotPoint::new(pt[0], pt[1] * ysign));
                let d = s.distance(pos);
                if d <= 8.0 {
                    found.push((d, tip.as_str()));
                }
            }
        }
        if !found.is_empty() {
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            let more = found.len().saturating_sub(8);
            let mut text: Vec<&str> = found.iter().take(8).map(|f| f.1).collect();
            let tail = format!("… and {more} more");
            if more > 0 {
                text.push(&tail);
            }
            let joined = text.join("\n\n");
            resp.response.on_hover_ui_at_pointer(|ui| {
                ui.label(joined);
            });
        }
    }
    if let Some(cb) = &p.colorbar {
        colorbar(ui, cb);
    }
    hidden
}

/// The smallest x of a panel's data.
fn data_left(p: &Panel) -> f64 {
    let mut left = f64::INFINITY;
    for m in &p.marks {
        if let Mark::Line { points, .. } | Mark::Scatter { points, .. } = m {
            left = points.iter().map(|q| q[0]).filter(|x| x.is_finite()).fold(left, f64::min);
        }
    }
    if left.is_finite() { left } else { 0.0 }
}

/// The largest x of a panel's data.
fn data_right(p: &Panel) -> f64 {
    let mut right = f64::NEG_INFINITY;
    for m in &p.marks {
        match m {
            Mark::Line { points, .. } | Mark::Scatter { points, .. } => {
                right = points.iter().map(|q| q[0]).filter(|x| x.is_finite()).fold(right, f64::max);
            }
            Mark::Bars { rects, .. } => right = rects.iter().map(|r| r.x + r.w).fold(right, f64::max),
            Mark::Boxes { boxes, .. } => right = boxes.iter().map(|b| b.pos + b.width).fold(right, f64::max),
            _ => {}
        }
    }
    if right.is_finite() { right } else { 1.0 }
}

fn add_mark(pu: &mut egui_plot::PlotUi<'_>, m: &Mark, ysign: f64, right: f64) {
    match m {
        Mark::Line { label, color, points, .. } => {
            let name = label.clone().unwrap_or_default();
            // NaN breaks the line into segments.
            let mut seg: Vec<[f64; 2]> = Vec::new();
            let flush = |seg: &mut Vec<[f64; 2]>, pu: &mut egui_plot::PlotUi<'_>| {
                if !seg.is_empty() {
                    pu.line(
                        Line::new(name.clone(), PlotPoints::from(std::mem::take(seg)))
                            .color(c32(*color))
                            .width(1.8),
                    );
                }
            };
            for p in points {
                if p[0].is_finite() && p[1].is_finite() {
                    seg.push(*p);
                } else {
                    flush(&mut seg, pu);
                }
            }
            flush(&mut seg, pu);
            let finite: Vec<[f64; 2]> =
                points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).copied().collect();
            pu.points(
                Points::new(name, PlotPoints::from(finite)).color(c32(*color)).radius(2.5).filled(true),
            );
        }
        Mark::Scatter { label, color, points, size, alpha, .. } => {
            pu.points(
                Points::new(label.clone().unwrap_or_default(), PlotPoints::from(points.clone()))
                    .color(with_alpha(*color, *alpha))
                    .radius((size.sqrt() * 0.75) as f32)
                    .shape(MarkerShape::Circle)
                    .filled(true),
            );
        }
        Mark::Bars { label, color, rects, alpha, edge } => {
            let bars = rects
                .iter()
                .map(|r| {
                    let mut b = Bar::new(r.x + r.w / 2.0, r.h)
                        .width(r.w)
                        .base_offset(r.y)
                        .fill(with_alpha(*color, *alpha));
                    if let Some(e) = edge {
                        b = b.stroke(egui::Stroke::new(0.6, c32(*e)));
                    } else {
                        b = b.stroke(egui::Stroke::new(0.5, c32(*color)));
                    }
                    b
                })
                .collect();
            pu.bar_chart(BarChart::new(label.clone().unwrap_or_default(), bars).color(c32(*color)));
        }
        Mark::HLine { y, color, dashed, text } => {
            let style = if *dashed { LineStyle::Dashed { length: 8.0 } } else { LineStyle::Solid };
            pu.hline(HLine::new("", *y).color(c32(*color)).style(style).width(1.2));
            if let Some(t) = text {
                pu.text(
                    Text::new("", PlotPoint::new(right, *y), t.as_str())
                        .color(c32(*color))
                        .anchor(Align2::RIGHT_BOTTOM),
                );
            }
        }
        Mark::Boxes { color, boxes } => {
            let elems = boxes
                .iter()
                .map(|b| {
                    let s = &b.stats;
                    BoxElem::new(b.pos, BoxSpread::new(s.whisker_lo, s.q1, s.median, s.q3, s.whisker_hi))
                        .box_width(b.width)
                        .whisker_width(b.width / 2.0)
                        .fill(with_alpha(*color, 0.35))
                        .stroke(egui::Stroke::new(1.0, Color32::BLACK))
                })
                .collect();
            pu.box_plot(BoxPlot::new("", elems));
            let fliers: Vec<[f64; 2]> =
                boxes.iter().flat_map(|b| b.stats.fliers.iter().map(move |f| [b.pos, *f])).collect();
            if !fliers.is_empty() {
                pu.points(
                    Points::new("", PlotPoints::from(fliers)).radius(3.5).filled(false).color(Color32::BLACK),
                );
            }
        }
        Mark::Pie { slices } => {
            for s in slices {
                let steps = ((s.theta2 - s.theta1).abs() / 2.0).ceil().max(1.0) as usize;
                let mut pts = vec![[0.0, 0.0]];
                for k in 0..=steps {
                    let a = (s.theta1 + (s.theta2 - s.theta1) * k as f64 / steps as f64).to_radians();
                    pts.push([a.cos(), a.sin()]);
                }
                pu.polygon(
                    Polygon::new(s.label.clone(), PlotPoints::from(pts))
                        .fill_color(c32(s.color))
                        .stroke(egui::Stroke::new(1.0, Color32::WHITE)),
                );
                let mid = ((s.theta1 + s.theta2) / 2.0).to_radians();
                let anchor = if mid.cos() >= 0.0 { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER };
                pu.text(
                    Text::new("", PlotPoint::new(1.12 * mid.cos(), 1.12 * mid.sin()), s.label.as_str())
                        .anchor(anchor),
                );
                pu.text(Text::new("", PlotPoint::new(0.6 * mid.cos(), 0.6 * mid.sin()), s.pct.as_str()));
            }
        }
        Mark::Heatmap { values, min, max, texts } => {
            let span = if max > min { max - min } else { 1.0 };
            for (r, row) in values.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    let Some(v) = v else { continue };
                    let (x0, x1) = (c as f64 - 0.5, c as f64 + 0.5);
                    let (y0, y1) = ((r as f64 - 0.5) * ysign, (r as f64 + 0.5) * ysign);
                    pu.polygon(
                        Polygon::new("", PlotPoints::from(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]))
                            .fill_color(c32(viridis((v - min) / span)))
                            .stroke(egui::Stroke::NONE),
                    );
                }
            }
            for t in texts {
                let color = if t.dark { Color32::BLACK } else { Color32::WHITE };
                pu.text(
                    Text::new("", PlotPoint::new(t.col as f64, t.row as f64 * ysign), t.text.as_str())
                        .color(color),
                );
            }
        }
    }
}

/// Place point labels at the first free corner around their point (inside the plot, not over another
/// label); returns how many didn't fit.
fn place_labels(pu: &mut egui_plot::PlotUi<'_>, p: &Panel) -> usize {
    if p.labels.is_empty() {
        return 0;
    }
    let tr = *pu.transform();
    let area = *tr.frame();
    let size = 11.0f32;
    let off = 5.0f32;
    let mut kept: Vec<egui::Rect> = Vec::new();
    for l in &p.labels {
        let s: Pos2 = tr.position_from_point(&PlotPoint::new(l.x, l.y));
        let w = text_width(&l.text, size as f64) as f32;
        for (dx, dy, align) in [
            (off, -off, Align2::LEFT_BOTTOM),
            (-off, -off, Align2::RIGHT_BOTTOM),
            (off, off, Align2::LEFT_TOP),
            (-off, off, Align2::RIGHT_TOP),
        ] {
            let anchor = Pos2::new(s.x + dx, s.y + dy);
            let rect = align.anchor_size(anchor, egui::vec2(w, size)).expand(4.0);
            if area.shrink(-4.0).contains_rect(rect) && !kept.iter().any(|k| k.intersects(rect)) {
                kept.push(rect);
                let at = tr.value_from_position(anchor);
                pu.text(Text::new("", at, l.text.as_str()).anchor(align).color(Color32::from_gray(40)));
                break;
            }
        }
    }
    p.labels_total - kept.len()
}

fn colorbar(ui: &mut Ui, cb: &wbquery_charts::scene::Colorbar) {
    ui.horizontal(|ui| {
        ui.label(format_g(cb.min, 4));
        let (rect, _) = ui.allocate_exact_size(egui::vec2(220.0, 12.0), egui::Sense::hover());
        let steps = 64;
        for i in 0..steps {
            let t = i as f32 / steps as f32;
            let r = egui::Rect::from_min_max(
                Pos2::new(rect.left() + t * rect.width(), rect.top()),
                Pos2::new(rect.left() + (t + 1.0 / steps as f32) * rect.width() + 0.5, rect.bottom()),
            );
            ui.painter().rect_filled(r, 0.0, c32(viridis(t as f64)));
        }
        ui.label(format_g(cb.max, 4));
        ui.label(&cb.label);
    });
}
