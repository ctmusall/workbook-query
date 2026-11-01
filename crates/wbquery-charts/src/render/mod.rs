//! Drawing charts to files: SVG directly, PNG through resvg, PDF through svg2pdf.

pub mod font;
pub mod svg;
pub mod ticks;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use resvg::{tiny_skia, usvg};

pub use svg::{DPI, Rendered, render_svg};

use crate::Chart;

/// The size charts are saved at, in inches (matplotlib's figure size in the app).
pub const DEFAULT_SIZE: (f64, f64) = (8.0, 5.0);
/// Pixels per inch of saved PNG files.
pub const PNG_DPI: f64 = 150.0;

/// A file format charts can be saved in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Svg,
    Pdf,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
            Format::Pdf => "pdf",
        }
    }

    fn from_path(path: &Path) -> Option<Format> {
        match path.extension()?.to_string_lossy().to_lowercase().as_str() {
            "png" => Some(Format::Png),
            "svg" => Some(Format::Svg),
            "pdf" => Some(Format::Pdf),
            _ => None,
        }
    }
}

/// Saving a chart failed.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SaveError(pub String);

/// A saved chart: where it went, and notes from drawing it (labels left out).
#[derive(Debug, Clone)]
pub struct Saved {
    pub path: PathBuf,
    pub notes: Vec<String>,
}

/// The note for point labels left out so the rest don't overlap.
pub fn hidden_note(hidden: usize) -> Option<String> {
    (hidden > 0).then(|| {
        let s = if hidden == 1 { "" } else { "s" };
        format!("{hidden} point label{s} hidden so labels don't overlap (hover a point to see its text)")
    })
}

fn options() -> usvg::Options<'static> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_font_data(font::FONT.to_vec());
    opt.font_family = font::family();
    opt.dpi = DPI as f32;
    opt
}

/// The SVG parsed with the embedded font.
fn tree(svg: &str) -> Result<usvg::Tree, SaveError> {
    let opt = options();
    let _: &Arc<usvg::fontdb::Database> = &opt.fontdb;
    usvg::Tree::from_str(svg, &opt).map_err(|e| SaveError(format!("Couldn't draw the chart: {e}")))
}

/// The chart as PNG bytes at `dpi`.
pub fn to_png(svg: &str, dpi: f64) -> Result<Vec<u8>, SaveError> {
    let tree = tree(svg)?;
    let scale = (dpi / DPI) as f32;
    let size = tree.size();
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1))
        .ok_or_else(|| SaveError("The chart is too large".into()))?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| SaveError(format!("Couldn't encode the PNG: {e}")))
}

/// The chart as a one-page PDF.
pub fn to_pdf(svg: &str) -> Result<Vec<u8>, SaveError> {
    let tree = tree(svg)?;
    let page = svg2pdf::PageOptions { dpi: DPI as f32 };
    svg2pdf::to_pdf(&tree, svg2pdf::ConversionOptions::default(), page)
        .map_err(|e| SaveError(format!("Couldn't write the PDF: {e}")))
}

/// `path` with a known chart extension; anything else gets `.png` added.
pub fn chart_path(path: impl AsRef<Path>) -> (PathBuf, Format) {
    let path = path.as_ref();
    match Format::from_path(path) {
        Some(f) => (path.to_path_buf(), f),
        None => {
            let mut s = path.as_os_str().to_owned();
            s.push(".png");
            (PathBuf::from(s), Format::Png)
        }
    }
}

/// Save `chart` as PNG, SVG or PDF, chosen by the file extension (none or another one: PNG, with
/// `.png` added). `size` is in inches.
pub fn save(chart: &Chart, path: impl AsRef<Path>, size: (f64, f64)) -> Result<Saved, SaveError> {
    let (path, format) = chart_path(path);
    let rendered = render_svg(chart, size.0 * DPI, size.1 * DPI);
    let bytes = match format {
        Format::Svg => rendered.svg.clone().into_bytes(),
        Format::Png => to_png(&rendered.svg, PNG_DPI)?,
        Format::Pdf => to_pdf(&rendered.svg)?,
    };
    std::fs::write(&path, bytes).map_err(|e| {
        let hint = if e.kind() == std::io::ErrorKind::PermissionDenied {
            " (is it open in another program?)"
        } else {
            ""
        };
        SaveError(format!("{}: {e}{hint}", path.display()))
    })?;
    Ok(Saved { path, notes: hidden_note(rendered.hidden_labels).into_iter().collect() })
}
