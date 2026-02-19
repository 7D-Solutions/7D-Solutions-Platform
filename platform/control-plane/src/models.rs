/// Request and response types for the control-plane API

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ============================================================================
// POST /api/control/tenants
// ============================================================================

/// Request body for tenant provisioning
#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    /// Optional tenant ID (UUID). If omitted, one is generated.
    pub tenant_id: Option<Uuid>,

    /// Caller-supplied idempotency key. Required. Must be unique per provisioning attempt.
    /// Duplicate keys return the existing result (idempotent).
    pub idempotency_key: String,

    /// Deployment environment
    pub environment: Environment,
}

/// Accepted environments for provisioning
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl Environment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Staging => "staging",
            Self::Production => "production",
        }
    }
}

/// Response body for a provisioning request (202 Accepted or 200 OK on idempotency replay)
#[derive(Debug, Serialize)]
pub struct CreateTenantResponse {
    /// Assigned or provided tenant ID
    pub tenant_id: Uuid,

    /// Current provisioning status
    pub status: String,

    /// Echoed idempotency key
    pub idempotency_key: String,
}

// ============================================================================
// Retention policy
// ============================================================================

/// Retention configuration for a tenant (stored in cp_retention_policies)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RetentionConfig {
    pub tenant_id: Uuid,
    /// Days data must be retained after deletion (default 2555 ≈ 7 years)
    pub data_retention_days: i32,
    /// Export artifact format (currently only "jsonl")
    pub export_format: String,
    /// Grace window between export_ready_at and permitted tombstone (days)
    pub auto_tombstone_days: i32,
    /// When a deterministic export artifact was last produced; null if never
    pub export_ready_at: Option<DateTime<Utc>>,
    /// When tenant data was tombstoned; null if not yet tombstoned
    pub data_tombstoned_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// PUT body to upsert retention config for a tenant
#[derive(Debug, Deserialize)]
pub struct SetRetentionRequest {
    pub data_retention_days: Option<i32>,
    pub auto_tombstone_days: Option<i32>,
}

/// Response for the tombstone operation
#[derive(Debug, Serialize)]
pub struct TombstoneResponse {
    pub tenant_id: Uuid,
    pub data_tombstoned_at: DateTime<Utc>,
    pub audit_note: String,
}

// ============================================================================
// Error response
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}
