use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

/// Maximum allowed byte size for `internal_value` or `external_value` JSON.
pub const MAX_VALUE_BYTES: usize = 262_144; // 256 KB

#[derive(Debug, Clone, PartialEq)]
pub enum ConflictClass {
    Creation,
    Edit,
    Deletion,
}

impl ConflictClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConflictClass::Creation => "creation",
            ConflictClass::Edit => "edit",
            ConflictClass::Deletion => "deletion",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "creation" => Some(ConflictClass::Creation),
            "edit" => Some(ConflictClass::Edit),
            "deletion" => Some(ConflictClass::Deletion),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConflictStatus {
    Pending,
    Resolved,
    Ignored,
    Unresolvable,
}

impl ConflictStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConflictStatus::Pending => "pending",
            ConflictStatus::Resolved => "resolved",
            ConflictStatus::Ignored => "ignored",
            ConflictStatus::Unresolvable => "unresolvable",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ConflictStatus::Pending),
            "resolved" => Some(ConflictStatus::Resolved),
            "ignored" => Some(ConflictStatus::Ignored),
            "unresolvable" => Some(ConflictStatus::Unresolvable),
            _ => None,
        }
    }

    /// Returns true when no further transitions are permitted.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            ConflictStatus::Resolved | ConflictStatus::Ignored | ConflictStatus::Unresolvable
        )
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct ConflictRow {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    pub conflict_class: String,
    pub status: String,
    pub detected_by: String,
    pub detected_at: DateTime<Utc>,
    pub internal_value: Option<Value>,
    pub external_value: Option<Value>,
    pub internal_id: Option<String>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution_note: Option<String>,
    /// Server-computed deterministic key (sha256 of conflict_id:action:authority_version).
    /// Set only for items resolved via the bulk-resolve endpoint; NULL otherwise.
    #[sqlx(default)]
    pub resolution_idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct CreateConflictRequest {
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    pub conflict_class: ConflictClass,
    pub detected_by: String,
    /// Required for creation/edit classes.
    pub internal_value: Option<Value>,
    /// Required for creation/edit classes.
    pub external_value: Option<Value>,
}

pub struct ResolveConflictRequest {
    pub internal_id: String,
    pub resolved_by: String,
    pub resolution_note: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConflictError {
    #[error("value blob exceeds 256 KB limit")]
    ValueTooLarge,
    #[error("creation/edit conflicts require both internal_value and external_value")]
    MissingValues,
    #[error("conflict not found: {0}")]
    NotFound(Uuid),
    #[error("invalid status transition: {0} → {1}")]
    InvalidTransition(String, String),
    #[error("resolved status requires internal_id")]
    MissingInternalId,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
}
