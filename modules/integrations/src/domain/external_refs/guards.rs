//! Guard functions for external_refs domain.
//!
//! Guards enforce preconditions before mutations occur.
//! They run BEFORE the database transaction opens.

use super::models::{CreateExternalRefRequest, ExternalRefError, UpdateExternalRefRequest};

/// Validate a create request: required fields must be non-empty.
pub fn validate_create(req: &CreateExternalRefRequest) -> Result<(), ExternalRefError> {
    if req.entity_type.trim().is_empty() {
        return Err(ExternalRefError::Validation(
            "entity_type is required".to_string(),
        ));
    }
    if req.entity_id.trim().is_empty() {
        return Err(ExternalRefError::Validation(
            "entity_id is required".to_string(),
        ));
    }
    if req.system.trim().is_empty() {
        return Err(ExternalRefError::Validation("system is required".to_string()));
    }
    if req.external_id.trim().is_empty() {
        return Err(ExternalRefError::Validation(
            "external_id is required".to_string(),
        ));
    }
    Ok(())
}

/// Validate an update request: at least one field must be provided.
pub fn validate_update(req: &UpdateExternalRefRequest) -> Result<(), ExternalRefError> {
    if req.label.is_none() && req.metadata.is_none() {
        return Err(ExternalRefError::Validation(
            "At least one of label or metadata must be provided".to_string(),
        ));
    }
    Ok(())
}
