//! The font charts are drawn with (Ubuntu Light, the app's UI font) and text measurement for layout.

use std::sync::LazyLock;

/// The font file embedded in the program, so charts look the same everywhere.
pub const FONT: &[u8] = epaint_default_fonts::UBUNTU_LIGHT;

struct Metrics {
    units_per_em: f64,
    advances: std::collections::HashMap<char, f64>,
    fallback: f64,
}

static METRICS: LazyLock<Metrics> = LazyLock::new(|| {
    let face = ttf_parser::Face::parse(FONT, 0).expect("embedded font parses");
    let units_per_em = face.units_per_em() as f64;
    let advance = |c: char| face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)).map(f64::from);
    let fallback = advance('n').unwrap_or(units_per_em * 0.5);
    let advances =
        (' '..='~').chain("−—·…X̄MR̄°µ²³".chars()).filter_map(|c| advance(c).map(|a| (c, a))).collect();
    Metrics { units_per_em, advances, fallback }
});

/// The font's family name, as SVG renderers look it up.
pub fn family() -> String {
    let face = ttf_parser::Face::parse(FONT, 0).expect("embedded font parses");
    face.names()
        .into_iter()
        .find(|n| n.name_id == ttf_parser::name_id::TYPOGRAPHIC_FAMILY && n.is_unicode())
        .or_else(|| {
            face.names().into_iter().find(|n| n.name_id == ttf_parser::name_id::FAMILY && n.is_unicode())
        })
        .and_then(|n| n.to_string())
        .unwrap_or_else(|| "Ubuntu".into())
}

/// The width of `text` at `size` pixels.
pub fn text_width(text: &str, size: f64) -> f64 {
    let m = &*METRICS;
    let units: f64 = text
        .chars()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c)) // combining marks take no space
        .map(|c| m.advances.get(&c).copied().unwrap_or(m.fallback))
        .sum();
    units * size / m.units_per_em
}
