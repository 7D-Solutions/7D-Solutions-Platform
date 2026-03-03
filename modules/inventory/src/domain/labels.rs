//! Barcode/label generation for inventory traceability.
//!
//! Labels are durable records that capture everything needed to print or reprint
//! a physical label. The payload is deterministic: given the same item revision
//! context and label type, the same payload is produced.
//!
//! Invariants:
//! - Every label references an item revision for audit trail
//! - Idempotent generation via idempotency_key (same key = same label)
//! - All writes follow Guard → Mutation → Outbox pattern
//! - Tenant-scoped: labels cannot leak across tenants

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::revisions::ItemRevision;
use crate::events::{
    build_label_generated_envelope, LabelGeneratedPayload, EVENT_TYPE_LABEL_GENERATED,
};

// ============================================================================
// Domain model
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Label {
    pub id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Uuid,
    pub label_type: String,
    pub barcode_format: String,
    pub payload: serde_json::Value,
    pub idempotency_key: Option<String>,
    pub actor_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateLabelRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub revision_id: Uuid,
    pub label_type: String,
    #[serde(default = "default_barcode_format")]
    pub barcode_format: String,
    /// Optional extra data merged into the label payload (e.g. lot_code).
    #[serde(default)]
    pub extra: Option<serde_json::Value>,
    pub idempotency_key: String,
    pub actor_id: Option<Uuid>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

fn default_barcode_format() -> String {
    "code128".to_string()
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum LabelError {
    #[error("Item not found")]
    ItemNotFound,

    #[error("Item is inactive")]
    ItemInactive,

    #[error("Revision not found")]
    RevisionNotFound,

    #[error("Revision belongs to a different item or tenant")]
    RevisionMismatch,

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

fn require_non_empty(value: &str, field: &str) -> Result<(), LabelError> {
    if value.trim().is_empty() {
        return Err(LabelError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

const VALID_LABEL_TYPES: &[&str] = &["item_label", "lot_label"];
const VALID_BARCODE_FORMATS: &[&str] = &["code128", "code39", "qr", "datamatrix", "ean13"];

fn validate_request(req: &GenerateLabelRequest) -> Result<(), LabelError> {
    require_non_empty(&req.tenant_id, "tenant_id")?;
    require_non_empty(&req.idempotency_key, "idempotency_key")?;
    require_non_empty(&req.label_type, "label_type")?;
    require_non_empty(&req.barcode_format, "barcode_format")?;

    if !VALID_LABEL_TYPES.contains(&req.label_type.as_str()) {
        return Err(LabelError::Validation(format!(
            "label_type must be one of: {}",
            VALID_LABEL_TYPES.join(", ")
        )));
    }

    if !VALID_BARCODE_FORMATS.contains(&req.barcode_format.as_str()) {
        return Err(LabelError::Validation(format!(
            "barcode_format must be one of: {}",
            VALID_BARCODE_FORMATS.join(", ")
        )));
    }

    Ok(())
}

/// Build a deterministic label payload from the item revision context.
///
/// The payload includes all data needed for rendering: barcode value, item
/// metadata, revision details, and any extra caller-supplied data.
fn build_label_payload(
    item_sku: &str,
    revision: &ItemRevision,
    label_type: &str,
    extra: &Option<serde_json::Value>,
) -> serde_json::Value {
    let barcode_value = format!(
        "{}-R{}",
        item_sku,
        revision.revision_number
    );

    let mut payload = serde_json::json!({
        "barcode_value": barcode_value,
        "item_sku": item_sku,
        "item_name": revision.name,
        "uom": revision.uom,
        "revision_number": revision.revision_number,
        "traceability_level": revision.traceability_level,
        "label_type": label_type,
    });

    if let Some(desc) = &revision.description {
        payload["description"] = serde_json::Value::String(desc.clone());
    }

    if let Some(extra_data) = extra {
        if let serde_json::Value::Object(map) = extra_data {
            if let serde_json::Value::Object(ref mut p) = payload {
                for (k, v) in map {
                    p.insert(k.clone(), v.clone());
                }
            }
        }
    }

    payload
}

// ============================================================================
// Generate label service
// ============================================================================

/// Generate a label record with deterministic payload.
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(Label, is_replay)`.
pub async fn generate_label(
    pool: &PgPool,
    req: &GenerateLabelRequest,
) -> Result<(Label, bool), LabelError> {
    // --- Guard: validate inputs ---
    validate_request(req)?;

    // --- Guard: idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(LabelError::ConflictingIdempotencyKey);
        }
        let result: Label = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    let item = guard_item_exists_active(pool, req.item_id, &req.tenant_id).await?;

    // --- Guard: revision must exist and belong to the item+tenant ---
    let revision = guard_revision_exists(pool, req.revision_id, req.item_id, &req.tenant_id).await?;

    // --- Build deterministic payload ---
    let label_payload = build_label_payload(&item.sku, &revision, &req.label_type, &req.extra);

    // --- Mutation + Outbox in single transaction ---
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    let label = sqlx::query_as::<_, Label>(
        r#"
        INSERT INTO inv_labels
            (tenant_id, item_id, revision_id, label_type, barcode_format,
             payload, idempotency_key, actor_id, created_at)
        VALUES ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, $9)
        RETURNING *
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.revision_id)
    .bind(&req.label_type)
    .bind(&req.barcode_format)
    .bind(&label_payload)
    .bind(&req.idempotency_key)
    .bind(req.actor_id)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.code().as_deref() == Some("23505") {
                return LabelError::ConflictingIdempotencyKey;
            }
        }
        LabelError::Database(e)
    })?;

    // Outbox event
    let event_payload = LabelGeneratedPayload {
        label_id: label.id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_id: req.revision_id,
        revision_number: revision.revision_number,
        label_type: req.label_type.clone(),
        barcode_format: req.barcode_format.clone(),
        payload: label_payload,
        actor_id: req.actor_id,
        created_at: now,
    };
    let envelope = build_label_generated_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        event_payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES ($1, $2, 'label', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_LABEL_GENERATED)
    .bind(label.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Idempotency key
    let response_json = serde_json::to_string(&label)?;
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
    Ok((label, false))
}

// ============================================================================
// Queries
// ============================================================================

/// Fetch a single label by ID, scoped to tenant.
pub async fn get_label(
    pool: &PgPool,
    tenant_id: &str,
    label_id: Uuid,
) -> Result<Option<Label>, LabelError> {
    let label = sqlx::query_as::<_, Label>(
        "SELECT * FROM inv_labels WHERE id = $1 AND tenant_id = $2",
    )
    .bind(label_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;

    Ok(label)
}

/// List all labels for an item, ordered by created_at descending.
pub async fn list_labels(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<Label>, LabelError> {
    let labels = sqlx::query_as::<_, Label>(
        r#"
        SELECT * FROM inv_labels
        WHERE tenant_id = $1 AND item_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await?;

    Ok(labels)
}

// ============================================================================
// Guards
// ============================================================================

#[derive(sqlx::FromRow)]
struct ItemRow {
    sku: String,
    active: bool,
}

async fn guard_item_exists_active(
    pool: &PgPool,
    item_id: Uuid,
    tenant_id: &str,
) -> Result<ItemRow, LabelError> {
    let row = sqlx::query_as::<_, ItemRow>(
        "SELECT sku, active FROM items WHERE id = $1 AND tenant_id = $2",
    )
    .bind(item_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or(LabelError::ItemNotFound)?;

    if !row.active {
        return Err(LabelError::ItemInactive);
    }

    Ok(row)
}

async fn guard_revision_exists(
    pool: &PgPool,
    revision_id: Uuid,
    item_id: Uuid,
    tenant_id: &str,
) -> Result<ItemRevision, LabelError> {
    sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(revision_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .and_then(|rev| {
        if rev.item_id == item_id {
            Some(rev)
        } else {
            None
        }
    })
    .ok_or(LabelError::RevisionNotFound)
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
// Unit tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> GenerateLabelRequest {
        GenerateLabelRequest {
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
            label_type: "item_label".to_string(),
            barcode_format: "code128".to_string(),
            extra: None,
            idempotency_key: "idem-1".to_string(),
            actor_id: None,
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn valid_request_passes_validation() {
        assert!(validate_request(&valid_request()).is_ok());
    }

    #[test]
    fn empty_tenant_rejected() {
        let mut r = valid_request();
        r.tenant_id = "  ".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(LabelError::Validation(_))
        ));
    }

    #[test]
    fn empty_idempotency_key_rejected() {
        let mut r = valid_request();
        r.idempotency_key = "".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(LabelError::Validation(_))
        ));
    }

    #[test]
    fn invalid_label_type_rejected() {
        let mut r = valid_request();
        r.label_type = "pallet_label".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(LabelError::Validation(_))
        ));
    }

    #[test]
    fn invalid_barcode_format_rejected() {
        let mut r = valid_request();
        r.barcode_format = "pdf417".to_string();
        assert!(matches!(
            validate_request(&r),
            Err(LabelError::Validation(_))
        ));
    }

    #[test]
    fn lot_label_type_accepted() {
        let mut r = valid_request();
        r.label_type = "lot_label".to_string();
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn qr_format_accepted() {
        let mut r = valid_request();
        r.barcode_format = "qr".to_string();
        assert!(validate_request(&r).is_ok());
    }

    #[test]
    fn deterministic_payload_generation() {
        let revision = ItemRevision {
            id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            revision_number: 3,
            name: "Widget Pro".to_string(),
            description: Some("Premium widget".to_string()),
            uom: "ea".to_string(),
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            traceability_level: "lot".to_string(),
            inspection_required: true,
            shelf_life_days: Some(90),
            shelf_life_enforced: true,
            effective_from: None,
            effective_to: None,
            change_reason: "test".to_string(),
            idempotency_key: None,
            created_at: Utc::now(),
            activated_at: None,
        };

        let p1 = build_label_payload("SKU-001", &revision, "item_label", &None);
        let p2 = build_label_payload("SKU-001", &revision, "item_label", &None);
        assert_eq!(p1, p2, "same inputs must produce same payload");

        assert_eq!(p1["barcode_value"], "SKU-001-R3");
        assert_eq!(p1["item_sku"], "SKU-001");
        assert_eq!(p1["item_name"], "Widget Pro");
        assert_eq!(p1["uom"], "ea");
        assert_eq!(p1["revision_number"], 3);
        assert_eq!(p1["traceability_level"], "lot");
        assert_eq!(p1["description"], "Premium widget");
    }

    #[test]
    fn payload_merges_extra_data() {
        let revision = ItemRevision {
            id: Uuid::new_v4(),
            tenant_id: "t1".to_string(),
            item_id: Uuid::new_v4(),
            revision_number: 1,
            name: "Widget".to_string(),
            description: None,
            uom: "ea".to_string(),
            inventory_account_ref: "1200".to_string(),
            cogs_account_ref: "5000".to_string(),
            variance_account_ref: "5010".to_string(),
            traceability_level: "lot".to_string(),
            inspection_required: false,
            shelf_life_days: None,
            shelf_life_enforced: false,
            effective_from: None,
            effective_to: None,
            change_reason: "test".to_string(),
            idempotency_key: None,
            created_at: Utc::now(),
            activated_at: None,
        };

        let extra = Some(serde_json::json!({"lot_code": "LOT-2026-001", "quantity": 50}));
        let p = build_label_payload("SKU-002", &revision, "lot_label", &extra);

        assert_eq!(p["lot_code"], "LOT-2026-001");
        assert_eq!(p["quantity"], 50);
        assert_eq!(p["barcode_value"], "SKU-002-R1");
    }
}
