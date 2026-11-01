//! Laying out and drawing a [`Chart`] as SVG.
//!
//! Sizes follow matplotlib's defaults at 100 pixels per inch: 10 pt tick and axis labels, a 12 pt
//! title, 8 pt legends. Point labels are placed at the first free corner around their point and left
//! out when every corner would overlap another label or the legend.

use std::fmt::Write as _;

use crate::render::font::{family, text_width};
use crate::render::ticks::{date_ticks, nice_ticks, number_labels};
use crate::scene::{Axis, Chart, LegendKind, Mark, Panel, viridis};

/// Pixels per inch of the SVG coordinate system.
pub const DPI: f64 = 100.0;

fn pt(v: f64) -> f64 {
    v * DPI / 72.0
}

/// An SVG document, and how many point labels were left out so the rest don't overlap.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub svg: String,
    pub hidden_labels: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Box2 {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Box2 {
    fn w(&self) -> f64 {
        self.x1 - self.x0
    }
    fn h(&self) -> f64 {
        self.y1 - self.y0
    }
    fn padded(&self, p: f64) -> Box2 {
        Box2 { x0: self.x0 - p, y0: self.y0 - p, x1: self.x1 + p, y1: self.y1 + p }
    }
    fn overlaps(&self, o: &Box2) -> bool {
        self.x0 < o.x1 && o.x0 < self.x1 && self.y0 < o.y1 && o.y0 < self.y1
    }
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Escaped text, with a letter followed by a combining macron (`X̄`, `MR̄`) drawn overlined, since
/// the font has no combining marks.
fn text_content(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars.get(i + 1) == Some(&'\u{304}') {
            out.push_str(&format!(
                r#"<tspan text-decoration="overline">{}</tspan>"#,
                esc(&chars[i].to_string())
            ));
            i += 2;
        } else {
            out.push_str(&esc(&chars[i].to_string()));
            i += 1;
        }
    }
    out
}

struct Svg {
    out: String,
    clip: usize,
}

#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    fn name(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        }
    }
}

impl Svg {
    #[allow(clippy::too_many_arguments)]
    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64, extra: &str) {
        let _ = writeln!(
            self.out,
            r#"<line x1="{x1:.2}" y1="{y1:.2}" x2="{x2:.2}" y2="{y2:.2}" stroke="{color}" stroke-width="{width:.2}"{extra}/>"#
        );
    }

    fn rect(&mut self, b: Box2, fill: &str, extra: &str) {
        let _ = writeln!(
            self.out,
            r#"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{fill}"{extra}/>"#,
            b.x0,
            b.y0,
            b.w().max(0.0),
            b.h().max(0.0)
        );
    }

    fn circle(&mut self, x: f64, y: f64, r: f64, fill: &str, extra: &str) {
        let _ = writeln!(self.out, r#"<circle cx="{x:.2}" cy="{y:.2}" r="{r:.2}" fill="{fill}"{extra}/>"#);
    }

    /// Text whose vertical middle is at `y` (so it needs no baseline support from the reader).
    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, x: f64, y: f64, size: f64, anchor: Anchor, color: &str, rotate: f64, s: &str) {
        let baseline = y + 0.35 * size;
        let rot = if rotate != 0.0 {
            format!(r#" transform="rotate({rotate:.1} {x:.2} {y:.2})""#)
        } else {
            String::new()
        };
        let _ = writeln!(
            self.out,
            r#"<text x="{x:.2}" y="{baseline:.2}" font-size="{size:.2}" text-anchor="{}" fill="{color}"{rot}>{}</text>"#,
            anchor.name(),
            text_content(s)
        );
    }

    fn begin_clip(&mut self, b: Box2) {
        self.clip += 1;
        let _ = writeln!(
            self.out,
            r#"<clipPath id="c{}"><rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}"/></clipPath><g clip-path="url(#c{})">"#,
            self.clip,
            b.x0,
            b.y0,
            b.w(),
            b.h(),
            self.clip
        );
    }

    fn end_group(&mut self) {
        self.out.push_str("</g>\n");
    }
}

/// Where values land on screen for one panel.
struct Frame {
    plot: Box2,
    x: (f64, f64),
    y: (f64, f64),
    y_inverted: bool,
}

impl Frame {
    fn sx(&self, v: f64) -> f64 {
        self.plot.x0 + (v - self.x.0) / (self.x.1 - self.x.0) * self.plot.w()
    }
    fn sy(&self, v: f64) -> f64 {
        let t = (v - self.y.0) / (self.y.1 - self.y.0);
        if self.y_inverted { self.plot.y0 + t * self.plot.h() } else { self.plot.y1 - t * self.plot.h() }
    }
}

fn is_pie(p: &Panel) -> bool {
    p.marks.iter().any(|m| matches!(m, Mark::Pie { .. }))
}

fn is_heatmap(p: &Panel) -> bool {
    p.marks.iter().any(|m| matches!(m, Mark::Heatmap { .. }))
}

fn category_count(axis: &Axis) -> Option<usize> {
    match axis {
        Axis::Category(c) => Some(c.len()),
        _ => None,
    }
}

/// The data range of a panel's marks: (x min, x max, y min, y max).
fn bounds(p: &Panel) -> (f64, f64, f64, f64) {
    let (mut x0, mut x1, mut y0, mut y1) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    let add_x = |v: f64, x0: &mut f64, x1: &mut f64| {
        if v.is_finite() {
            *x0 = x0.min(v);
            *x1 = x1.max(v);
        }
    };
    for m in &p.marks {
        match m {
            Mark::Line { points, .. } | Mark::Scatter { points, .. } => {
                for [x, y] in points {
                    if x.is_finite() && y.is_finite() {
                        add_x(*x, &mut x0, &mut x1);
                        add_x(*y, &mut y0, &mut y1);
                    }
                }
            }
            Mark::Bars { rects, .. } => {
                for r in rects {
                    add_x(r.x, &mut x0, &mut x1);
                    add_x(r.x + r.w, &mut x0, &mut x1);
                    add_x(r.y, &mut y0, &mut y1);
                    add_x(r.y + r.h, &mut y0, &mut y1);
                }
            }
            Mark::HLine { y, .. } => add_x(*y, &mut y0, &mut y1),
            Mark::Boxes { boxes, .. } => {
                for b in boxes {
                    add_x(b.pos - b.width / 2.0, &mut x0, &mut x1);
                    add_x(b.pos + b.width / 2.0, &mut x0, &mut x1);
                    add_x(b.stats.whisker_lo, &mut y0, &mut y1);
                    add_x(b.stats.whisker_hi, &mut y0, &mut y1);
                    for f in &b.stats.fliers {
                        add_x(*f, &mut y0, &mut y1);
                    }
                }
            }
            Mark::Heatmap { .. } | Mark::Pie { .. } => {}
        }
    }
    (x0, x1, y0, y1)
}

fn with_margin(lo: f64, hi: f64, sticky_zero: bool) -> (f64, f64) {
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if hi == lo {
        let d = if lo == 0.0 { 0.5 } else { lo.abs() * 0.05 };
        return (lo - d, hi + d);
    }
    let m = (hi - lo) * 0.05;
    let lo2 = if sticky_zero && lo >= 0.0 { lo } else { lo - m };
    let hi2 = if sticky_zero && hi <= 0.0 { hi } else { hi + m };
    (lo2, hi2)
}

struct Ticks {
    items: Vec<(f64, String)>,
    rotate: f64,
}

fn axis_ticks(axis: &Axis, range: (f64, f64), length: f64, integer: bool) -> Ticks {
    match axis {
        Axis::Number => {
            let t = nice_ticks(range.0, range.1, (length / 80.0).clamp(3.0, 10.0) as usize, integer);
            let labels = number_labels(&t);
            Ticks { items: t.into_iter().zip(labels).collect(), rotate: 0.0 }
        }
        Axis::Date => Ticks {
            items: date_ticks(range.0, range.1, (length / 90.0).clamp(2.0, 10.0) as usize),
            rotate: 30.0,
        },
        Axis::Category(c) => {
            if c.len() > crate::stats::MAX_CATEGORIES {
                return Ticks { items: Vec::new(), rotate: 0.0 };
            }
            let longest = c.iter().map(|s| s.chars().count()).max().unwrap_or(0);
            let rotate = if longest > 6 || c.len() > 8 { 45.0 } else { 0.0 };
            Ticks { items: c.iter().enumerate().map(|(i, s)| (i as f64, s.clone())).collect(), rotate }
        }
        Axis::Ticks(t) => Ticks { items: t.clone(), rotate: if t.len() > 8 { 45.0 } else { 0.0 } },
    }
}

/// How far rotated tick labels reach below the axis.
fn tick_depth(t: &Ticks, size: f64) -> f64 {
    let a = t.rotate.to_radians();
    let w = t.items.iter().map(|(_, s)| text_width(s, size)).fold(0.0, f64::max);
    if t.rotate == 0.0 { size } else { w * a.sin() + size * a.cos() }
}

struct Sizes {
    tick: f64,
    label: f64,
    title: f64,
    small: f64,
    point: f64,
}

/// Draw `chart` as an SVG document `width` × `height` pixels.
pub fn render_svg(chart: &Chart, width: f64, height: f64) -> Rendered {
    let s = Sizes { tick: pt(10.0), label: pt(10.0), title: pt(12.0), small: pt(8.0), point: pt(7.0) };
    let mut svg = Svg { out: String::new(), clip: 0 };
    let font = family();
    let _ = writeln!(
        svg.out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.0} {height:.0}" font-family="{}, Ubuntu, 'DejaVu Sans', Arial, sans-serif">"#,
        esc(&font)
    );
    svg.rect(Box2 { x0: 0.0, y0: 0.0, x1: width, y1: height }, "#ffffff", "");

    let mut top = 10.0;
    if !chart.title.is_empty() {
        svg.text(width / 2.0, top + s.title * 0.6, s.title, Anchor::Middle, "#000000", 0.0, &chart.title);
        top += s.title * 1.6;
    }

    // Ranges and ticks first, so every panel can share one left and right margin.
    struct Plan {
        x: (f64, f64),
        y: (f64, f64),
        xt: Ticks,
        yt: Ticks,
        top_pad: f64,
        bottom_pad: f64,
    }
    let approx_w = width * 0.75;
    let total_ratio: f64 = chart.panels.iter().map(|p| p.height).sum::<f64>().max(1e-9);
    let mut plans: Vec<Plan> = Vec::new();
    let (mut left, mut right) = (20.0f64, 16.0f64);
    for p in &chart.panels {
        let approx_h = (height - top) * p.height / total_ratio * 0.7;
        let (bx0, bx1, by0, by1) = bounds(p);
        let (x, y) = if is_heatmap(p) {
            let nx = category_count(&p.x).unwrap_or(1) as f64;
            let ny = category_count(&p.y).unwrap_or(1) as f64;
            ((-0.5, nx - 0.5), (-0.5, ny - 0.5))
        } else {
            let mut x = with_margin(bx0, bx1, false);
            if let Some(n) = category_count(&p.x)
                && !bx0.is_finite()
            {
                x = (-0.5, n as f64 - 0.5);
            }
            (x, with_margin(by0, by1, p.sticky_zero))
        };
        let xt = axis_ticks(&p.x, x, approx_w, p.integer_x);
        let yt = axis_ticks(&p.y, y, approx_h, false);
        let pie = is_pie(p);
        if !pie {
            let ytw = yt.items.iter().map(|(_, l)| text_width(l, s.tick)).fold(0.0, f64::max);
            left = left.max(ytw + 8.0 + if p.ylabel.is_empty() { 0.0 } else { s.label * 1.5 } + 8.0);
        }
        let hline_w = p
            .marks
            .iter()
            .filter_map(|m| {
                if let Mark::HLine { text: Some(t), .. } = m { Some(text_width(t, s.small)) } else { None }
            })
            .fold(0.0, f64::max);
        right = right.max(if hline_w > 0.0 { hline_w + 12.0 } else { 0.0 });
        if let Some(cb) = &p.colorbar {
            let labels = number_labels(&nice_ticks(cb.min, cb.max, 6, false));
            let lw = labels.iter().map(|l| text_width(l, s.tick)).fold(0.0, f64::max);
            right = right.max(12.0 + 14.0 + 6.0 + lw + 8.0 + s.label * 1.5);
        }
        let top_pad = if p.title.is_empty() { 8.0 } else { s.label * 1.6 };
        let bottom_pad = if pie {
            8.0 + if p.xlabel.is_empty() { 0.0 } else { s.label * 1.6 }
        } else {
            6.0 + tick_depth(&xt, s.tick) + 6.0 + if p.xlabel.is_empty() { 0.0 } else { s.label * 1.6 }
        };
        plans.push(Plan { x, y, xt, yt, top_pad, bottom_pad });
    }

    let fixed: f64 = plans.iter().map(|p| p.top_pad + p.bottom_pad).sum();
    let free = (height - top - 8.0 - fixed).max(40.0);
    let mut hidden = 0;
    let mut y_cursor = top;
    for (p, plan) in chart.panels.iter().zip(plans) {
        let h = free * p.height / total_ratio;
        let plot = Box2 {
            x0: left,
            y0: y_cursor + plan.top_pad,
            x1: width - right,
            y1: y_cursor + plan.top_pad + h,
        };
        y_cursor = plot.y1 + plan.bottom_pad;
        if !p.title.is_empty() {
            svg.text(plot.x0, plot.y0 - s.label * 0.7, s.label, Anchor::Start, "#000000", 0.0, &p.title);
        }
        if is_pie(p) {
            draw_pie(&mut svg, p, plot, &s);
            continue;
        }
        let frame = Frame { plot, x: plan.x, y: plan.y, y_inverted: p.y_inverted };
        hidden += draw_panel(&mut svg, p, &frame, &plan.xt, &plan.yt, &s);
    }
    svg.out.push_str("</svg>\n");
    Rendered { svg: svg.out, hidden_labels: hidden }
}

fn draw_panel(svg: &mut Svg, p: &Panel, f: &Frame, xt: &Ticks, yt: &Ticks, s: &Sizes) -> usize {
    let plot = f.plot;
    let in_x = |v: f64| v >= f.x.0.min(f.x.1) - 1e-9 && v <= f.x.0.max(f.x.1) + 1e-9;
    let in_y = |v: f64| v >= f.y.0.min(f.y.1) - 1e-9 && v <= f.y.0.max(f.y.1) + 1e-9;
    if p.grid {
        for (v, _) in xt.items.iter().filter(|(v, _)| in_x(*v)) {
            let x = f.sx(*v);
            svg.line(x, plot.y0, x, plot.y1, "#b0b0b0", 0.8, r#" stroke-opacity="0.3""#);
        }
        for (v, _) in yt.items.iter().filter(|(v, _)| in_y(*v)) {
            let y = f.sy(*v);
            svg.line(plot.x0, y, plot.x1, y, "#b0b0b0", 0.8, r#" stroke-opacity="0.3""#);
        }
    }

    svg.begin_clip(plot);
    for m in &p.marks {
        draw_mark(svg, m, f);
    }
    svg.end_group();
    // Limit-line labels sit just outside the plot, so they're drawn unclipped.
    for m in &p.marks {
        if let Mark::HLine { y, color, text: Some(t), .. } = m {
            svg.text(plot.x1 + 4.0, f.sy(*y), s.small, Anchor::Start, &color.hex(), 0.0, t);
        }
    }

    svg.rect(plot, "none", r##" stroke="#000000" stroke-width="0.8""##);
    // Ticks and their labels.
    for (v, label) in xt.items.iter().filter(|(v, _)| in_x(*v)) {
        let x = f.sx(*v);
        svg.line(x, plot.y1, x, plot.y1 + 3.5, "#000000", 0.8, "");
        if xt.rotate == 0.0 {
            svg.text(x, plot.y1 + 6.0 + s.tick * 0.5, s.tick, Anchor::Middle, "#000000", 0.0, label);
        } else {
            svg.text(x, plot.y1 + 6.0 + s.tick * 0.35, s.tick, Anchor::End, "#000000", -xt.rotate, label);
        }
    }
    let mut ytw: f64 = 0.0;
    for (v, label) in yt.items.iter().filter(|(v, _)| in_y(*v)) {
        let y = f.sy(*v);
        svg.line(plot.x0 - 3.5, y, plot.x0, y, "#000000", 0.8, "");
        svg.text(plot.x0 - 6.0, y, s.tick, Anchor::End, "#000000", 0.0, label);
        ytw = ytw.max(text_width(label, s.tick));
    }
    if !p.xlabel.is_empty() {
        let y = plot.y1 + 6.0 + tick_depth(xt, s.tick) + 6.0 + s.label * 0.6;
        svg.text((plot.x0 + plot.x1) / 2.0, y, s.label, Anchor::Middle, "#000000", 0.0, &p.xlabel);
    }
    if !p.ylabel.is_empty() {
        let x = plot.x0 - 6.0 - ytw - 8.0 - s.label * 0.5;
        svg.text(x, (plot.y0 + plot.y1) / 2.0, s.label, Anchor::Middle, "#000000", -90.0, &p.ylabel);
    }
    if let Some(cb) = &p.colorbar {
        draw_colorbar(svg, cb, plot, s);
    }
    let legend = p.legend.as_ref().map(|title| draw_legend(svg, p, title, f, s));
    place_labels(svg, p, f, legend, s)
}

fn draw_mark(svg: &mut Svg, m: &Mark, f: &Frame) {
    match m {
        Mark::Line { color, points, .. } => {
            let c = color.hex();
            let mut d = String::new();
            let mut pen = false;
            for [x, y] in points {
                if !(x.is_finite() && y.is_finite()) {
                    pen = false;
                    continue;
                }
                let _ = write!(d, "{}{:.2},{:.2} ", if pen { "L" } else { "M" }, f.sx(*x), f.sy(*y));
                pen = true;
            }
            if !d.is_empty() {
                let _ = writeln!(
                    svg.out,
                    r#"<path d="{d}" fill="none" stroke="{c}" stroke-width="{:.2}" stroke-linejoin="round"/>"#,
                    pt(1.3)
                );
            }
            for [x, y] in points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
                svg.circle(f.sx(*x), f.sy(*y), pt(1.5), &c, "");
            }
        }
        Mark::Scatter { color, points, size, alpha, .. } => {
            let extra = format!(r#" fill-opacity="{alpha:.2}""#);
            for [x, y] in points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
                svg.circle(f.sx(*x), f.sy(*y), pt(size.sqrt()) / 2.0, &color.hex(), &extra);
            }
        }
        Mark::Bars { color, rects, alpha, edge, .. } => {
            let extra = match edge {
                Some(e) => format!(r#" fill-opacity="{alpha:.2}" stroke="{}" stroke-width="0.6""#, e.hex()),
                None => format!(r#" fill-opacity="{alpha:.2}""#),
            };
            for r in rects {
                let (xa, xb) = (f.sx(r.x), f.sx(r.x + r.w));
                let (ya, yb) = (f.sy(r.y), f.sy(r.y + r.h));
                let b = Box2 { x0: xa.min(xb), y0: ya.min(yb), x1: xa.max(xb), y1: ya.max(yb) };
                svg.rect(b, &color.hex(), &extra);
            }
        }
        Mark::HLine { y, color, dashed, .. } => {
            let dash = if *dashed { r#" stroke-dasharray="5,3""# } else { "" };
            let y = f.sy(*y);
            svg.line(f.plot.x0, y, f.plot.x1, y, &color.hex(), 1.2, dash);
        }
        Mark::Boxes { color, boxes } => {
            for b in boxes {
                let st = &b.stats;
                let (cx, hw) = (f.sx(b.pos), (f.sx(b.pos + b.width / 2.0) - f.sx(b.pos)).abs());
                let qw = hw / 2.0;
                let (y1, y3) = (f.sy(st.q1), f.sy(st.q3));
                svg.line(cx, y1, cx, f.sy(st.whisker_lo), "#000000", 1.0, "");
                svg.line(cx, y3, cx, f.sy(st.whisker_hi), "#000000", 1.0, "");
                svg.line(cx - qw, f.sy(st.whisker_lo), cx + qw, f.sy(st.whisker_lo), "#000000", 1.0, "");
                svg.line(cx - qw, f.sy(st.whisker_hi), cx + qw, f.sy(st.whisker_hi), "#000000", 1.0, "");
                let rect = Box2 { x0: cx - hw, y0: y1.min(y3), x1: cx + hw, y1: y1.max(y3) };
                svg.rect(rect, &color.hex(), r##" fill-opacity="0.35" stroke="#000000" stroke-width="1""##);
                svg.line(cx - hw, f.sy(st.median), cx + hw, f.sy(st.median), "#000000", 1.4, "");
                for v in &st.fliers {
                    svg.circle(cx, f.sy(*v), pt(3.0), "none", r##" stroke="#000000" stroke-width="0.8""##);
                }
            }
        }
        Mark::Heatmap { values, min, max, texts } => {
            let span = if max > min { max - min } else { 1.0 };
            for (r, row) in values.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    let Some(v) = v else { continue };
                    let (xa, xb) = (f.sx(c as f64 - 0.5), f.sx(c as f64 + 0.5));
                    let (ya, yb) = (f.sy(r as f64 - 0.5), f.sy(r as f64 + 0.5));
                    let b = Box2 { x0: xa.min(xb), y0: ya.min(yb), x1: xa.max(xb), y1: ya.max(yb) };
                    svg.rect(b, &viridis((v - min) / span).hex(), "");
                }
            }
            for t in texts {
                let color = if t.dark { "#000000" } else { "#ffffff" };
                svg.text(
                    f.sx(t.col as f64),
                    f.sy(t.row as f64),
                    pt(8.0),
                    Anchor::Middle,
                    color,
                    0.0,
                    &t.text,
                );
            }
        }
        Mark::Pie { .. } => {}
    }
}

fn draw_colorbar(svg: &mut Svg, cb: &crate::scene::Colorbar, plot: Box2, s: &Sizes) {
    let bar = Box2 { x0: plot.x1 + 12.0, y0: plot.y0, x1: plot.x1 + 26.0, y1: plot.y1 };
    let steps = 64;
    for i in 0..steps {
        let t0 = i as f64 / steps as f64;
        let y1 = bar.y1 - t0 * bar.h();
        let y0 = bar.y1 - (i + 1) as f64 / steps as f64 * bar.h();
        svg.rect(
            Box2 { x0: bar.x0, y0: y0 - 0.3, x1: bar.x1, y1 },
            &viridis(t0 + 0.5 / steps as f64).hex(),
            "",
        );
    }
    svg.rect(bar, "none", r##" stroke="#000000" stroke-width="0.8""##);
    let span = if cb.max > cb.min { cb.max - cb.min } else { 1.0 };
    let ticks = nice_ticks(cb.min, cb.max, 6, false);
    let labels = number_labels(&ticks);
    let mut lw: f64 = 0.0;
    for (v, l) in ticks.iter().zip(&labels) {
        let y = bar.y1 - (v - cb.min) / span * bar.h();
        svg.line(bar.x1, y, bar.x1 + 3.5, y, "#000000", 0.8, "");
        svg.text(bar.x1 + 6.0, y, s.tick, Anchor::Start, "#000000", 0.0, l);
        lw = lw.max(text_width(l, s.tick));
    }
    svg.text(
        bar.x1 + 6.0 + lw + 8.0 + s.label * 0.5,
        (bar.y0 + bar.y1) / 2.0,
        s.label,
        Anchor::Middle,
        "#000000",
        -90.0,
        &cb.label,
    );
}

/// How much a legend at `b` would cover: data points inside it count heavily, bars by area.
fn legend_cost(p: &Panel, f: &Frame, b: &Box2) -> f64 {
    let mut cost = 0.0;
    for m in &p.marks {
        match m {
            Mark::Line { points, .. } | Mark::Scatter { points, .. } => {
                let inside = points
                    .iter()
                    .filter(|q| q[0].is_finite() && q[1].is_finite() && b.contains(f.sx(q[0]), f.sy(q[1])))
                    .count();
                cost += 400.0 * inside as f64;
            }
            Mark::Bars { rects, .. } => {
                for r in rects {
                    let (xa, xb) = (f.sx(r.x), f.sx(r.x + r.w));
                    let (ya, yb) = (f.sy(r.y), f.sy(r.y + r.h));
                    let w = (b.x1.min(xa.max(xb)) - b.x0.max(xa.min(xb))).max(0.0);
                    let h = (b.y1.min(ya.max(yb)) - b.y0.max(ya.min(yb))).max(0.0);
                    cost += w * h;
                }
            }
            _ => {}
        }
    }
    cost
}

fn draw_legend(svg: &mut Svg, p: &Panel, title: &str, f: &Frame, s: &Sizes) -> Box2 {
    let entries = p.legend_entries();
    let size = s.small;
    let row = size * 1.5;
    let swatch = 22.0;
    let text_w = entries.iter().map(|(l, _, _)| text_width(l, size)).fold(0.0, f64::max);
    let title_rows = if title.is_empty() { 0.0 } else { 1.0 };
    let w = (swatch + 6.0 + text_w).max(text_width(title, size)) + 14.0;
    let h = (entries.len() as f64 + title_rows) * row + 8.0;
    let inset = 6.0;
    let plot = f.plot;
    // Matplotlib's "best" candidates, in its order of preference.
    let (cx, cy) = ((plot.x0 + plot.x1 - w) / 2.0, (plot.y0 + plot.y1 - h) / 2.0);
    let (left, right, top, bottom) =
        (plot.x0 + inset, plot.x1 - inset - w, plot.y0 + inset, plot.y1 - inset - h);
    let corners: Vec<Box2> = [
        (right, top),
        (left, top),
        (left, bottom),
        (right, bottom),
        (right, cy),
        (left, cy),
        (right, cy),
        (cx, bottom),
        (cx, top),
        (cx, cy),
    ]
    .iter()
    .map(|&(x0, y0)| Box2 { x0, y0, x1: x0 + w, y1: y0 + h })
    .collect();
    let b = *corners
        .iter()
        .min_by(|a, c| legend_cost(p, f, a).total_cmp(&legend_cost(p, f, c)))
        .expect("four corners");
    svg.rect(b, "#ffffff", r##" fill-opacity="0.85" stroke="#cccccc" stroke-width="0.8" rx="3""##);
    let mut y = b.y0 + 4.0 + row / 2.0;
    if !title.is_empty() {
        svg.text((b.x0 + b.x1) / 2.0, y, size, Anchor::Middle, "#000000", 0.0, title);
        y += row;
    }
    for (label, color, kind) in entries {
        let (x0, c) = (b.x0 + 7.0, color.hex());
        match kind {
            LegendKind::Line => {
                svg.line(x0, y, x0 + swatch, y, &c, pt(1.3), "");
                svg.circle(x0 + swatch / 2.0, y, pt(1.5), &c, "");
            }
            LegendKind::Dot => svg.circle(x0 + swatch / 2.0, y, pt(2.3), &c, ""),
            LegendKind::Patch => {
                svg.rect(Box2 { x0, y0: y - size * 0.4, x1: x0 + swatch, y1: y + size * 0.4 }, &c, "")
            }
        }
        svg.text(x0 + swatch + 6.0, y, size, Anchor::Start, "#000000", 0.0, label);
        y += row;
    }
    b
}

/// Place each point label at the first corner around its point where it stays inside the plot without
/// overlapping a label already placed (or the legend). Returns how many labels aren't shown.
fn place_labels(svg: &mut Svg, p: &Panel, f: &Frame, legend: Option<Box2>, s: &Sizes) -> usize {
    if p.labels.is_empty() {
        return 0;
    }
    let size = s.point;
    let off = pt(4.0);
    let mut kept: Vec<Box2> = legend.into_iter().collect();
    let legend_boxes = kept.len();
    let area = f.plot;
    for l in &p.labels {
        let (px, py) = (f.sx(l.x), f.sy(l.y));
        let w = text_width(&l.text, size);
        let spots = [
            (off, -off, Anchor::Start),
            (-off, -off, Anchor::End),
            (off, off, Anchor::Start),
            (-off, off, Anchor::End),
        ];
        for (dx, dy, anchor) in spots {
            let x = px + dx;
            let (x0, x1) = match anchor {
                Anchor::Start => (x, x + w),
                _ => (x - w, x),
            };
            // Above the point the text's bottom is at the offset; below it, its top.
            let (y0, y1) = if dy < 0.0 { (py + dy - size, py + dy) } else { (py + dy, py + dy + size) };
            let b = Box2 { x0, y0, x1, y1 }.padded(5.0);
            let inside = area.x0 <= b.x0 + 5.0
                && b.x1 - 5.0 <= area.x1
                && area.y0 <= b.y0 + 5.0
                && b.y1 - 5.0 <= area.y1;
            if inside && !kept.iter().any(|k| b.overlaps(k)) {
                kept.push(b);
                svg.text(x, (y0 + y1) / 2.0, size, anchor, "#000000", 0.0, &l.text);
                break;
            }
        }
    }
    p.labels_total - (kept.len() - legend_boxes)
}

fn draw_pie(svg: &mut Svg, p: &Panel, plot: Box2, s: &Sizes) {
    let Some(Mark::Pie { slices }) = p.marks.iter().find(|m| matches!(m, Mark::Pie { .. })) else { return };
    let (cx, cy) = ((plot.x0 + plot.x1) / 2.0, (plot.y0 + plot.y1) / 2.0);
    let r = 0.4 * plot.w().min(plot.h());
    let at = |deg: f64, rad: f64| (cx + rad * deg.to_radians().cos(), cy - rad * deg.to_radians().sin());
    for sl in slices {
        let c = sl.color.hex();
        let sweep = sl.theta2 - sl.theta1;
        if sweep >= 359.999 {
            svg.circle(cx, cy, r, &c, r##" stroke="#ffffff" stroke-width="1""##);
        } else {
            let (x1, y1) = at(sl.theta1, r);
            let (x2, y2) = at(sl.theta2, r);
            let large = if sweep > 180.0 { 1 } else { 0 };
            let _ = writeln!(
                svg.out,
                r##"<path d="M{cx:.2},{cy:.2} L{x1:.2},{y1:.2} A{r:.2},{r:.2} 0 {large} 0 {x2:.2},{y2:.2} Z" fill="{c}" stroke="#ffffff" stroke-width="1"/>"##
            );
        }
    }
    for sl in slices {
        let mid = (sl.theta1 + sl.theta2) / 2.0;
        let (lx, ly) = at(mid, r * 1.1);
        let anchor = if mid.to_radians().cos() >= 0.0 { Anchor::Start } else { Anchor::End };
        svg.text(lx, ly, s.tick, anchor, "#000000", 0.0, &sl.label);
        let (px, py) = at(mid, r * 0.6);
        svg.text(px, py, s.tick, Anchor::Middle, "#000000", 0.0, &sl.pct);
    }
    if !p.xlabel.is_empty() {
        svg.text(cx, plot.y1 + 8.0 + s.label * 0.6, s.label, Anchor::Middle, "#000000", 0.0, &p.xlabel);
    }
}
