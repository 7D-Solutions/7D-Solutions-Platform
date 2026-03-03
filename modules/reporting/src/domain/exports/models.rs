use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Xlsx,
    Pdf,
}

impl ExportFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Xlsx => "xlsx",
            Self::Pdf => "pdf",
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl ExportStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct ExportRun {
    pub id: Uuid,
    pub tenant_id: String,
    pub report_id: String,
    pub format: String,
    pub status: String,
    pub output_ref: Option<String>,
    pub row_count: Option<i32>,
    pub idempotency_key: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Payload emitted in the outbox event after export completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportCompletedPayload {
    pub export_run_id: Uuid,
    pub report_id: String,
    pub format: String,
    pub row_count: i32,
    pub output_ref: String,
}

/// Row data used to build exports from trial balance cache.
#[derive(Debug, Clone, Serialize)]
pub struct ExportRow {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    pub debit_minor: i64,
    pub credit_minor: i64,
    pub net_minor: i64,
}
