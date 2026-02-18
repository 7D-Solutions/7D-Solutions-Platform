//! Bank statement CSV import — types, errors, and public API.
//!
//! Supports deterministic CSV ingestion: raw file bytes are hashed (UUID v5)
//! to produce a stable `statement_hash`. Re-importing the same file returns
//! the existing statement without creating duplicate lines.

pub mod parser;
pub mod service;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("Account not found: {0}")]
    AccountNotFound(Uuid),

    #[error("Account is not active")]
    AccountNotActive,

    #[error("Duplicate import: statement {statement_id} already exists")]
    DuplicateImport { statement_id: Uuid },

    #[error("CSV contains no valid transaction lines")]
    EmptyImport,

    #[error("All CSV lines failed validation")]
    AllLinesFailed(Vec<LineError>),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Response types
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct LineError {
    pub line: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub statement_id: Uuid,
    pub lines_imported: usize,
    pub lines_skipped: usize,
    pub errors: Vec<LineError>,
}
