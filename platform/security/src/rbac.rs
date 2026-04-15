//! Role-Based Access Control (RBAC) utilities
//!
//! ## Migration: Role enum → JWT permissions
//!
//! The legacy [`Role`] / [`RbacPolicy`] system is deprecated.  CLI tools now
//! authenticate via JWT (same as HTTP endpoints) and check permission strings
//! from [`VerifiedClaims::perms`](crate::claims::VerifiedClaims).
//!
//! Use [`check_permissions`] for CLI authorization and the `PERM_*` constants
//! below for the required permission strings.

use crate::claims::VerifiedClaims;
use crate::SecurityError;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

// ============================================================================
// Permission string constants for CLI / operator actions
// ============================================================================

pub const PERM_TENANT_SUSPEND: &str = "tenant.suspend";
pub const PERM_TENANT_DEPROVISION: &str = "tenant.deprovision";
pub const PERM_PROJECTION_REBUILD: &str = "projection.rebuild";
pub const PERM_PROJECTION_VERIFY: &str = "projection.verify";
pub const PERM_PROJECTION_STATUS: &str = "projection.status";
pub const PERM_PROJECTION_LIST: &str = "projection.list";
pub const PERM_FLEET_MIGRATE: &str = "fleet.migrate";

// ============================================================================
// JWT-based permission check (replaces RbacPolicy)
// ============================================================================

/// Check that `claims` contain every permission in `required`.
///
/// Returns `Ok(())` when all required permissions are present, or
/// `Err(SecurityError::InsufficientPermissions)` with a warning log
/// listing the missing entries.
pub fn check_permissions(claims: &VerifiedClaims, required: &[&str]) -> Result<(), SecurityError> {
    let missing: Vec<&str> = required
        .iter()
        .filter(|p| !claims.perms.iter().any(|cp| cp == *p))
        .copied()
        .collect();

    if missing.is_empty() {
        info!(
            user_id = %claims.user_id,
            tenant_id = %claims.tenant_id,
            "CLI authorization granted"
        );
        Ok(())
    } else {
        warn!(
            user_id = %claims.user_id,
            tenant_id = %claims.tenant_id,
            missing = ?missing,
            "CLI authorization denied — insufficient permissions"
        );
        Err(SecurityError::InsufficientPermissions)
    }
}

// ============================================================================
// Deprecated types — kept for backward compatibility
// ============================================================================

/// Roles for operator actions
#[deprecated(
    since = "0.2.0",
    note = "Use JWT permission strings (PERM_* constants) with check_permissions() instead"
)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Role {
    /// Full access - can perform all operations
    Admin,
    /// Limited operational access - suspend, rebuild, verify, status
    Operator,
    /// Read-only - verify and status only
    Auditor,
}

#[allow(deprecated)]
impl Role {
    /// Parse role from string (case-insensitive)
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "admin" => Some(Role::Admin),
            "operator" => Some(Role::Operator),
            "auditor" => Some(Role::Auditor),
            _ => None,
        }
    }

    /// Convert role to string
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Operator => "operator",
            Role::Auditor => "auditor",
        }
    }
}

/// Operations that require authorization
#[deprecated(
    since = "0.2.0",
    note = "Use JWT permission strings (PERM_* constants) with check_permissions() instead"
)]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Suspend tenant (disable access, retain data)
    TenantSuspend,
    /// Deprovision tenant (soft delete, mark for cleanup)
    TenantDeprovision,
    /// Rebuild projection from events
    ProjectionRebuild,
    /// Verify projection integrity
    ProjectionVerify,
    /// Check projection status
    ProjectionStatus,
    /// List available projections
    ProjectionList,
    /// Run fleet migration (N tenants)
    FleetMigrate,
}

#[allow(deprecated)]
impl Operation {
    /// Convert operation to string
    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::TenantSuspend => "tenant.suspend",
            Operation::TenantDeprovision => "tenant.deprovision",
            Operation::ProjectionRebuild => "projection.rebuild",
            Operation::ProjectionVerify => "projection.verify",
            Operation::ProjectionStatus => "projection.status",
            Operation::ProjectionList => "projection.list",
            Operation::FleetMigrate => "fleet.migrate",
        }
    }
}

/// RBAC policy enforcement
#[deprecated(
    since = "0.2.0",
    note = "Use check_permissions() with VerifiedClaims instead"
)]
pub struct RbacPolicy;

#[allow(deprecated)]
impl RbacPolicy {
    pub fn new() -> Self {
        Self
    }

    /// Check if a role has permission to perform an operation
    ///
    /// Permission matrix:
    /// - Admin: All operations
    /// - Operator: TenantSuspend, Projection* (all), but NOT TenantDeprovision or FleetMigrate
    /// - Auditor: ProjectionVerify, ProjectionStatus, ProjectionList only (read-only)
    pub fn has_permission(role: Role, operation: Operation) -> bool {
        match role {
            Role::Admin => {
                // Admin has full access
                true
            }
            Role::Operator => {
                // Operator can suspend, but not deprovision or fleet migrate
                matches!(
                    operation,
                    Operation::TenantSuspend
                        | Operation::ProjectionRebuild
                        | Operation::ProjectionVerify
                        | Operation::ProjectionStatus
                        | Operation::ProjectionList
                )
            }
            Role::Auditor => {
                // Auditor is read-only - verify and status only
                matches!(
                    operation,
                    Operation::ProjectionVerify
                        | Operation::ProjectionStatus
                        | Operation::ProjectionList
                )
            }
        }
    }

    /// Authorize an operation for a role
    ///
    /// Returns Ok(()) if authorized, Err with details if denied.
    /// Logs authorization attempts for audit trail.
    pub fn authorize(
        role: Role,
        operation: Operation,
        actor: &str,
        resource: &str,
    ) -> Result<(), RbacError> {
        let allowed = Self::has_permission(role, operation);

        if allowed {
            info!(
                role = role.as_str(),
                operation = operation.as_str(),
                actor = actor,
                resource = resource,
                "Authorization granted"
            );
            Ok(())
        } else {
            warn!(
                role = role.as_str(),
                operation = operation.as_str(),
                actor = actor,
                resource = resource,
                "Authorization denied - insufficient permissions"
            );
            Err(RbacError::InsufficientPermissions {
                role,
                operation,
                actor: actor.to_string(),
                resource: resource.to_string(),
            })
        }
    }
}

#[allow(deprecated)]
impl Default for RbacPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// RBAC error types
#[deprecated(
    since = "0.2.0",
    note = "Use SecurityError::InsufficientPermissions instead"
)]
#[derive(Debug, thiserror::Error)]
pub enum RbacError {
    #[allow(deprecated)]
    #[error("Insufficient permissions: role={role:?} cannot perform operation={operation:?} on resource={resource} (actor={actor})")]
    InsufficientPermissions {
        role: Role,
        operation: Operation,
        actor: String,
        resource: String,
    },
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_has_all_permissions() {
        assert!(RbacPolicy::has_permission(
            Role::Admin,
            Operation::TenantSuspend
        ));
        assert!(RbacPolicy::has_permission(
            Role::Admin,
            Operation::TenantDeprovision
        ));
        assert!(RbacPolicy::has_permission(
            Role::Admin,
            Operation::ProjectionRebuild
        ));
        assert!(RbacPolicy::has_permission(
            Role::Admin,
            Operation::ProjectionVerify
        ));
        assert!(RbacPolicy::has_permission(
            Role::Admin,
            Operation::FleetMigrate
        ));
    }

    #[test]
    fn test_operator_limited_permissions() {
        // Operator can suspend
        assert!(RbacPolicy::has_permission(
            Role::Operator,
            Operation::TenantSuspend
        ));
        // Operator can rebuild and verify projections
        assert!(RbacPolicy::has_permission(
            Role::Operator,
            Operation::ProjectionRebuild
        ));
        assert!(RbacPolicy::has_permission(
            Role::Operator,
            Operation::ProjectionVerify
        ));
        assert!(RbacPolicy::has_permission(
            Role::Operator,
            Operation::ProjectionStatus
        ));

        // Operator CANNOT deprovision
        assert!(!RbacPolicy::has_permission(
            Role::Operator,
            Operation::TenantDeprovision
        ));
        // Operator CANNOT fleet migrate
        assert!(!RbacPolicy::has_permission(
            Role::Operator,
            Operation::FleetMigrate
        ));
    }

    #[test]
    fn test_auditor_read_only() {
        // Auditor can verify and check status
        assert!(RbacPolicy::has_permission(
            Role::Auditor,
            Operation::ProjectionVerify
        ));
        assert!(RbacPolicy::has_permission(
            Role::Auditor,
            Operation::ProjectionStatus
        ));
        assert!(RbacPolicy::has_permission(
            Role::Auditor,
            Operation::ProjectionList
        ));

        // Auditor CANNOT perform any write operations
        assert!(!RbacPolicy::has_permission(
            Role::Auditor,
            Operation::TenantSuspend
        ));
        assert!(!RbacPolicy::has_permission(
            Role::Auditor,
            Operation::TenantDeprovision
        ));
        assert!(!RbacPolicy::has_permission(
            Role::Auditor,
            Operation::ProjectionRebuild
        ));
        assert!(!RbacPolicy::has_permission(
            Role::Auditor,
            Operation::FleetMigrate
        ));
    }

    #[test]
    fn test_authorize_success() {
        let result = RbacPolicy::authorize(
            Role::Admin,
            Operation::TenantDeprovision,
            "admin-user",
            "tenant-123",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_authorize_denial() {
        let result = RbacPolicy::authorize(
            Role::Auditor,
            Operation::TenantSuspend,
            "auditor-user",
            "tenant-123",
        );
        assert!(result.is_err());

        if let Err(RbacError::InsufficientPermissions {
            role, operation, ..
        }) = result
        {
            assert_eq!(role, Role::Auditor);
            assert_eq!(operation, Operation::TenantSuspend);
        } else {
            panic!("Expected InsufficientPermissions error");
        }
    }

    #[test]
    fn test_role_parsing() {
        assert_eq!(Role::from_str("admin"), Some(Role::Admin));
        assert_eq!(Role::from_str("ADMIN"), Some(Role::Admin));
        assert_eq!(Role::from_str("operator"), Some(Role::Operator));
        assert_eq!(Role::from_str("auditor"), Some(Role::Auditor));
        assert_eq!(Role::from_str("invalid"), None);
    }

    // ── New permission-based tests ──────────────────────────────────

    #[test]
    fn check_permissions_grants_with_all_required() {
        let claims = test_claims(vec![
            PERM_TENANT_SUSPEND.to_string(),
            PERM_FLEET_MIGRATE.to_string(),
        ]);
        assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND]).is_ok());
        assert!(check_permissions(&claims, &[PERM_FLEET_MIGRATE]).is_ok());
        assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND, PERM_FLEET_MIGRATE]).is_ok());
    }

    #[test]
    fn check_permissions_denies_with_missing() {
        let claims = test_claims(vec![PERM_TENANT_SUSPEND.to_string()]);
        assert!(check_permissions(&claims, &[PERM_FLEET_MIGRATE]).is_err());
        assert!(check_permissions(&claims, &[PERM_TENANT_SUSPEND, PERM_FLEET_MIGRATE]).is_err());
    }

    #[test]
    fn check_permissions_grants_empty_required() {
        let claims = test_claims(vec![]);
        assert!(check_permissions(&claims, &[]).is_ok());
    }

    #[test]
    fn perm_constants_match_operation_strings() {
        assert_eq!(PERM_TENANT_SUSPEND, Operation::TenantSuspend.as_str());
        assert_eq!(
            PERM_TENANT_DEPROVISION,
            Operation::TenantDeprovision.as_str()
        );
        assert_eq!(
            PERM_PROJECTION_REBUILD,
            Operation::ProjectionRebuild.as_str()
        );
        assert_eq!(PERM_PROJECTION_VERIFY, Operation::ProjectionVerify.as_str());
        assert_eq!(PERM_PROJECTION_STATUS, Operation::ProjectionStatus.as_str());
        assert_eq!(PERM_PROJECTION_LIST, Operation::ProjectionList.as_str());
        assert_eq!(PERM_FLEET_MIGRATE, Operation::FleetMigrate.as_str());
    }

    /// Build a minimal VerifiedClaims for unit tests (no real JWT involved).
    fn test_claims(perms: Vec<String>) -> VerifiedClaims {
        use chrono::Utc;
        use uuid::Uuid;
        VerifiedClaims {
            user_id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            app_id: None,
            roles: vec!["admin".into()],
            perms,
            actor_type: crate::claims::ActorType::User,
            issued_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::minutes(15),
            token_id: Uuid::new_v4(),
            version: "1".to_string(),
        }
    }
}
