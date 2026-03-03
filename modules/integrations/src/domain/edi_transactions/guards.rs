//! EDI transaction guards — stateless validation before DB mutations.

use super::models::{
    CreateOutboundEdiRequest, EdiTransactionError, IngestEdiRequest, TransitionEdiRequest,
    DIRECTION_INBOUND, DIRECTION_OUTBOUND, STATUS_ACCEPTED, STATUS_CREATED, STATUS_EMITTED,
    STATUS_INGESTED, STATUS_PARSED, STATUS_REJECTED, STATUS_VALIDATED,
};

/// Allowed inbound transitions:
///   ingested → parsed | rejected
///   parsed   → validated | rejected
///   validated → accepted | rejected
const INBOUND_TRANSITIONS: &[(&str, &str)] = &[
    (STATUS_INGESTED, STATUS_PARSED),
    (STATUS_INGESTED, STATUS_REJECTED),
    (STATUS_PARSED, STATUS_VALIDATED),
    (STATUS_PARSED, STATUS_REJECTED),
    (STATUS_VALIDATED, STATUS_ACCEPTED),
    (STATUS_VALIDATED, STATUS_REJECTED),
];

/// Allowed outbound transitions:
///   created   → validated | rejected
///   validated → emitted | rejected
const OUTBOUND_TRANSITIONS: &[(&str, &str)] = &[
    (STATUS_CREATED, STATUS_VALIDATED),
    (STATUS_CREATED, STATUS_REJECTED),
    (STATUS_VALIDATED, STATUS_EMITTED),
    (STATUS_VALIDATED, STATUS_REJECTED),
];

pub fn validate_ingest(req: &IngestEdiRequest) -> Result<(), EdiTransactionError> {
    if req.tenant_id.is_empty() {
        return Err(EdiTransactionError::Validation(
            "tenant_id is required".into(),
        ));
    }
    if req.transaction_type.trim().is_empty() {
        return Err(EdiTransactionError::Validation(
            "transaction_type is required".into(),
        ));
    }
    if req.version.trim().is_empty() {
        return Err(EdiTransactionError::Validation(
            "version is required".into(),
        ));
    }
    if req.raw_payload.trim().is_empty() {
        return Err(EdiTransactionError::Validation(
            "raw_payload is required".into(),
        ));
    }
    Ok(())
}

pub fn validate_create_outbound(req: &CreateOutboundEdiRequest) -> Result<(), EdiTransactionError> {
    if req.tenant_id.is_empty() {
        return Err(EdiTransactionError::Validation(
            "tenant_id is required".into(),
        ));
    }
    if req.transaction_type.trim().is_empty() {
        return Err(EdiTransactionError::Validation(
            "transaction_type is required".into(),
        ));
    }
    if req.version.trim().is_empty() {
        return Err(EdiTransactionError::Validation(
            "version is required".into(),
        ));
    }
    Ok(())
}

pub fn validate_transition(
    current_status: &str,
    direction: &str,
    req: &TransitionEdiRequest,
) -> Result<(), EdiTransactionError> {
    if req.tenant_id.is_empty() {
        return Err(EdiTransactionError::Validation(
            "tenant_id is required".into(),
        ));
    }

    let transitions = match direction {
        DIRECTION_INBOUND => INBOUND_TRANSITIONS,
        DIRECTION_OUTBOUND => OUTBOUND_TRANSITIONS,
        _ => {
            return Err(EdiTransactionError::Validation(format!(
                "unknown direction '{direction}'"
            )));
        }
    };

    let allowed = transitions
        .iter()
        .any(|(from, to)| *from == current_status && *to == req.new_status);

    if !allowed {
        return Err(EdiTransactionError::InvalidTransition {
            from: current_status.to_string(),
            to: req.new_status.clone(),
        });
    }

    // error_details only valid when transitioning to rejected
    if req.error_details.is_some() && req.new_status != STATUS_REJECTED {
        return Err(EdiTransactionError::Validation(
            "error_details only allowed when transitioning to 'rejected'".into(),
        ));
    }

    Ok(())
}
