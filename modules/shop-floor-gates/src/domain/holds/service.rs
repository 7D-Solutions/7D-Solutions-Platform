use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{self, HoldCancelledPayload, HoldPlacedPayload, HoldReleasedPayload};
use crate::outbox::enqueue_event_tx;
use platform_http_contracts::ApiError;

use super::{repo, CancelHoldRequest, ListHoldsQuery, PlaceHoldRequest, ReleaseHoldRequest, TravelerHold};

const VALID_HOLD_TYPES: &[&str] = &["quality", "engineering", "material", "customer", "other"];
const VALID_SCOPES: &[&str] = &["work_order", "operation"];
const VALID_RELEASE_AUTHORITIES: &[&str] = &["quality", "engineering", "planner", "supervisor", "owner_only", "any_with_role"];

// System actor for auto-transitions triggered by production events
pub const SYSTEM_ACTOR: Uuid = Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

fn generate_hold_number() -> String {
    format!("HLD-{:06}", fastrand::u32(1..=999999))
}

pub async fn place_hold(
    pool: &PgPool,
    tenant_id: &str,
    placed_by: Uuid,
    req: PlaceHoldRequest,
) -> Result<TravelerHold, ApiError> {
    if !VALID_HOLD_TYPES.contains(&req.hold_type.as_str()) {
        return Err(ApiError::bad_request(format!("Invalid hold_type: {}", req.hold_type)));
    }
    if !VALID_SCOPES.contains(&req.scope.as_str()) {
        return Err(ApiError::bad_request(format!("Invalid scope: {}", req.scope)));
    }
    if req.scope == "operation" && req.operation_id.is_none() {
        return Err(ApiError::bad_request("operation_id required when scope is 'operation'"));
    }
    let release_authority = req.release_authority.as_deref().unwrap_or("any_with_role");
    if !VALID_RELEASE_AUTHORITIES.contains(&release_authority) {
        return Err(ApiError::bad_request(format!("Invalid release_authority: {}", release_authority)));
    }

    let now = Utc::now();
    let hold = TravelerHold {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        hold_number: generate_hold_number(),
        hold_type: req.hold_type,
        scope: req.scope,
        work_order_id: req.work_order_id,
        operation_id: req.operation_id,
        reason: req.reason,
        status: "active".to_string(),
        release_authority: release_authority.to_string(),
        placed_by,
        placed_at: now,
        released_by: None,
        released_at: None,
        release_notes: None,
        cancelled_by: None,
        cancelled_at: None,
        cancel_reason: None,
        created_at: now,
        updated_at: now,
    };

    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        r#"INSERT INTO traveler_holds
           (id, tenant_id, hold_number, hold_type, scope, work_order_id, operation_id,
            reason, status, release_authority, placed_by, placed_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)"#,
    )
    .bind(hold.id)
    .bind(&hold.tenant_id)
    .bind(&hold.hold_number)
    .bind(&hold.hold_type)
    .bind(&hold.scope)
    .bind(hold.work_order_id)
    .bind(hold.operation_id)
    .bind(&hold.reason)
    .bind(&hold.status)
    .bind(&hold.release_authority)
    .bind(hold.placed_by)
    .bind(hold.placed_at)
    .bind(hold.created_at)
    .bind(hold.updated_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let payload = serde_json::to_value(HoldPlacedPayload {
        tenant_id: hold.tenant_id.clone(),
        hold_id: hold.id,
        hold_number: hold.hold_number.clone(),
        hold_type: hold.hold_type.clone(),
        scope: hold.scope.clone(),
        work_order_id: hold.work_order_id,
        operation_id: hold.operation_id,
        placed_by: hold.placed_by,
        placed_at: hold.placed_at,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HOLD_PLACED, "traveler_hold", &hold.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(hold)
}

pub async fn release_hold(
    pool: &PgPool,
    tenant_id: &str,
    hold_id: Uuid,
    released_by: Uuid,
    req: ReleaseHoldRequest,
    // release_authority_role: the role/type of the user releasing
    user_role: &str,
    is_system: bool,
) -> Result<TravelerHold, ApiError> {
    let hold = repo::fetch_hold(pool, hold_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Hold not found"))?;

    if hold.status != "active" {
        return Err(ApiError::bad_request(format!("Hold is already {}", hold.status)));
    }

    // Enforce release authority — system bypasses
    if !is_system {
        let ok = match hold.release_authority.as_str() {
            "any_with_role" => true,
            "owner_only" => released_by == hold.placed_by,
            authority => user_role == authority,
        };
        if !ok {
            return Err(ApiError::forbidden("Insufficient authority to release this hold"));
        }
    }

    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, TravelerHold>(
        r#"UPDATE traveler_holds
           SET status = 'released', released_by = $3, released_at = NOW(),
               release_notes = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'active'
           RETURNING *"#,
    )
    .bind(hold_id)
    .bind(tenant_id)
    .bind(released_by)
    .bind(&req.release_notes)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::conflict("Hold was modified concurrently"))?;

    let payload = serde_json::to_value(HoldReleasedPayload {
        tenant_id: updated.tenant_id.clone(),
        hold_id: updated.id,
        hold_number: updated.hold_number.clone(),
        work_order_id: updated.work_order_id,
        released_by,
        released_at: updated.released_at.unwrap_or_else(Utc::now),
        release_notes: req.release_notes,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HOLD_RELEASED, "traveler_hold", &updated.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn cancel_hold(
    pool: &PgPool,
    tenant_id: &str,
    hold_id: Uuid,
    cancelled_by: Uuid,
    req: CancelHoldRequest,
) -> Result<TravelerHold, ApiError> {
    let hold = repo::fetch_hold(pool, hold_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Hold not found"))?;

    if hold.status != "active" {
        return Err(ApiError::bad_request(format!("Hold is already {}", hold.status)));
    }

    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, TravelerHold>(
        r#"UPDATE traveler_holds
           SET status = 'cancelled', cancelled_by = $3, cancelled_at = NOW(),
               cancel_reason = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'active'
           RETURNING *"#,
    )
    .bind(hold_id)
    .bind(tenant_id)
    .bind(cancelled_by)
    .bind(&req.cancel_reason)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::conflict("Hold was modified concurrently"))?;

    let payload = serde_json::to_value(HoldCancelledPayload {
        tenant_id: updated.tenant_id.clone(),
        hold_id: updated.id,
        hold_number: updated.hold_number.clone(),
        work_order_id: updated.work_order_id,
        cancelled_by,
        cancelled_at: updated.cancelled_at.unwrap_or_else(Utc::now),
        cancel_reason: req.cancel_reason,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HOLD_CANCELLED, "traveler_hold", &updated.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn list_holds(pool: &PgPool, tenant_id: &str, q: ListHoldsQuery) -> Result<Vec<TravelerHold>, ApiError> {
    repo::list_holds(pool, tenant_id, &q).await.map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn get_hold(pool: &PgPool, hold_id: Uuid, tenant_id: &str) -> Result<TravelerHold, ApiError> {
    repo::fetch_hold(pool, hold_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Hold not found"))
}
