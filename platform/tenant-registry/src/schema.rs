/// Tenant schema definitions
///
/// Core types for tenant registry, status tracking, and schema versioning

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Globally unique tenant identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(pub Uuid);

impl TenantId {
    /// Create a new random tenant ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a tenant ID from an existing UUID
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl Default for TenantId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TenantId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Tenant lifecycle status
///
/// Mirrors the `status` column CHECK constraint in the `tenants` table.
/// New states `Pending` and `Failed` support the control-plane HTTP API
/// state machine (pending → provisioning → active/failed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TenantStatus {
    /// Tenant record created; provisioning not yet started.
    Pending,
    /// Tenant is being provisioned (databases, schemas, initial data).
    Provisioning,
    /// Tenant is fully provisioned and operational.
    Active,
    /// Provisioning failed; tenant is not operational.
    Failed,
    /// Tenant is temporarily suspended (access disabled, data retained).
    Suspended,
    /// Tenant is soft-deleted (marked for cleanup).
    Deleted,
}

impl std::fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Provisioning => write!(f, "provisioning"),
            Self::Active => write!(f, "active"),
            Self::Failed => write!(f, "failed"),
            Self::Suspended => write!(f, "suspended"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

/// Deployment environment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    Development,
    Staging,
    Production,
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Development => write!(f, "development"),
            Self::Staging => write!(f, "staging"),
            Self::Production => write!(f, "production"),
        }
    }
}

/// Per-module schema version tracking
/// Maps module name -> migration version (e.g., "ar" -> "20260216000001")
pub type ModuleSchemaVersions = HashMap<String, String>;

/// Complete tenant registry record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantRecord {
    pub tenant_id: TenantId,
    pub status: TenantStatus,
    pub environment: Environment,
    pub module_schema_versions: ModuleSchemaVersions,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Provisioning step status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProvisioningStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl std::fmt::Display for ProvisioningStepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// Provisioning step verification result (JSON-serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResult {
    pub checks_passed: Vec<String>,
    pub checks_failed: Vec<String>,
    pub details: HashMap<String, serde_json::Value>,
}

/// Individual provisioning step record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningStep {
    pub step_id: Uuid,
    pub tenant_id: TenantId,
    pub step_name: String,
    pub step_order: i32,
    pub status: ProvisioningStepStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub error_message: Option<String>,
    pub verification_result: Option<VerificationResult>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_id_creates_unique_ids() {
        let id1 = TenantId::new();
        let id2 = TenantId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn tenant_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let tenant_id = TenantId::from_uuid(uuid);
        assert_eq!(tenant_id.0, uuid);
    }

    #[test]
    fn tenant_status_serialization() {
        let json = serde_json::to_string(&TenantStatus::Active).unwrap();
        assert_eq!(json, r#""active""#);
    }

    #[test]
    fn environment_serialization() {
        let json = serde_json::to_string(&Environment::Production).unwrap();
        assert_eq!(json, r#""production""#);
    }

    #[test]
    fn module_schema_versions_map() {
        let mut versions = ModuleSchemaVersions::new();
        versions.insert("ar".to_string(), "20260216000001".to_string());
        versions.insert("payments".to_string(), "20260215000002".to_string());
        assert_eq!(versions.len(), 2);
    }

    #[test]
    fn verification_result_structure() {
        let result = VerificationResult {
            checks_passed: vec!["database_exists".to_string()],
            checks_failed: vec![],
            details: HashMap::new(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("checks_passed"));
    }
}
