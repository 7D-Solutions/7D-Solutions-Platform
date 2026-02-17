/// Tenant registry operations
///
/// Core registry logic for tenant lookup, provisioning status, and metadata queries

use crate::schema::{TenantId, TenantRecord, TenantStatus, ProvisioningStep};

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

/// Valid state transitions for tenant lifecycle
pub fn is_valid_state_transition(from: TenantStatus, to: TenantStatus) -> bool {
    use TenantStatus::*;

    match (from, to) {
        // Provisioning can go to active or deleted (if provisioning fails)
        (Provisioning, Active) => true,
        (Provisioning, Deleted) => true,

        // Active can be suspended or deleted
        (Active, Suspended) => true,
        (Active, Deleted) => true,

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
}
