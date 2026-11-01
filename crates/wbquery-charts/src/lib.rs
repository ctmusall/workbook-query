//! Charts of query result tables: line, scatter, bar, grouped / stacked bar, histogram, box, pie,
//! heatmap and I-MR control charts.
//!
//! [`build`] turns a table and a [`ChartSpec`] into a [`Chart`] (marks, axes, notes), independent of
//! how it is drawn; [`render`] writes a chart as SVG, PNG or PDF, and the app draws the same chart
//! interactively.

pub mod build;
pub mod render;
pub mod scene;
pub mod spec;
pub mod stats;
pub mod values;

pub use build::build;
pub use scene::Chart;
pub use spec::{ChartSpec, Dataset, KINDS, KindInfo, kind_info};

/// The chosen columns can't make this chart; the message says why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ChartError(pub String);
