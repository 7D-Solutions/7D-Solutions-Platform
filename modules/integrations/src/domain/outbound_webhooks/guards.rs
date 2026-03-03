//! Outbound webhook guards — stateless validation before DB mutations.

use super::models::{
    CreateOutboundWebhookRequest, OutboundWebhookError, UpdateOutboundWebhookRequest,
};

const VALID_STATUSES: &[&str] = &["active", "paused", "disabled"];
const MAX_URL_LEN: usize = 2048;
const MAX_EVENT_TYPES: usize = 50;

pub fn validate_create(req: &CreateOutboundWebhookRequest) -> Result<(), OutboundWebhookError> {
    if req.tenant_id.is_empty() {
        return Err(OutboundWebhookError::Validation(
            "tenant_id is required".into(),
        ));
    }
    validate_url(&req.url)?;
    validate_event_types(&req.event_types)?;
    Ok(())
}

pub fn validate_update(req: &UpdateOutboundWebhookRequest) -> Result<(), OutboundWebhookError> {
    if req.tenant_id.is_empty() {
        return Err(OutboundWebhookError::Validation(
            "tenant_id is required".into(),
        ));
    }
    if let Some(ref url) = req.url {
        validate_url(url)?;
    }
    if let Some(ref types) = req.event_types {
        validate_event_types(types)?;
    }
    if let Some(ref status) = req.status {
        if !VALID_STATUSES.contains(&status.as_str()) {
            return Err(OutboundWebhookError::Validation(format!(
                "invalid status '{}', must be one of: {}",
                status,
                VALID_STATUSES.join(", ")
            )));
        }
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<(), OutboundWebhookError> {
    if url.is_empty() {
        return Err(OutboundWebhookError::Validation("url is required".into()));
    }
    if url.len() > MAX_URL_LEN {
        return Err(OutboundWebhookError::Validation(format!(
            "url exceeds maximum length of {}",
            MAX_URL_LEN
        )));
    }
    if !url.starts_with("https://") {
        return Err(OutboundWebhookError::Validation(
            "url must use HTTPS".into(),
        ));
    }
    Ok(())
}

fn validate_event_types(types: &[String]) -> Result<(), OutboundWebhookError> {
    if types.len() > MAX_EVENT_TYPES {
        return Err(OutboundWebhookError::Validation(format!(
            "event_types exceeds maximum of {}",
            MAX_EVENT_TYPES
        )));
    }
    for t in types {
        if t.is_empty() {
            return Err(OutboundWebhookError::Validation(
                "event type must not be empty".into(),
            ));
        }
    }
    Ok(())
}
