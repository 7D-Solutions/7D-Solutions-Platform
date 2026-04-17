use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{self, HandoffAcceptedPayload, HandoffCancelledPayload, HandoffInitiatedPayload, HandoffRejectedPayload};
use crate::outbox::enqueue_event_tx;
use platform_http_contracts::ApiError;

use super::{
    repo, AcceptHandoffRequest, CancelHandoffRequest, InitiateHandoffRequest, ListHandoffsQuery,
    OperationHandoff, RejectHandoffRequest,
};

const VALID_INITIATION_TYPES: &[&str] = &["push", "pull"];

fn generate_handoff_number() -> String {
    format!("HO-{:06}", fastrand::u32(1..=999999))
}

pub async fn initiate_handoff(
    pool: &PgPool,
    tenant_id: &str,
    initiated_by: Uuid,
    req: InitiateHandoffRequest,
) -> Result<OperationHandoff, ApiError> {
    let initiation_type = req.initiation_type.as_deref().unwrap_or("push");
    if !VALID_INITIATION_TYPES.contains(&initiation_type) {
        return Err(ApiError::bad_request(format!("Invalid initiation_type: {}", initiation_type)));
    }
    if req.quantity <= 0.0 {
        return Err(ApiError::bad_request("quantity must be positive"));
    }

    let now = Utc::now();
    let handoff = OperationHandoff {
        id: Uuid::new_v4(),
        tenant_id: tenant_id.to_string(),
        handoff_number: generate_handoff_number(),
        work_order_id: req.work_order_id,
        source_operation_id: req.source_operation_id,
        dest_operation_id: req.dest_operation_id,
        initiation_type: initiation_type.to_string(),
        status: "initiated".to_string(),
        quantity: req.quantity,
        unit_of_measure: req.unit_of_measure,
        lot_number: req.lot_number,
        serial_numbers: req.serial_numbers,
        notes: req.notes,
        initiated_by,
        initiated_at: now,
        accepted_by: None,
        accepted_at: None,
        rejected_by: None,
        rejected_at: None,
        rejection_reason: None,
        cancelled_by: None,
        cancelled_at: None,
        cancel_reason: None,
        created_at: now,
        updated_at: now,
    };

    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    sqlx::query(
        r#"INSERT INTO operation_handoffs
           (id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
            initiation_type, status, quantity, unit_of_measure, lot_number, serial_numbers, notes,
            initiated_by, initiated_at, created_at, updated_at)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9::float8::numeric,$10,$11,$12,$13,$14,$15,$16,$17)"#,
    )
    .bind(handoff.id)
    .bind(&handoff.tenant_id)
    .bind(&handoff.handoff_number)
    .bind(handoff.work_order_id)
    .bind(handoff.source_operation_id)
    .bind(handoff.dest_operation_id)
    .bind(&handoff.initiation_type)
    .bind(&handoff.status)
    .bind(handoff.quantity)
    .bind(&handoff.unit_of_measure)
    .bind(&handoff.lot_number)
    .bind(&handoff.serial_numbers)
    .bind(&handoff.notes)
    .bind(handoff.initiated_by)
    .bind(handoff.initiated_at)
    .bind(handoff.created_at)
    .bind(handoff.updated_at)
    .execute(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let payload = serde_json::to_value(HandoffInitiatedPayload {
        tenant_id: handoff.tenant_id.clone(),
        handoff_id: handoff.id,
        handoff_number: handoff.handoff_number.clone(),
        work_order_id: handoff.work_order_id,
        source_operation_id: handoff.source_operation_id,
        dest_operation_id: handoff.dest_operation_id,
        initiation_type: handoff.initiation_type.clone(),
        quantity: handoff.quantity,
        unit_of_measure: handoff.unit_of_measure.clone(),
        initiated_by: handoff.initiated_by,
        initiated_at: handoff.initiated_at,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HANDOFF_INITIATED, "operation_handoff", &handoff.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(handoff)
}

pub async fn accept_handoff(
    pool: &PgPool,
    tenant_id: &str,
    handoff_id: Uuid,
    accepted_by: Uuid,
    _req: AcceptHandoffRequest,
) -> Result<OperationHandoff, ApiError> {
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, OperationHandoff>(
        r#"UPDATE operation_handoffs
           SET status = 'accepted', accepted_by = $3, accepted_at = NOW(), updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'initiated'
           RETURNING id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
               initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
               initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
               cancelled_by, cancelled_at, cancel_reason, created_at, updated_at"#,
    )
    .bind(handoff_id)
    .bind(tenant_id)
    .bind(accepted_by)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Handoff not found or not in 'initiated' status"))?;

    let payload = serde_json::to_value(HandoffAcceptedPayload {
        tenant_id: updated.tenant_id.clone(),
        handoff_id: updated.id,
        handoff_number: updated.handoff_number.clone(),
        work_order_id: updated.work_order_id,
        accepted_by,
        accepted_at: updated.accepted_at.unwrap_or_else(Utc::now),
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HANDOFF_ACCEPTED, "operation_handoff", &updated.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn reject_handoff(
    pool: &PgPool,
    tenant_id: &str,
    handoff_id: Uuid,
    rejected_by: Uuid,
    req: RejectHandoffRequest,
) -> Result<OperationHandoff, ApiError> {
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, OperationHandoff>(
        r#"UPDATE operation_handoffs
           SET status = 'rejected', rejected_by = $3, rejected_at = NOW(),
               rejection_reason = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'initiated'
           RETURNING id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
               initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
               initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
               cancelled_by, cancelled_at, cancel_reason, created_at, updated_at"#,
    )
    .bind(handoff_id)
    .bind(tenant_id)
    .bind(rejected_by)
    .bind(&req.rejection_reason)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Handoff not found or not in 'initiated' status"))?;

    let payload = serde_json::to_value(HandoffRejectedPayload {
        tenant_id: updated.tenant_id.clone(),
        handoff_id: updated.id,
        handoff_number: updated.handoff_number.clone(),
        work_order_id: updated.work_order_id,
        rejected_by,
        rejected_at: updated.rejected_at.unwrap_or_else(Utc::now),
        rejection_reason: req.rejection_reason,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HANDOFF_REJECTED, "operation_handoff", &updated.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn cancel_handoff(
    pool: &PgPool,
    tenant_id: &str,
    handoff_id: Uuid,
    cancelled_by: Uuid,
    req: CancelHandoffRequest,
) -> Result<OperationHandoff, ApiError> {
    let mut tx = pool.begin().await.map_err(|e| ApiError::internal(e.to_string()))?;

    let updated = sqlx::query_as::<_, OperationHandoff>(
        r#"UPDATE operation_handoffs
           SET status = 'cancelled', cancelled_by = $3, cancelled_at = NOW(),
               cancel_reason = $4, updated_at = NOW()
           WHERE id = $1 AND tenant_id = $2 AND status = 'initiated'
           RETURNING id, tenant_id, handoff_number, work_order_id, source_operation_id, dest_operation_id,
               initiation_type, status, quantity::float8 AS quantity, unit_of_measure, lot_number, serial_numbers, notes,
               initiated_by, initiated_at, accepted_by, accepted_at, rejected_by, rejected_at, rejection_reason,
               cancelled_by, cancelled_at, cancel_reason, created_at, updated_at"#,
    )
    .bind(handoff_id)
    .bind(tenant_id)
    .bind(cancelled_by)
    .bind(&req.cancel_reason)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Handoff not found or not in 'initiated' status"))?;

    let payload = serde_json::to_value(HandoffCancelledPayload {
        tenant_id: updated.tenant_id.clone(),
        handoff_id: updated.id,
        handoff_number: updated.handoff_number.clone(),
        work_order_id: updated.work_order_id,
        cancelled_by,
        cancelled_at: updated.cancelled_at.unwrap_or_else(Utc::now),
        cancel_reason: req.cancel_reason,
    })
    .map_err(|e| ApiError::internal(e.to_string()))?;

    enqueue_event_tx(&mut *tx, Uuid::new_v4(), events::HANDOFF_CANCELLED, "operation_handoff", &updated.id.to_string(), &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    tx.commit().await.map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(updated)
}

pub async fn list_handoffs(pool: &PgPool, tenant_id: &str, q: ListHandoffsQuery) -> Result<Vec<OperationHandoff>, ApiError> {
    repo::list_handoffs(pool, tenant_id, &q).await.map_err(|e| ApiError::internal(e.to_string()))
}

pub async fn get_handoff(pool: &PgPool, handoff_id: Uuid, tenant_id: &str) -> Result<OperationHandoff, ApiError> {
    repo::fetch_handoff(pool, handoff_id, tenant_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Handoff not found"))
}
