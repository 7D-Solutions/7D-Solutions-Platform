//! Audit event schema definitions
//!
//! Rust types corresponding to the audit_events table

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Classification of mutation type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "mutation_class", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MutationClass {
    Create,
    Update,
    Delete,
    StateTransition,
    Reversal,
}

/// Complete audit event record
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditEvent {
    pub audit_id: Uuid,
    pub occurred_at: DateTime<Utc>,

    // Actor context
    pub actor_id: Uuid,
    pub actor_type: String,

    // Action details
    pub action: String,
    pub mutation_class: MutationClass,

    // Entity identification
    pub entity_type: String,
    pub entity_id: String,

    // State snapshots
    pub before_snapshot: Option<serde_json::Value>,
    pub after_snapshot: Option<serde_json::Value>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,

    // Event correlation
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub trace_id: Option<String>,

    // Metadata
    pub metadata: Option<serde_json::Value>,
}

/// Request to write an audit event (before database insertion)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteAuditRequest {
    // Actor context
    pub actor_id: Uuid,
    pub actor_type: String,

    // Action details
    pub action: String,
    pub mutation_class: MutationClass,

    // Entity identification
    pub entity_type: String,
    pub entity_id: String,

    // State snapshots
    pub before_snapshot: Option<serde_json::Value>,
    pub after_snapshot: Option<serde_json::Value>,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,

    // Event correlation
    pub causation_id: Option<Uuid>,
    pub correlation_id: Option<Uuid>,
    pub trace_id: Option<String>,

    // Metadata
    pub metadata: Option<serde_json::Value>,
}

impl WriteAuditRequest {
    /// Create a minimal audit request
    pub fn new(
        actor_id: Uuid,
        actor_type: String,
        action: String,
        mutation_class: MutationClass,
        entity_type: String,
        entity_id: String,
    ) -> Self {
        Self {
            actor_id,
            actor_type,
            action,
            mutation_class,
            entity_type,
            entity_id,
            before_snapshot: None,
            after_snapshot: None,
            before_hash: None,
            after_hash: None,
            causation_id: None,
            correlation_id: None,
            trace_id: None,
            metadata: None,
        }
    }

    /// Add before/after snapshots
    pub fn with_snapshots(
        mut self,
        before: Option<serde_json::Value>,
        after: Option<serde_json::Value>,
    ) -> Self {
        self.before_snapshot = before;
        self.after_snapshot = after;
        self
    }

    /// Add hash values for integrity verification
    pub fn with_hashes(mut self, before_hash: Option<String>, after_hash: Option<String>) -> Self {
        self.before_hash = before_hash;
        self.after_hash = after_hash;
        self
    }

    /// Add event correlation IDs
    pub fn with_correlation(
        mut self,
        causation_id: Option<Uuid>,
        correlation_id: Option<Uuid>,
        trace_id: Option<String>,
    ) -> Self {
        self.causation_id = causation_id;
        self.correlation_id = correlation_id;
        self.trace_id = trace_id;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }
}
