//! Platform blob storage — S3-compatible object store with tenant-scoped keys.
//!
//! Implements the contract defined in ADR-018: tenant-scoped keys, MIME
//! validation, size limits, presigned PUT/GET URLs, and structured audit
//! events for every blob operation.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use blob_storage::{BlobStorageClient, BlobStorageConfig, BlobKeyBuilder, BlobAuditEvent, BlobOperation, BlobResult};
//! use blob_storage::validation::{validate_mime_type, validate_size};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), blob_storage::BlobError> {
//! let config = BlobStorageConfig::from_env()?;
//! let max_bytes = config.max_upload_bytes;
//! let client = BlobStorageClient::new(config).await?;
//!
//! // Validate before issuing a presigned URL.
//! validate_mime_type("application/pdf")?;
//! validate_size(1024, max_bytes)?;
//!
//! let key = BlobKeyBuilder {
//!     tenant_id: "t-123",
//!     service: "doc-mgmt",
//!     artifact_type: "upload",
//!     entity_id: "e-456",
//!     object_id: "o-789",
//!     filename: "invoice.pdf",
//! }
//! .build_today();
//!
//! let url = client.presign_put(&key, "application/pdf", None).await?;
//!
//! // Build the audit event — emit it via your service's audit writer.
//! let _audit = BlobAuditEvent::new("t-123", "actor-uuid", BlobOperation::PutPresign)
//!     .with_key(&key)
//!     .with_result(BlobResult::Allowed);
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod key_builder;
pub mod validation;

pub use client::{BlobStorageClient, BlobStorageConfig};
pub use key_builder::{normalize_filename, BlobKeyBuilder};
pub use validation::{validate_mime_type, validate_size, ALLOWED_MIME_TYPES};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Error type ───────────────────────────────────────────────────────────────

/// Errors returned by blob storage operations.
#[derive(Debug, Error)]
pub enum BlobError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("S3 operation failed: {0}")]
    S3(String),

    #[error("MIME type not allowed: {0}")]
    MimeTypeNotAllowed(String),

    #[error("file too large: {size_bytes} bytes exceeds limit of {max_bytes} bytes")]
    FileTooLarge { size_bytes: u64, max_bytes: u64 },
}

// ── Audit event ──────────────────────────────────────────────────────────────

/// Blob operations that must be audited per ADR-018.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobOperation {
    /// Presigned PUT issued (upload initiated).
    PutPresign,
    /// Presigned GET issued (download initiated).
    GetPresign,
    /// Object deleted.
    Delete,
    /// Legal hold applied.
    HoldApply,
    /// Legal hold released.
    HoldRelease,
}

/// Result of a blob operation, as required by ADR-018 audit fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobResult {
    Allowed,
    Denied,
    Failed,
}

/// Structured audit event for a single blob operation.
///
/// Build with [`BlobAuditEvent::new`] and the builder methods, then pass
/// to your service's audit writer (e.g. `audit::writer::AuditWriter`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobAuditEvent {
    pub tenant_id: String,
    pub actor_id: String,
    pub actor_type: String,
    pub service: Option<String>,
    pub bucket: Option<String>,
    pub object_key: Option<String>,
    pub operation: BlobOperation,
    pub result: Option<BlobResult>,
    pub trace_id: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl BlobAuditEvent {
    /// Start building an audit event for the given tenant, actor, and operation.
    pub fn new(tenant_id: &str, actor_id: &str, operation: BlobOperation) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            actor_id: actor_id.to_string(),
            actor_type: "user".to_string(),
            service: None,
            bucket: None,
            object_key: None,
            operation,
            result: None,
            trace_id: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_actor_type(mut self, actor_type: &str) -> Self {
        self.actor_type = actor_type.to_string();
        self
    }

    pub fn with_service(mut self, service: &str) -> Self {
        self.service = Some(service.to_string());
        self
    }

    pub fn with_bucket(mut self, bucket: &str) -> Self {
        self.bucket = Some(bucket.to_string());
        self
    }

    pub fn with_key(mut self, key: &str) -> Self {
        self.object_key = Some(key.to_string());
        self
    }

    pub fn with_result(mut self, result: BlobResult) -> Self {
        self.result = Some(result);
        self
    }

    pub fn with_trace_id(mut self, trace_id: &str) -> Self {
        self.trace_id = Some(trace_id.to_string());
        self
    }
}
