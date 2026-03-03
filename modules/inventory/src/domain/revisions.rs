//! Item revision management with effective dating.
//!
//! Revisions are the audit spine of item master data. Each revision captures
//! a snapshot of the item definition (name, UoM, GL accounts) and can be
//! activated for an effective window [effective_from, effective_to).
//!
//! Invariants:
//! - revision_number auto-increments per (tenant_id, item_id)
//! - Effective windows are non-overlapping (DB exclusion constraint)
//! - Activating a new revision auto-closes any open-ended predecessor
//! - Idempotent creation and activation via idempotency_key
//! - All writes follow Guard → Mutation → Outbox pattern

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::events::{
    build_item_revision_activated_envelope, build_item_revision_created_envelope,
    build_item_revision_policy_updated_envelope, ItemRevisionActivatedPayload,
    ItemRevisionCreatedPayload, ItemRevisionPolicyUpdatedPayload,
    EVENT_TYPE_ITEM_REVISION_ACTIVATED, EVENT_TYPE_ITEM_REVISION_CREATED,
    EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED,
};

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivateRevisionRequest {
    pub tenant_id: String,
    pub effective_from: DateTime<Utc>,
    pub effective_to: Option<DateTime<Utc>>,
    pub idempotency_key: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
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
struct IdempotencyRecord {
    response_body: String,
    request_hash: String,
}

fn require_non_empty(value: &str, field: &str) -> Result<(), RevisionError> {
    if value.trim().is_empty() {
        return Err(RevisionError::Validation(format!(
            "{} must not be empty",
            field
        )));
    }
    Ok(())
}

fn default_traceability_level() -> String {
    "none".to_string()
}

fn normalize_traceability_level(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

// ============================================================================
// Create revision service
// ============================================================================

/// Create a new item revision (draft state, not yet effective).
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemRevision, is_replay)`.
pub async fn create_revision(
    pool: &PgPool,
    req: &CreateRevisionRequest,
) -> Result<(ItemRevision, bool), RevisionError> {
    // --- Guard: validate inputs ---
    validate_create_request(req)?;

    // --- Guard: idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RevisionError::ConflictingIdempotencyKey);
        }
        let result: ItemRevision = serde_json::from_str(&record.response_body)?;
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

    // Auto-increment revision_number
    let revision = sqlx::query_as::<_, ItemRevision>(
        r#"
        INSERT INTO item_revisions
            (tenant_id, item_id, revision_number,
             name, description, uom,
             inventory_account_ref, cogs_account_ref, variance_account_ref,
             traceability_level, inspection_required, shelf_life_days, shelf_life_enforced,
             change_reason, idempotency_key, created_at)
        VALUES
            ($1, $2,
             (SELECT COALESCE(MAX(revision_number), 0) + 1
              FROM item_revisions WHERE tenant_id = $1 AND item_id = $2),
             $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        RETURNING *
        "#,
    )
    .bind(&req.tenant_id)
    .bind(req.item_id)
    .bind(req.name.trim())
    .bind(req.description.as_deref())
    .bind(req.uom.trim())
    .bind(req.inventory_account_ref.trim())
    .bind(req.cogs_account_ref.trim())
    .bind(req.variance_account_ref.trim())
    .bind(normalize_traceability_level(&req.traceability_level))
    .bind(req.inspection_required)
    .bind(req.shelf_life_days)
    .bind(req.shelf_life_enforced)
    .bind(req.change_reason.trim())
    .bind(&req.idempotency_key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            if dbe.code().as_deref() == Some("23505") {
                return RevisionError::ConflictingIdempotencyKey;
            }
        }
        RevisionError::Database(e)
    })?;

    // Outbox event
    let payload = ItemRevisionCreatedPayload {
        revision_id: revision.id,
        tenant_id: req.tenant_id.clone(),
        item_id: req.item_id,
        revision_number: revision.revision_number,
        name: revision.name.clone(),
        uom: revision.uom.clone(),
        traceability_level: revision.traceability_level.clone(),
        inspection_required: revision.inspection_required,
        shelf_life_days: revision.shelf_life_days,
        shelf_life_enforced: revision.shelf_life_enforced,
        change_reason: revision.change_reason.clone(),
        created_at: now,
    };
    let envelope = build_item_revision_created_envelope(
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
        VALUES ($1, $2, 'item_revision', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_REVISION_CREATED)
    .bind(revision.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Idempotency key
    let response_json = serde_json::to_string(&revision)?;
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
    Ok((revision, false))
}

// ============================================================================
// Activate revision service
// ============================================================================

/// Activate a revision for an effective window.
///
/// Automatically closes any currently open-ended revision for the same item
/// by setting its effective_to = this revision's effective_from.
///
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemRevision, is_replay)`.
pub async fn activate_revision(
    pool: &PgPool,
    item_id: Uuid,
    revision_id: Uuid,
    req: &ActivateRevisionRequest,
) -> Result<(ItemRevision, bool), RevisionError> {
    // --- Guard: validate inputs ---
    validate_activate_request(req)?;

    // --- Guard: idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RevisionError::ConflictingIdempotencyKey);
        }
        let result: ItemRevision = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: item must exist and be active ---
    guard_item_exists_active(pool, item_id, &req.tenant_id).await?;

    // --- Mutation + Outbox in single transaction ---
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Lock the revision row
    let revision = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE id = $1 AND item_id = $2 AND tenant_id = $3
        FOR UPDATE
        "#,
    )
    .bind(revision_id)
    .bind(item_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RevisionError::RevisionNotFound)?;

    if revision.effective_from.is_some() {
        return Err(RevisionError::AlreadyActivated);
    }

    // Close any open-ended predecessor revision for this item
    let superseded_id: Option<Uuid> = sqlx::query_scalar(
        r#"
        UPDATE item_revisions
        SET effective_to = $1, activated_at = COALESCE(activated_at, NOW())
        WHERE tenant_id = $2 AND item_id = $3
          AND effective_from IS NOT NULL AND effective_to IS NULL
          AND id != $4
        RETURNING id
        "#,
    )
    .bind(req.effective_from)
    .bind(&req.tenant_id)
    .bind(item_id)
    .bind(revision_id)
    .fetch_optional(&mut *tx)
    .await?;

    // Activate this revision
    let activated = sqlx::query_as::<_, ItemRevision>(
        r#"
        UPDATE item_revisions
        SET effective_from = $1, effective_to = $2, activated_at = $3
        WHERE id = $4 AND tenant_id = $5
        RETURNING *
        "#,
    )
    .bind(req.effective_from)
    .bind(req.effective_to)
    .bind(now)
    .bind(revision_id)
    .bind(&req.tenant_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref dbe) = e {
            // Exclusion constraint violation (overlapping windows)
            if dbe.code().as_deref() == Some("23P01") {
                return RevisionError::OverlappingWindow;
            }
        }
        RevisionError::Database(e)
    })?;

    // Outbox event
    let payload = ItemRevisionActivatedPayload {
        revision_id: activated.id,
        tenant_id: req.tenant_id.clone(),
        item_id,
        revision_number: activated.revision_number,
        traceability_level: activated.traceability_level.clone(),
        inspection_required: activated.inspection_required,
        shelf_life_days: activated.shelf_life_days,
        shelf_life_enforced: activated.shelf_life_enforced,
        effective_from: req.effective_from,
        effective_to: req.effective_to,
        superseded_revision_id: superseded_id,
        activated_at: now,
    };
    let envelope = build_item_revision_activated_envelope(
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
        VALUES ($1, $2, 'item_revision', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_REVISION_ACTIVATED)
    .bind(activated.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Idempotency key
    let response_json = serde_json::to_string(&activated)?;
    let expires_at = now + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 200, $5)
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
    Ok((activated, false))
}

/// Update policy flags on a draft revision.
///
/// Activated revisions are immutable; create a new revision to change policy.
/// Pattern: Guard → Mutation → Outbox (single transaction).
/// Returns `(ItemRevision, is_replay)`.
pub async fn update_revision_policy(
    pool: &PgPool,
    item_id: Uuid,
    revision_id: Uuid,
    req: &UpdateRevisionPolicyRequest,
) -> Result<(ItemRevision, bool), RevisionError> {
    validate_update_policy_request(req)?;

    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(RevisionError::ConflictingIdempotencyKey);
        }
        let result: ItemRevision = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    guard_item_exists_active(pool, item_id, &req.tenant_id).await?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    let current = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE id = $1 AND item_id = $2 AND tenant_id = $3
        FOR UPDATE
        "#,
    )
    .bind(revision_id)
    .bind(item_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(RevisionError::RevisionNotFound)?;

    if current.effective_from.is_some() {
        return Err(RevisionError::PolicyLockedOnActivatedRevision);
    }

    let updated = sqlx::query_as::<_, ItemRevision>(
        r#"
        UPDATE item_revisions
        SET traceability_level = $1,
            inspection_required = $2,
            shelf_life_days = $3,
            shelf_life_enforced = $4
        WHERE id = $5 AND tenant_id = $6
        RETURNING *
        "#,
    )
    .bind(normalize_traceability_level(&req.traceability_level))
    .bind(req.inspection_required)
    .bind(req.shelf_life_days)
    .bind(req.shelf_life_enforced)
    .bind(revision_id)
    .bind(&req.tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    let payload = ItemRevisionPolicyUpdatedPayload {
        revision_id: updated.id,
        tenant_id: req.tenant_id.clone(),
        item_id,
        revision_number: updated.revision_number,
        traceability_level: updated.traceability_level.clone(),
        inspection_required: updated.inspection_required,
        shelf_life_days: updated.shelf_life_days,
        shelf_life_enforced: updated.shelf_life_enforced,
        updated_at: now,
    };
    let envelope = build_item_revision_policy_updated_envelope(
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
        VALUES ($1, $2, 'item_revision', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_ITEM_REVISION_POLICY_UPDATED)
    .bind(updated.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    let response_json = serde_json::to_string(&updated)?;
    let expires_at = now + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES ($1, $2, $3, $4::JSONB, 200, $5)
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
    Ok((updated, false))
}

// ============================================================================
// Query: revision effective at time T
// ============================================================================

/// Find the revision for an item that is effective at a given timestamp.
///
/// Returns None if no revision covers the requested time.
pub async fn revision_at(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    at: DateTime<Utc>,
) -> Result<Option<ItemRevision>, RevisionError> {
    let rev = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE tenant_id = $1 AND item_id = $2
          AND effective_from IS NOT NULL
          AND effective_from <= $3
          AND (effective_to IS NULL OR effective_to > $3)
        ORDER BY effective_from DESC
        LIMIT 1
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(at)
    .fetch_optional(pool)
    .await?;

    Ok(rev)
}

/// List all revisions for an item ordered by revision_number.
pub async fn list_revisions(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
) -> Result<Vec<ItemRevision>, RevisionError> {
    let revs = sqlx::query_as::<_, ItemRevision>(
        r#"
        SELECT * FROM item_revisions
        WHERE tenant_id = $1 AND item_id = $2
        ORDER BY revision_number ASC
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .fetch_all(pool)
    .await?;

    Ok(revs)
}

// ============================================================================
// Validation helpers
// ============================================================================

fn validate_create_request(req: &CreateRevisionRequest) -> Result<(), RevisionError> {
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

fn validate_activate_request(req: &ActivateRevisionRequest) -> Result<(), RevisionError> {
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

fn validate_update_policy_request(req: &UpdateRevisionPolicyRequest) -> Result<(), RevisionError> {
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

async fn guard_item_exists_active(
    pool: &PgPool,
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
