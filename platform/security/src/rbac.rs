//! Role-Based Access Control (RBAC) utilities
//!
//! Enforces role-based authorization for operator actions:
//! - tenant suspend/deprovision
//! - projection rebuild/verify
//! - fleet migration
//!
//! Unauthorized attempts are rejected and audited.

use serde::{Deserialize, Serialize};
use tracing::{warn, info};

/// Roles for operator actions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum Role {
    /// Full access - can perform all operations
    Admin,
    /// Limited operational access - suspend, rebuild, verify, status
    Operator,
    /// Read-only - verify and status only
    Auditor,
}

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
pub struct RbacPolicy;

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

impl Default for RbacPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// RBAC error types
#[derive(Debug, thiserror::Error)]
pub enum RbacError {
    #[error("Insufficient permissions: role={role:?} cannot perform operation={operation:?} on resource={resource} (actor={actor})")]
    InsufficientPermissions {
        role: Role,
        operation: Operation,
        actor: String,
        resource: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_admin_has_all_permissions() {
        assert!(RbacPolicy::has_permission(Role::Admin, Operation::TenantSuspend));
        assert!(RbacPolicy::has_permission(Role::Admin, Operation::TenantDeprovision));
        assert!(RbacPolicy::has_permission(Role::Admin, Operation::ProjectionRebuild));
        assert!(RbacPolicy::has_permission(Role::Admin, Operation::ProjectionVerify));
        assert!(RbacPolicy::has_permission(Role::Admin, Operation::FleetMigrate));
    }

    #[test]
    fn test_operator_limited_permissions() {
        // Operator can suspend
        assert!(RbacPolicy::has_permission(Role::Operator, Operation::TenantSuspend));
        // Operator can rebuild and verify projections
        assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionRebuild));
        assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionVerify));
        assert!(RbacPolicy::has_permission(Role::Operator, Operation::ProjectionStatus));

        // Operator CANNOT deprovision
        assert!(!RbacPolicy::has_permission(Role::Operator, Operation::TenantDeprovision));
        // Operator CANNOT fleet migrate
        assert!(!RbacPolicy::has_permission(Role::Operator, Operation::FleetMigrate));
    }

    #[test]
    fn test_auditor_read_only() {
        // Auditor can verify and check status
        assert!(RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionVerify));
        assert!(RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionStatus));
        assert!(RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionList));

        // Auditor CANNOT perform any write operations
        assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::TenantSuspend));
        assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::TenantDeprovision));
        assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::ProjectionRebuild));
        assert!(!RbacPolicy::has_permission(Role::Auditor, Operation::FleetMigrate));
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

        if let Err(RbacError::InsufficientPermissions { role, operation, .. }) = result {
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
}
