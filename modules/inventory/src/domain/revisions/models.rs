//! Revision domain model, request types, and error definitions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ItemRevision {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_number: i32,
    pub name: String,
    pub description: Option<String>,
    pub uom: String,
    pub inventory_account_ref: String,
    pub cogs_account_ref: String,
    pub variance_account_ref: String,
    pub traceability_level: String,
    pub inspection_required: bool,
    pub shelf_life_days: Option<i32>,
    pub shelf_life_enforced: bool,
    pub effective_from: Option<DateTime<Utc>>,
    pub effective_to: Option<DateTime<Utc>>,
    pub change_reason: String,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub activated_at: Option<DateTime<Utc>>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRevisionRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub uom: String,
    pub inventory_account_ref: String,
    pub cogs_account_ref: String,
    pub variance_account_ref: String,
    #[serde(default = "default_traceability_level")]
    pub traceability_level: String,
    #[serde(default)]
    pub inspection_required: bool,
    #[serde(default)]
    pub shelf_life_days: Option<i32>,
    #[serde(default)]
    pub shelf_life_enforced: bool,
    pub change_reason: String,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    /// Identity of the actor performing this change (for audit trail).
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivateRevisionRequest {
    pub tenant_id: String,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    /// Identity of the actor performing this change (for audit trail).
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateRevisionPolicyRequest {
    pub tenant_id: String,
    pub traceability_level: String,
    pub inspection_required: bool,
    pub shelf_life_days: Option<i32>,
    pub shelf_life_enforced: bool,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    /// Identity of the actor performing this change (for audit trail).
    #[serde(default)]
    pub actor_id: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum RevisionError {
    #[error("Item not found")]
    ItemNotFound,

    #[error("Item is inactive")]
    ItemInactive,

    #[error("Revision not found")]
    RevisionNotFound,

    #[error("Revision already activated")]
    AlreadyActivated,

    #[error("Policy flags can only be updated on draft revisions")]
    PolicyLockedOnActivatedRevision,

    #[error("Effective window overlap: another revision covers this period")]
    OverlappingWindow,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Idempotency key conflict: same key used with a different request")]
    ConflictingIdempotencyKey,

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal DB helpers
// ============================================================================

#[derive(sqlx::FromRow)]
pub(super) struct IdempotencyRecord {
    pub(super) response_body: String,
    pub(super) request_hash: String,
}

pub(super) fn require_non_empty(value: &str, field: &str) -> Result<(), RevisionError> {
    if value.trim().is_empty() {
        return Err(RevisionError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

pub(super) fn default_traceability_level() -> String {
    "none".to_string()
}

pub(super) fn normalize_traceability_level(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(super) fn validate_create_request(req: &CreateRevisionRequest) -> Result<(), RevisionError> {
    require_non_empty(&req.tenant_id, "tenant_id")?;
    require_non_empty(&req.name, "name")?;
    require_non_empty(&req.uom, "uom")?;
    require_non_empty(&req.inventory_account_ref, "inventory_account_ref")?;
    require_non_empty(&req.cogs_account_ref, "cogs_account_ref")?;
    require_non_empty(&req.variance_account_ref, "variance_account_ref")?;
    require_non_empty(&req.change_reason, "change_reason")?;
    require_non_empty(&req.idempotency_key, "idempotency_key")?;
    validate_policy_flags(
        &req.traceability_level,
        req.inspection_required,
        req.shelf_life_days,
        req.shelf_life_enforced,
    )?;
    Ok(())
}

pub(super) fn validate_activate_request(
    req: &ActivateRevisionRequest,
) -> Result<(), RevisionError> {
    require_non_empty(&req.tenant_id, "tenant_id")?;
    require_non_empty(&req.idempotency_key, "idempotency_key")?;
    if let Some(ref to) = req.effective_to {
        if *to <= req.effective_from {
            return Err(RevisionError::Validation(
                "effective_to must be after effective_from".to_string(),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_update_policy_request(
    req: &UpdateRevisionPolicyRequest,
) -> Result<(), RevisionError> {
    require_non_empty(&req.tenant_id, "tenant_id")?;
    require_non_empty(&req.traceability_level, "traceability_level")?;
    require_non_empty(&req.idempotency_key, "idempotency_key")?;
    validate_policy_flags(
        &req.traceability_level,
        req.inspection_required,
        req.shelf_life_days,
        req.shelf_life_enforced,
    )
}

fn validate_policy_flags(
    traceability_level: &str,
    _inspection_required: bool,
    shelf_life_days: Option<i32>,
    shelf_life_enforced: bool,
) -> Result<(), RevisionError> {
    let traceability_level = normalize_traceability_level(traceability_level);
    if !matches!(
        traceability_level.as_str(),
        "none" | "lot" | "serial" | "batch"
    ) {
        return Err(RevisionError::Validation(
            "traceability_level must be one of: none, lot, serial, batch".to_string(),
        ));
    }

    if let Some(days) = shelf_life_days {
        if days <= 0 {
            return Err(RevisionError::Validation(
                "shelf_life_days must be positive when provided".to_string(),
            ));
        }
    }

    if shelf_life_enforced && shelf_life_days.is_none() {
        return Err(RevisionError::Validation(
            "shelf_life_days is required when shelf_life_enforced is true".to_string(),
        ));
    }
    Ok(())
}

pub(super) async fn guard_item_exists_active(
    pool: &sqlx::PgPool,
    item_id: Uuid,
    tenant_id: &str,
) -> Result<(), RevisionError> {
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT active FROM items WHERE id = $1 AND tenant_id = $2")
            .bind(item_id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?;

    match row {
        None => Err(RevisionError::ItemNotFound),
        Some((false,)) => Err(RevisionError::ItemInactive),
        Some((true,)) => Ok(()),
    }
}

pub(super) async fn find_idempotency_key(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    idempotency_key: &str,
) -> Result<Option<IdempotencyRecord>, sqlx::Error> {
    sqlx::query_as::<_, IdempotencyRecord>(
        r#"
        SELECT response_body::TEXT AS response_body, request_hash
        FROM inv_idempotency_keys
        WHERE tenant_id = $1 AND idempotency_key = $2
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .fetch_optional(pool)
    .await
}

pub(super) async fn insert_outbox_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
    event_type: &str,
    aggregate_id: &str,
    tenant_id: &str,
    envelope_json: &str,
    correlation_id: &str,
    causation_id: &Option<String>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'item_revision', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(event_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(super) async fn store_idempotency_key(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    idempotency_key: &str,
    request_hash: &str,
    response_json: &str,
    status_code: i16,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, $5, $6)
        "#,
    )
    .bind(tenant_id)
    .bind(idempotency_key)
    .bind(request_hash)
    .bind(response_json)
    .bind(status_code)
    .bind(expires_at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn valid_create() -> CreateRevisionRequest {
        CreateRevisionRequest {
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            name: "Widget v2".to_string(),
            description: None,
            uom: "ea".to_string(),
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            traceability_level: "none".to_string(),
            inspection_required: false,
            shelf_life_days: None,
            shelf_life_enforced: false,
            change_reason: "Updated specs".to_string(),
            idempotency_key: "idem-1".to_string(),
            correlation_id: None,
            causation_id: None,
            actor_id: None,
        }
    }

    #[test]
    fn create_request_valid() {
        assert!(validate_create_request(&valid_create()).is_ok());
    }

    #[test]
    fn create_request_empty_name_rejected() {
        let mut r = valid_create();
        r.name = "  ".to_string();
        assert!(matches!(
            validate_create_request(&r),
            Err(RevisionError::Validation(_))
        ));
    }

    #[test]
    fn create_request_empty_change_reason_rejected() {
        let mut r = valid_create();
        r.change_reason = "".to_string();
        assert!(matches!(
            validate_create_request(&r),
            Err(RevisionError::Validation(_))
        ));
    }

    #[test]
    fn create_request_empty_idempotency_key_rejected() {
        let mut r = valid_create();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(
            validate_create_request(&r),
            Err(RevisionError::Validation(_))
        ));
    }

    #[test]
    fn activate_request_valid() {
        let req = ActivateRevisionRequest {
            tenant_id: "t1".to_string(),
            effective_from: Utc::now(),
            effective_to: None,
            idempotency_key: "act-1".to_string(),
            correlation_id: None,
            causation_id: None,
            actor_id: None,
        };
        assert!(validate_activate_request(&req).is_ok());
    }

    #[test]
    fn activate_request_rejects_inverted_window() {
        let now = Utc::now();
        let req = ActivateRevisionRequest {
            tenant_id: "t1".to_string(),
            effective_from: now,
            effective_to: Some(now - Duration::hours(1)),
            idempotency_key: "act-2".to_string(),
            correlation_id: None,
            causation_id: None,
            actor_id: None,
        };
        assert!(matches!(
            validate_activate_request(&req),
            Err(RevisionError::Validation(_))
        ));
    }

    #[test]
    fn create_request_rejects_invalid_traceability_level() {
        let mut r = valid_create();
        r.traceability_level = "invalid".to_string();
        assert!(matches!(
            validate_create_request(&r),
            Err(RevisionError::Validation(_))
        ));
    }

    #[test]
    fn create_request_rejects_shelf_life_enforced_without_days() {
        let mut r = valid_create();
        r.shelf_life_enforced = true;
        r.shelf_life_days = None;
        assert!(matches!(
            validate_create_request(&r),
            Err(RevisionError::Validation(_))
        ));
    }
}
