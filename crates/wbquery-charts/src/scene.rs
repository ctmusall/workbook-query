//! A drawn chart, described independently of how it is rendered: the SVG / PNG / PDF writer and the
//! app's interactive view both draw from this.

/// An RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// Matplotlib's `tab10` palette, which series colors cycle through.
pub const TAB10: [Color; 10] = [
    Color(0x1f, 0x77, 0xb4),
    Color(0xff, 0x7f, 0x0e),
    Color(0x2c, 0xa0, 0x2c),
    Color(0xd6, 0x27, 0x28),
    Color(0x94, 0x67, 0xbd),
    Color(0x8c, 0x56, 0x4b),
    Color(0xe3, 0x77, 0xc2),
    Color(0x7f, 0x7f, 0x7f),
    Color(0xbc, 0xbd, 0x22),
    Color(0x17, 0xbe, 0xcf),
];

pub fn series_color(i: usize) -> Color {
    TAB10[i % TAB10.len()]
}

pub const OUT_COLOR: Color = Color(0xd6, 0x27, 0x28);
pub const LIMIT_COLOR: Color = Color(0xc0, 0x39, 0x2b);
pub const CENTER_COLOR: Color = Color(0x2c, 0x7a, 0x3f);
pub const BLACK: Color = Color(0, 0, 0);
pub const WHITE: Color = Color(255, 255, 255);

/// The viridis color map at `t` in 0..=1.
pub fn viridis(t: f64) -> Color {
    const STOPS: [(u8, u8, u8); 9] = [
        (68, 1, 84),
        (71, 44, 122),
        (59, 81, 139),
        (44, 113, 142),
        (33, 144, 141),
        (39, 173, 129),
        (92, 200, 99),
        (170, 220, 50),
        (253, 231, 37),
    ];
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let pos = t * (STOPS.len() - 1) as f64;
    let i = (pos.floor() as usize).min(STOPS.len() - 2);
    let f = pos - i as f64;
    let mix = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f).round() as u8;
    let (a, b) = (STOPS[i], STOPS[i + 1]);
    Color(mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))
}

/// How an axis reads its values.
#[derive(Debug, Clone, PartialEq)]
pub enum Axis {
    Number,
    /// Matplotlib date numbers (days since 1970-01-01).
    Date,
    /// Labels at positions 0, 1, 2, …
    Category(Vec<String>),
    /// Labels at the given positions (box plots: 1, 2, 3, …).
    Ticks(Vec<(f64, String)>),
}

/// One rectangle of a bar chart or histogram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One box of a box plot at `pos`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxMark {
    pub pos: f64,
    pub width: f64,
    pub stats: crate::stats::BoxStats,
}

/// One pie slice: angles in degrees, counter-clockwise from the positive x axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    pub label: String,
    pub value: f64,
    pub color: Color,
    pub theta1: f64,
    pub theta2: f64,
    pub pct: String,
}

/// One cell's value written on a heatmap.
#[derive(Debug, Clone, PartialEq)]
pub struct CellText {
    pub row: usize,
    pub col: usize,
    pub text: String,
    pub dark: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Mark {
    /// A polyline through the points (NaN y breaks it), with small round markers.
    Line {
        label: Option<String>,
        color: Color,
        points: Vec<[f64; 2]>,
        tooltips: Vec<String>,
    },
    /// Round dots; `size` is the marker area in points².
    Scatter {
        label: Option<String>,
        color: Color,
        points: Vec<[f64; 2]>,
        size: f64,
        alpha: f64,
        tooltips: Vec<String>,
    },
    Bars {
        label: Option<String>,
        color: Color,
        rects: Vec<Rect>,
        alpha: f64,
        edge: Option<Color>,
    },
    /// A horizontal line across the plot with an optional label just right of it.
    HLine {
        y: f64,
        color: Color,
        dashed: bool,
        text: Option<String>,
    },
    Boxes {
        color: Color,
        boxes: Vec<BoxMark>,
    },
    Pie {
        slices: Vec<Slice>,
    },
    /// Rows × columns of values (None: no value) on the viridis scale from `min` to `max`.
    Heatmap {
        values: Vec<Vec<Option<f64>>>,
        min: f64,
        max: f64,
        texts: Vec<CellText>,
    },
}

impl Mark {
    pub fn label(&self) -> Option<&str> {
        match self {
            Mark::Line { label, .. } | Mark::Scatter { label, .. } | Mark::Bars { label, .. } => {
                label.as_deref()
            }
            _ => None,
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Mark::Line { color, .. }
            | Mark::Scatter { color, .. }
            | Mark::Bars { color, .. }
            | Mark::HLine { color, .. }
            | Mark::Boxes { color, .. } => *color,
            _ => BLACK,
        }
    }
}

/// Text that names a point (the words after its number); the renderer drops labels that would overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct PointLabel {
    pub x: f64,
    pub y: f64,
    pub text: String,
}

/// A colorbar beside a heatmap.
#[derive(Debug, Clone, PartialEq)]
pub struct Colorbar {
    pub label: String,
    pub min: f64,
    pub max: f64,
}

/// One plot area with its axes.
#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    pub title: String,
    pub xlabel: String,
    pub ylabel: String,
    pub x: Axis,
    pub y: Axis,
    pub marks: Vec<Mark>,
    /// Show a legend of the labeled marks, with this title.
    pub legend: Option<String>,
    pub grid: bool,
    pub colorbar: Option<Colorbar>,
    /// Point labels, most important first (at most [`crate::build::MAX_POINT_LABELS`]).
    pub labels: Vec<PointLabel>,
    /// How many points had a label before the cap.
    pub labels_total: usize,
    /// Relative height when panels are stacked.
    pub height: f64,
    /// Whole-number ticks only on the x axis.
    pub integer_x: bool,
    /// Category labels on the y axis run top to bottom (heatmap rows).
    pub y_inverted: bool,
    /// Bars and histograms start at zero: no margin below 0.
    pub sticky_zero: bool,
}

impl Panel {
    pub fn new(x: Axis, y: Axis) -> Panel {
        Panel {
            title: String::new(),
            xlabel: String::new(),
            ylabel: String::new(),
            x,
            y,
            marks: Vec::new(),
            legend: None,
            grid: true,
            colorbar: None,
            labels: Vec::new(),
            labels_total: 0,
            height: 1.0,
            integer_x: false,
            y_inverted: false,
            sticky_zero: false,
        }
    }

    /// The marks that appear in the legend, in drawing order.
    pub fn legend_entries(&self) -> Vec<(&str, Color, LegendKind)> {
        self.marks
            .iter()
            .filter_map(|m| {
                let kind = match m {
                    Mark::Line { .. } => LegendKind::Line,
                    Mark::Scatter { .. } => LegendKind::Dot,
                    _ => LegendKind::Patch,
                };
                m.label().map(|l| (l, m.color(), kind))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegendKind {
    Line,
    Dot,
    Patch,
}

/// A whole chart: an optional title over one or more stacked panels, plus notes about skipped data.
#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    pub title: String,
    pub panels: Vec<Panel>,
    pub notes: Vec<String>,
}
