//! Item classifications and commodity codes.
//!
//! Assigns taxonomy codes (internal categories, UNSPSC, NAICS, HS, ECCN)
//! to items for reporting, compliance, and downstream routing.
//!
//! Invariants:
//! - Each (tenant, item, classification_system, code) is unique
//! - Idempotent assignment via idempotency_key
//! - All writes follow Guard → Mutation → Outbox pattern
//! - Tenant-scoped: all queries filter by tenant_id

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::history::change_history::{record_change_in_tx, RecordChangeRequest};
use crate::events::{
    build_classification_assigned_envelope, ClassificationAssignedPayload,
    EVENT_TYPE_CLASSIFICATION_ASSIGNED,
};

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ItemClassification {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub classification_system: String,
    pub classification_code: String,
    pub classification_label: Option<String>,
    pub commodity_system: Option<String>,
    pub commodity_code: Option<String>,
    pub assigned_by: String,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct AssignClassificationRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Option<Uuid>,
    pub classification_system: String,
    pub classification_code: String,
    pub classification_label: Option<String>,
    pub commodity_system: Option<String>,
    pub commodity_code: Option<String>,
    #[serde(default = "default_actor")]
    pub assigned_by: String,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

fn default_actor() -> String {
    "system".to_string()
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ClassificationError {
    #[error("Item not found")]
    ItemNotFound,

    #[error("Item is inactive")]
    ItemInactive,

    #[error("Duplicate classification: this system+code is already assigned to this item")]
    DuplicateAssignment,

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
// Internal helpers
// ============================================================================

#[derive(sqlx::FromRow)]
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

fn require_non_empty(value: &str, field: &str) -> Result<(), ClassificationError> {
    if value.trim().is_empty() {
        return Err(ClassificationError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

fn validate_request(req: &AssignClassificationRequest) -> Result<(), ClassificationError> {
    require_non_empty(&req.tenant_id, "tenant_id")?;
    require_non_empty(&req.classification_system, "classification_system")?;
    require_non_empty(&req.classification_code, "classification_code")?;
    require_non_empty(&req.assigned_by, "assigned_by")?;
    require_non_empty(&req.idempotency_key, "idempotency_key")?;

    // If commodity_system is provided, commodity_code must also be provided
    if req.commodity_system.is_some() && req.commodity_code.is_none() {
        return Err(ClassificationError::Validation(
            "commodity_code is required when commodity_system is provided".to_string(),
        ));
    }
    if req.commodity_code.is_some() && req.commodity_system.is_none() {
        return Err(ClassificationError::Validation(
            "commodity_system is required when commodity_code is provided".to_string(),
        ));
    }

    // Validate non-empty commodity fields when present
    if let Some(ref sys) = req.commodity_system {
        require_non_empty(sys, "commodity_system")?;
    }
    if let Some(ref code) = req.commodity_code {
        require_non_empty(code, "commodity_code")?;
    }

    Ok(())
}

async fn guard_item_exists_active(
    pool: &PgPool,
    item_id: Uuid,
    tenant_id: &str,
) -> Result<(), ClassificationError> {
    let row: Option<(bool,)> =
        sqlx::query_as("SELECT active FROM items WHERE id = $1 AND tenant_id = $2")
            .bind(item_id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?;

    match row {
        None => Err(ClassificationError::ItemNotFound),
        Some((false,)) => Err(ClassificationError::ItemInactive),
        Some((true,)) => Ok(()),
    }
}

async fn find_idempotency_key(
    pool: &PgPool,
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

// ============================================================================
// Assign classification service
// ============================================================================

/// Assign a classification and optional commodity code to an item.
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemClassification, is_replay)`.
pub async fn assign_classification(
    pool: &PgPool,
    req: &AssignClassificationRequest,
) -> Result<(ItemClassification, bool), ClassificationError> {
    // --- Guard: validate inputs ---
    validate_request(req)?;

    // --- Guard: idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(ClassificationError::ConflictingIdempotencyKey);
        }
        let result: ItemClassification = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    guard_item_exists_active(pool, req.item_id, &req.tenant_id).await?;

    // --- Mutation + Outbox in single transaction ---
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    let classification = sqlx::query_as::<_, ItemClassification>(
        r#"
        INSERT INTO item_classifications
            (tenant_id, item_id, revision_id,
             classification_system, classification_code, classification_label,
             commodity_system, commodity_code,
             assigned_by, idempotency_key, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING *
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.revision_id)
    .bind(req.classification_system.trim())
    .bind(req.classification_code.trim())
    .bind(req.classification_label.as_deref())
    .bind(req.commodity_system.as_deref())
    .bind(req.commodity_code.as_deref())
    .bind(req.assigned_by.trim())
    .bind(&req.idempotency_key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            let code = dbe.code();
            if code.as_deref() == Some("23505") {
                // Check constraint name to distinguish duplicate assignment vs idempotency
                let msg = dbe.message();
                if msg.contains("item_classifications_tenant_idemp_unique") {
                    return ClassificationError::ConflictingIdempotencyKey;
                }
                return ClassificationError::DuplicateAssignment;
            }
        }
        ClassificationError::Database(e)
    })?;

    // Outbox event
    let payload = ClassificationAssignedPayload {
        classification_id: classification.id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_id: req.revision_id,
        classification_system: classification.classification_system.clone(),
        classification_code: classification.classification_code.clone(),
        classification_label: classification.classification_label.clone(),
        commodity_system: classification.commodity_system.clone(),
        commodity_code: classification.commodity_code.clone(),
        assigned_by: classification.assigned_by.clone(),
        assigned_at: now,
    };
    let envelope = build_classification_assigned_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'item_classification', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_CLASSIFICATION_ASSIGNED)
    .bind(classification.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Change history
    let diff = serde_json::json!({
        "classification_system": { "after": classification.classification_system },
        "classification_code": { "after": classification.classification_code },
        "classification_label": { "after": classification.classification_label },
        "commodity_system": { "after": classification.commodity_system },
        "commodity_code": { "after": classification.commodity_code },
    });
    let change_req = RecordChangeRequest {
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_id: req.revision_id,
        change_type: "classification_assigned".to_string(),
        actor_id: req.assigned_by.clone(),
        diff,
        reason: None,
        idempotency_key: format!("ch-{}", req.idempotency_key),
        correlation_id: Some(correlation_id.clone()),
        causation_id: req.causation_id.clone(),
    };
    record_change_in_tx(&mut tx, &change_req)
        .await
        .map_err(|e| ClassificationError::Database(sqlx::Error::Protocol(e.to_string())))?;

    // Idempotency key
    let response_json = serde_json::to_string(&classification)?;
    let expires_at = now + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((classification, false))
}

// ============================================================================
// Query: list classifications for an item
// ============================================================================

/// List all classifications for a (tenant, item).
pub async fn list_classifications(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<ItemClassification>, ClassificationError> {
    let rows = sqlx::query_as::<_, ItemClassification>(
        r#"
        SELECT * FROM item_classifications
        WHERE tenant_id = $1 AND item_id = $2
        ORDER BY classification_system ASC, classification_code ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// List all items with a given classification (for filtering).
pub async fn list_items_by_classification(
    pool: &PgPool,
    tenant_id: &str,
    classification_system: &str,
    classification_code: &str,
) -> Result<Vec<ItemClassification>, ClassificationError> {
    let rows = sqlx::query_as::<_, ItemClassification>(
        r#"
        SELECT * FROM item_classifications
        WHERE tenant_id = $1
          AND classification_system = $2
          AND classification_code = $3
        ORDER BY created_at ASC
        "#,
    )
    .bind(tenant_id)
    .bind(classification_system)
    .bind(classification_code)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ============================================================================
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> AssignClassificationRequest {
        AssignClassificationRequest {
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            revision_id: None,
            classification_system: "UNSPSC".to_string(),
            classification_code: "31162800".to_string(),
            classification_label: Some("Fasteners".to_string()),
            commodity_system: Some("UNSPSC".to_string()),
            commodity_code: Some("31162800".to_string()),
            assigned_by: "user-1".to_string(),
            idempotency_key: "cls-1".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn valid_request_passes() {
        assert!(validate_request(&valid_request()).is_ok());
    }

    #[test]
    fn empty_tenant_id_rejected() {
        let mut r = valid_request();
        r.tenant_id = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ClassificationError::Validation(_))
        ));
    }

    #[test]
    fn empty_classification_system_rejected() {
        let mut r = valid_request();
        r.classification_system = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ClassificationError::Validation(_))
        ));
    }

    #[test]
    fn empty_classification_code_rejected() {
        let mut r = valid_request();
        r.classification_code = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ClassificationError::Validation(_))
        ));
    }

    #[test]
    fn commodity_system_without_code_rejected() {
        let mut r = valid_request();
        r.commodity_system = Some("UNSPSC".to_string());
        r.commodity_code = None;
        assert!(matches!(
            validate_request(&r),
            Err(ClassificationError::Validation(_))
        ));
    }

    #[test]
    fn commodity_code_without_system_rejected() {
        let mut r = valid_request();
        r.commodity_system = None;
        r.commodity_code = Some("31162800".to_string());
        assert!(matches!(
            validate_request(&r),
            Err(ClassificationError::Validation(_))
        ));
    }

    #[test]
    fn no_commodity_fields_is_valid() {
        let mut r = valid_request();
        r.commodity_system = None;
        r.commodity_code = None;
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn empty_idempotency_key_rejected() {
        let mut r = valid_request();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(ClassificationError::Validation(_))
        ));
    }
}
