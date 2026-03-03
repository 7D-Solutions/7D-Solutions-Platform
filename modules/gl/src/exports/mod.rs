//! GL Export module
//!
//! Provides export adapters for QuickBooks (IIF) and Xero (CSV) formats,
//! covering chart of accounts and journal entries.

pub mod formats;
pub mod service;

pub use service::{ExportError, ExportRequest, ExportResult};
