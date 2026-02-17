/// Request and response types for the control-plane API

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
// Error response
// ============================================================================

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}
