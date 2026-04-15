use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use platform_audit::schema::{MutationClass, WriteAuditRequest};
use platform_audit::writer::AuditWriter;

use crate::domain::idempotency::{check_idempotency, store_idempotency_key, IdempotencyError};
use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Request type
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RequestFgReceiptRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub idempotency_key: Option<String>,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum FgReceiptError {
    #[error("Work order not found")]
    NotFound,

    #[error("Work order is not in 'released' status")]
    NotReleased,

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Conflicting idempotency key")]
    ConflictingIdempotencyKey,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

/// Returns `Ok(true)` on idempotency replay, `Ok(false)` on fresh creation.
pub async fn request_fg_receipt(
    pool: &PgPool,
    work_order_id: Uuid,
    req: &RequestFgReceiptRequest,
) -> Result<bool, FgReceiptError> {
    if req.tenant_id.trim().is_empty() {
        return Err(FgReceiptError::Validation(
            "tenant_id is required".to_string(),
        ));
    }
    if req.quantity <= 0 {
        return Err(FgReceiptError::Validation(
            "quantity must be > 0".to_string(),
        ));
    }
    if req.currency.trim().is_empty() {
        return Err(FgReceiptError::Validation(
            "currency is required".to_string(),
        ));
    }

    let request_hash = serde_json::to_string(req)
        .map_err(|e| FgReceiptError::Database(sqlx::Error::Protocol(e.to_string())))?;

    let mut tx = pool.begin().await?;

    // Idempotency check
    if let Some(key) = &req.idempotency_key {
        match check_idempotency(&mut tx, &req.tenant_id, key, &request_hash).await {
            Ok(Some(_)) => {
                tx.commit().await?;
                return Ok(true);
            }
            Ok(None) => {}
            Err(IdempotencyError::Conflict) => {
                return Err(FgReceiptError::ConflictingIdempotencyKey);
            }
            Err(IdempotencyError::Database(e)) => return Err(FgReceiptError::Database(e)),
            Err(IdempotencyError::Json(e)) => {
                return Err(FgReceiptError::Database(sqlx::Error::Protocol(
                    e.to_string(),
                )));
            }
        }
    }

    let row = sqlx::query_as::<_, (String, String, Uuid)>(
        "SELECT status, order_number, item_id FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(work_order_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(FgReceiptError::NotFound)?;

    if row.0 != "released" {
        return Err(FgReceiptError::NotReleased);
    }

    let order_number = row.1;
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    enqueue_event(
        &mut tx,
        &req.tenant_id,
        ProductionEventType::FgReceiptRequested,
        "work_order",
        &work_order_id.to_string(),
        &events::build_fg_receipt_requested_envelope(
            work_order_id,
            req.tenant_id.clone(),
            order_number,
            req.item_id,
            req.warehouse_id,
            req.quantity,
            req.currency.clone(),
            correlation_id.clone(),
            req.causation_id.clone(),
        ),
        &correlation_id,
        req.causation_id.as_deref(),
    )
    .await?;

    // Store idempotency key
    if let Some(key) = &req.idempotency_key {
        let resp = serde_json::json!({ "status": "accepted", "work_order_id": work_order_id });
        store_idempotency_key(
            &mut tx,
            &req.tenant_id,
            key,
            &request_hash,
            &resp.to_string(),
            202,
            Utc::now() + Duration::hours(24),
        )
        .await?;
    }

    // Audit: record FG receipt request inside the same transaction
    let audit_req = WriteAuditRequest::new(
        Uuid::nil(),
        "system".to_string(),
        "RequestFgReceipt".to_string(),
        MutationClass::Create,
        "WorkOrder".to_string(),
        work_order_id.to_string(),
    );
    AuditWriter::write_in_tx(&mut tx, audit_req)
        .await
        .map_err(|e| match e {
            platform_audit::writer::AuditWriterError::Database(db) => FgReceiptError::Database(db),
            platform_audit::writer::AuditWriterError::InvalidRequest(msg) => {
                FgReceiptError::Database(sqlx::Error::Protocol(msg))
            }
        })?;

    tx.commit().await?;
    Ok(false)
}
