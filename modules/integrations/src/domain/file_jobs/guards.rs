//! File job guards — stateless validation before DB mutations.

use super::models::{
    CreateFileJobRequest, FileJobError, TransitionFileJobRequest, STATUS_COMPLETED, STATUS_CREATED,
    STATUS_FAILED, STATUS_PROCESSING,
};

/// Allowed transitions: created→processing, processing→completed, processing→failed.
const VALID_TRANSITIONS: &[(&str, &str)] = &[
    (STATUS_CREATED, STATUS_PROCESSING),
    (STATUS_PROCESSING, STATUS_COMPLETED),
    (STATUS_PROCESSING, STATUS_FAILED),
];

pub fn validate_create(req: &CreateFileJobRequest) -> Result<(), FileJobError> {
    if req.tenant_id.is_empty() {
        return Err(FileJobError::Validation("tenant_id is required".into()));
    }
    if req.file_ref.trim().is_empty() {
        return Err(FileJobError::Validation("file_ref is required".into()));
    }
    if req.parser_type.trim().is_empty() {
        return Err(FileJobError::Validation("parser_type is required".into()));
    }
    Ok(())
}

pub fn validate_transition(
    current_status: &str,
    req: &TransitionFileJobRequest,
) -> Result<(), FileJobError> {
    if req.tenant_id.is_empty() {
        return Err(FileJobError::Validation("tenant_id is required".into()));
    }
    let allowed = VALID_TRANSITIONS
        .iter()
        .any(|(from, to)| *from == current_status && *to == req.new_status);
    if !allowed {
        return Err(FileJobError::InvalidTransition {
            from: current_status.to_string(),
            to: req.new_status.clone(),
        });
    }
    // error_details only valid when transitioning to failed
    if req.error_details.is_some() && req.new_status != STATUS_FAILED {
        return Err(FileJobError::Validation(
            "error_details only allowed when transitioning to 'failed'".into(),
        ));
    }
    Ok(())
}
