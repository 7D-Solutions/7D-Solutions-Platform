//! Audit policy enforcement
//!
//! Enforces mutation policies based on entity classification:
//! - StrictImmutable: No updates/deletes allowed
//! - CompensatingRequired: Must emit reversal/supersession linkage
//! - MutableWithAudit: Must emit field-level diff audit

use serde::{Deserialize, Serialize};
use chrono::Duration;
use thiserror::Error;

// ============================================================================
// Retention Policy (existing)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub retention_days: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            retention_days: 2555, // 7 years default
        }
    }
}

impl RetentionPolicy {
    pub fn duration(&self) -> Duration {
        Duration::days(self.retention_days)
    }
}

// ============================================================================
// Audit Policy Classification
// ============================================================================

/// Classification of entity mutation behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditPolicy {
    /// No updates or deletes allowed - entity is immutable after creation
    StrictImmutable,

    /// Updates allowed only via compensating transactions (reversals/supersessions)
    CompensatingRequired,

    /// Updates allowed but must emit field-level diff audit
    MutableWithAudit,

    /// Updates allowed with standard audit (no special requirements)
    MutableStandard,
}

impl AuditPolicy {
    /// Check if this policy allows direct updates
    pub fn allows_direct_updates(&self) -> bool {
        matches!(self, Self::MutableWithAudit | Self::MutableStandard)
    }

    /// Check if this policy requires compensating transactions
    pub fn requires_compensation(&self) -> bool {
        matches!(self, Self::CompensatingRequired)
    }

    /// Check if this policy requires field-level diffs
    pub fn requires_field_diff(&self) -> bool {
        matches!(self, Self::MutableWithAudit)
    }
}

// ============================================================================
// Policy Violation Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum PolicyViolation {
    #[error("Strict immutable entity cannot be updated: {entity_type}:{entity_id}")]
    StrictImmutableViolation {
        entity_type: String,
        entity_id: String,
    },

    #[error("Compensating transaction required for {entity_type}:{entity_id}, but no reversal/supersession linkage provided")]
    MissingCompensationLinkage {
        entity_type: String,
        entity_id: String,
    },

    #[error("Field-level diff required for {entity_type}:{entity_id}, but snapshots missing")]
    MissingFieldDiff {
        entity_type: String,
        entity_id: String,
    },

    #[error("Delete operation not allowed for policy: {policy:?}")]
    DeleteNotAllowed {
        policy: AuditPolicy,
    },
}

// ============================================================================
// Mutation Request (for policy validation)
// ============================================================================

/// Request to perform a mutation (for policy validation)
#[derive(Debug, Clone)]
pub struct MutationRequest {
    pub entity_type: String,
    pub entity_id: String,
    pub operation: MutationOperation,
    pub policy: AuditPolicy,

    // Metadata for policy enforcement
    pub has_reversal_linkage: bool,
    pub has_field_diff: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOperation {
    Create,
    Update,
    Delete,
}

// ============================================================================
// Policy Enforcement
// ============================================================================

/// Validate a mutation request against its audit policy
///
/// Returns Ok(()) if the mutation is allowed, or PolicyViolation if rejected.
pub fn validate_mutation(request: &MutationRequest) -> Result<(), PolicyViolation> {
    match request.operation {
        MutationOperation::Create => {
            // Creates always allowed
            Ok(())
        }

        MutationOperation::Update => {
            match request.policy {
                AuditPolicy::StrictImmutable => {
                    // StrictImmutable: No updates allowed
                    Err(PolicyViolation::StrictImmutableViolation {
                        entity_type: request.entity_type.clone(),
                        entity_id: request.entity_id.clone(),
                    })
                }

                AuditPolicy::CompensatingRequired => {
                    // CompensatingRequired: Must have reversal/supersession linkage
                    if !request.has_reversal_linkage {
                        Err(PolicyViolation::MissingCompensationLinkage {
                            entity_type: request.entity_type.clone(),
                            entity_id: request.entity_id.clone(),
                        })
                    } else {
                        Ok(())
                    }
                }

                AuditPolicy::MutableWithAudit => {
                    // MutableWithAudit: Must have field-level diff
                    if !request.has_field_diff {
                        Err(PolicyViolation::MissingFieldDiff {
                            entity_type: request.entity_type.clone(),
                            entity_id: request.entity_id.clone(),
                        })
                    } else {
                        Ok(())
                    }
                }

                AuditPolicy::MutableStandard => {
                    // MutableStandard: No special requirements
                    Ok(())
                }
            }
        }

        MutationOperation::Delete => {
            match request.policy {
                AuditPolicy::StrictImmutable | AuditPolicy::CompensatingRequired => {
                    // StrictImmutable and CompensatingRequired: No deletes allowed
                    Err(PolicyViolation::DeleteNotAllowed {
                        policy: request.policy,
                    })
                }

                AuditPolicy::MutableWithAudit | AuditPolicy::MutableStandard => {
                    // MutableWithAudit and MutableStandard: Deletes allowed (with audit)
                    Ok(())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strict_immutable_blocks_updates() {
        let request = MutationRequest {
            entity_type: "Invoice".to_string(),
            entity_id: "inv_123".to_string(),
            operation: MutationOperation::Update,
            policy: AuditPolicy::StrictImmutable,
            has_reversal_linkage: false,
            has_field_diff: false,
        };

        let result = validate_mutation(&request);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PolicyViolation::StrictImmutableViolation { .. }
        ));
    }

    #[test]
    fn test_compensating_requires_linkage() {
        let mut request = MutationRequest {
            entity_type: "Payment".to_string(),
            entity_id: "pay_456".to_string(),
            operation: MutationOperation::Update,
            policy: AuditPolicy::CompensatingRequired,
            has_reversal_linkage: false,
            has_field_diff: false,
        };

        // Without linkage: rejected
        let result = validate_mutation(&request);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PolicyViolation::MissingCompensationLinkage { .. }
        ));

        // With linkage: allowed
        request.has_reversal_linkage = true;
        let result = validate_mutation(&request);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mutable_with_audit_requires_diff() {
        let mut request = MutationRequest {
            entity_type: "Customer".to_string(),
            entity_id: "cust_789".to_string(),
            operation: MutationOperation::Update,
            policy: AuditPolicy::MutableWithAudit,
            has_reversal_linkage: false,
            has_field_diff: false,
        };

        // Without diff: rejected
        let result = validate_mutation(&request);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PolicyViolation::MissingFieldDiff { .. }
        ));

        // With diff: allowed
        request.has_field_diff = true;
        let result = validate_mutation(&request);
        assert!(result.is_ok());
    }

    #[test]
    fn test_mutable_standard_allows_updates() {
        let request = MutationRequest {
            entity_type: "TempData".to_string(),
            entity_id: "tmp_000".to_string(),
            operation: MutationOperation::Update,
            policy: AuditPolicy::MutableStandard,
            has_reversal_linkage: false,
            has_field_diff: false,
        };

        let result = validate_mutation(&request);
        assert!(result.is_ok());
    }

    #[test]
    fn test_creates_always_allowed() {
        for policy in [
            AuditPolicy::StrictImmutable,
            AuditPolicy::CompensatingRequired,
            AuditPolicy::MutableWithAudit,
            AuditPolicy::MutableStandard,
        ] {
            let request = MutationRequest {
                entity_type: "Test".to_string(),
                entity_id: "test_1".to_string(),
                operation: MutationOperation::Create,
                policy,
                has_reversal_linkage: false,
                has_field_diff: false,
            };

            assert!(validate_mutation(&request).is_ok());
        }
    }

    #[test]
    fn test_deletes_blocked_for_strict_immutable() {
        let request = MutationRequest {
            entity_type: "Invoice".to_string(),
            entity_id: "inv_123".to_string(),
            operation: MutationOperation::Delete,
            policy: AuditPolicy::StrictImmutable,
            has_reversal_linkage: false,
            has_field_diff: false,
        };

        let result = validate_mutation(&request);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PolicyViolation::DeleteNotAllowed { .. }
        ));
    }
}
