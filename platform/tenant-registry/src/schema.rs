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
/// `Trial` and `PastDue` support billing-aware lifecycle gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Tenant is on a free trial period (access allowed, billing not yet started).
    Trial,
    /// Tenant has an overdue payment; access gated by downstream IAM.
    PastDue,
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
            Self::Trial => write!(f, "trial"),
            Self::PastDue => write!(f, "past_due"),
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
    /// Product identifier (e.g. "starter", "professional", "enterprise")
    pub product_code: Option<String>,
    /// Billing plan within the product (e.g. "monthly", "annual")
    pub plan_code: Option<String>,
    /// Bridge to AR module app namespace; AR is app_id-based
    pub app_id: Option<String>,
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

// ============================================================
// Bundle types (Phase 40 — Tenant Control Plane)
// ============================================================

/// A product bundle definition (row in cp_bundles)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub bundle_id:    Uuid,
    pub product_code: String,
    pub bundle_name:  String,
    pub is_default:   bool,
    pub created_at:   DateTime<Utc>,
    pub updated_at:   DateTime<Utc>,
}

/// A module included in a bundle (row in cp_bundle_modules)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleModule {
    pub bundle_id:      Uuid,
    pub module_code:    String,
    /// Pinned version string; "latest" means always current
    pub module_version: String,
}

/// Status of a tenant's bundle assignment
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantBundleStatus {
    /// Tenant is fully operating on this bundle
    Active,
    /// Tenant is mid-upgrade or mid-downgrade
    InTransition,
}

impl std::fmt::Display for TenantBundleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active       => write!(f, "active"),
            Self::InTransition => write!(f, "in_transition"),
        }
    }
}

/// Tenant-to-bundle assignment (row in cp_tenant_bundle)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantBundle {
    pub tenant_id:    Uuid,
    pub bundle_id:    Uuid,
    pub status:       TenantBundleStatus,
    pub effective_at: DateTime<Utc>,
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
    fn tenant_status_trial_serialization() {
        let json = serde_json::to_string(&TenantStatus::Trial).unwrap();
        assert_eq!(json, r#""trial""#);
    }

    #[test]
    fn tenant_status_past_due_serialization() {
        let json = serde_json::to_string(&TenantStatus::PastDue).unwrap();
        assert_eq!(json, r#""past_due""#);
    }

    #[test]
    fn tenant_status_past_due_display() {
        assert_eq!(TenantStatus::PastDue.to_string(), "past_due");
    }

    #[test]
    fn tenant_record_includes_new_fields() {
        use chrono::Utc;
        use std::collections::HashMap;
        let record = TenantRecord {
            tenant_id: TenantId::new(),
            status: TenantStatus::Trial,
            environment: Environment::Development,
            module_schema_versions: HashMap::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            deleted_at: None,
            product_code: Some("starter".to_string()),
            plan_code: Some("monthly".to_string()),
            app_id: Some("app_abc123".to_string()),
        };
        assert_eq!(record.product_code.as_deref(), Some("starter"));
        assert_eq!(record.plan_code.as_deref(), Some("monthly"));
        assert_eq!(record.app_id.as_deref(), Some("app_abc123"));
    }

    #[test]
    fn environment_serialization() {
        let json = serde_json::to_string(&Environment::Production).unwrap();
        assert_eq!(json, r#""production""#);
    }

    #[test]
    fn bundle_status_serialization() {
        let json = serde_json::to_string(&TenantBundleStatus::Active).unwrap();
        assert_eq!(json, r#""active""#);
        let json = serde_json::to_string(&TenantBundleStatus::InTransition).unwrap();
        assert_eq!(json, r#""in_transition""#);
    }

    #[test]
    fn bundle_status_display() {
        assert_eq!(TenantBundleStatus::Active.to_string(), "active");
        assert_eq!(TenantBundleStatus::InTransition.to_string(), "in_transition");
    }

    #[test]
    fn bundle_struct_roundtrip() {
        use chrono::Utc;
        let bundle = Bundle {
            bundle_id:    Uuid::new_v4(),
            product_code: "starter".to_string(),
            bundle_name:  "Starter Bundle".to_string(),
            is_default:   true,
            created_at:   Utc::now(),
            updated_at:   Utc::now(),
        };
        let json = serde_json::to_string(&bundle).unwrap();
        let decoded: Bundle = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.product_code, "starter");
        assert!(decoded.is_default);
    }

    #[test]
    fn bundle_module_struct() {
        let bm = BundleModule {
            bundle_id:      Uuid::new_v4(),
            module_code:    "ar".to_string(),
            module_version: "latest".to_string(),
        };
        assert_eq!(bm.module_code, "ar");
    }

    #[test]
    fn tenant_bundle_struct() {
        use chrono::Utc;
        let tb = TenantBundle {
            tenant_id:    Uuid::new_v4(),
            bundle_id:    Uuid::new_v4(),
            status:       TenantBundleStatus::Active,
            effective_at: Utc::now(),
        };
        let json = serde_json::to_string(&tb).unwrap();
        assert!(json.contains("active"));
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
