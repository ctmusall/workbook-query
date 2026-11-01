//! Workbook Query engine: load sheets from Excel workbooks and CSV files, select rows with nested
//! AND / OR conditions, link sheets on a key column, and export the matches to Excel or HTML.

pub mod cell;
pub mod conditions;
pub mod dates;
pub mod error;
pub mod export_html;
pub mod export_xlsx;
pub mod loader;
pub mod proof;
pub mod pyfmt;
pub mod query;
pub mod report;

pub use cell::{Cell, Column, DType, Frame};
pub use conditions::{Condition, Group, Node};
pub use error::{Error, Result};
pub use query::{Query, QueryResult, ResultTable, Source, load_query, run_query, save_query};

/// The program version, shown in reports and `--version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
