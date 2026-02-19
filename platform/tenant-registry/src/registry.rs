/// Tenant registry operations
///
/// Core registry logic for tenant lookup, provisioning status, and metadata queries

use crate::schema::{TenantId, TenantRecord, TenantStatus, ProvisioningStep};
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Registry query result types
pub type RegistryResult<T> = Result<T, RegistryError>;

/// Registry operation errors
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Tenant not found: {0}")]
    TenantNotFound(TenantId),

    #[error("Tenant already exists: {0}")]
    TenantAlreadyExists(TenantId),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Invalid tenant state transition: {from:?} -> {to:?}")]
    InvalidStateTransition {
        from: TenantStatus,
        to: TenantStatus,
    },
}

// ============================================================
// ENTITLEMENTS
// ============================================================

/// Entitlement record for a tenant, read by identity-auth for concurrency enforcement
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct EntitlementRow {
    pub tenant_id: Uuid,
    pub plan_code: String,
    pub concurrent_user_limit: i32,
    pub effective_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Fetch entitlements for a tenant.
///
/// Returns `Ok(Some(row))` if the tenant exists and has an entitlements row.
/// Returns `Ok(None)` if the tenant exists but has no entitlements row.
/// Returns `Err(sqlx::Error::RowNotFound)` if the tenant itself does not exist.
pub async fn get_tenant_entitlements(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Option<EntitlementRow>, sqlx::Error> {
    // Guard: ensure the tenant exists (fail closed — 404 if tenant is unknown)
    let tenant_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM tenants WHERE tenant_id = $1)",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await?;

    if !tenant_exists {
        return Err(sqlx::Error::RowNotFound);
    }

    // Fetch entitlements row (None if absent — identity-auth treats absence as deny)
    let row: Option<EntitlementRow> = sqlx::query_as(
        r#"
        SELECT tenant_id, plan_code, concurrent_user_limit, effective_at, updated_at
        FROM cp_entitlements
        WHERE tenant_id = $1
        "#,
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}

// ============================================================
// APP-ID MAPPING
// ============================================================

/// Response for the app_id mapping endpoint
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TenantAppIdRow {
    pub tenant_id: Uuid,
    pub app_id: String,
    pub product_code: Option<String>,
}

/// Fetch the app_id (and product_code) for a tenant.
///
/// Returns `Ok(Some(row))` if the tenant exists and has a non-NULL app_id.
/// Returns `Ok(None)` if the tenant exists but app_id is NULL.
/// Returns `Err(sqlx::Error::RowNotFound)` if the tenant does not exist.
pub async fn get_tenant_app_id(
    pool: &PgPool,
    tenant_id: Uuid,
) -> Result<Option<TenantAppIdRow>, sqlx::Error> {
    // Guard: ensure the tenant exists first
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT app_id, product_code FROM tenants WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    match row {
        None => Err(sqlx::Error::RowNotFound),
        Some((None, product_code)) => {
            // Tenant exists but app_id is NULL — caller must handle this explicitly
            let _ = product_code;
            Ok(None)
        }
        Some((Some(app_id), product_code)) => Ok(Some(TenantAppIdRow {
            tenant_id,
            app_id,
            product_code,
        })),
    }
}

/// Valid state transitions for tenant lifecycle
pub fn is_valid_state_transition(from: TenantStatus, to: TenantStatus) -> bool {
    use TenantStatus::*;

    match (from, to) {
        // Provisioning can go to active, trial, or deleted
        (Provisioning, Active) => true,
        (Provisioning, Trial) => true,
        (Provisioning, Deleted) => true,

        // Trial tenants can convert to active (paid), go past_due, be suspended or deleted
        (Trial, Active) => true,
        (Trial, PastDue) => true,
        (Trial, Suspended) => true,
        (Trial, Deleted) => true,

        // Active can be suspended, go past_due, or be deleted
        (Active, Suspended) => true,
        (Active, PastDue) => true,
        (Active, Deleted) => true,

        // PastDue can recover to active, be suspended, or deleted
        (PastDue, Active) => true,
        (PastDue, Suspended) => true,
        (PastDue, Deleted) => true,

        // Suspended can be reactivated or deleted
        (Suspended, Active) => true,
        (Suspended, Deleted) => true,

        // Deleted is terminal
        (Deleted, _) => false,

        // No self-transitions
        _ if from == to => false,

        // All other transitions are invalid
        _ => false,
    }
}

/// Tenant registry interface (trait for future DB implementation)
pub trait TenantRegistry {
    /// Look up a tenant by ID
    fn get_tenant(&self, tenant_id: TenantId) -> RegistryResult<TenantRecord>;

    /// Get all provisioning steps for a tenant
    fn get_provisioning_steps(&self, tenant_id: TenantId) -> RegistryResult<Vec<ProvisioningStep>>;

    /// Check if tenant exists
    fn tenant_exists(&self, tenant_id: TenantId) -> RegistryResult<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_to_active_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Provisioning,
            TenantStatus::Active
        ));
    }

    #[test]
    fn provisioning_to_deleted_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Provisioning,
            TenantStatus::Deleted
        ));
    }

    #[test]
    fn active_to_suspended_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Active,
            TenantStatus::Suspended
        ));
    }

    #[test]
    fn active_to_deleted_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Active,
            TenantStatus::Deleted
        ));
    }

    #[test]
    fn suspended_to_active_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Suspended,
            TenantStatus::Active
        ));
    }

    #[test]
    fn deleted_to_anything_is_invalid() {
        assert!(!is_valid_state_transition(
            TenantStatus::Deleted,
            TenantStatus::Active
        ));
        assert!(!is_valid_state_transition(
            TenantStatus::Deleted,
            TenantStatus::Suspended
        ));
    }

    #[test]
    fn self_transitions_are_invalid() {
        assert!(!is_valid_state_transition(
            TenantStatus::Active,
            TenantStatus::Active
        ));
    }

    #[test]
    fn provisioning_to_suspended_is_invalid() {
        assert!(!is_valid_state_transition(
            TenantStatus::Provisioning,
            TenantStatus::Suspended
        ));
    }

    #[test]
    fn provisioning_to_trial_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Provisioning,
            TenantStatus::Trial
        ));
    }

    #[test]
    fn trial_to_active_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Trial,
            TenantStatus::Active
        ));
    }

    #[test]
    fn trial_to_past_due_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Trial,
            TenantStatus::PastDue
        ));
    }

    #[test]
    fn past_due_to_active_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::PastDue,
            TenantStatus::Active
        ));
    }

    #[test]
    fn past_due_to_suspended_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::PastDue,
            TenantStatus::Suspended
        ));
    }

    #[test]
    fn active_to_past_due_is_valid() {
        assert!(is_valid_state_transition(
            TenantStatus::Active,
            TenantStatus::PastDue
        ));
    }
}
