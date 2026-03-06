use serde::Deserialize;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::outbox::enqueue_event;
use crate::events::{self, ProductionEventType};

// ============================================================================
// Request type
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RequestFgReceiptRequest {
    pub tenant_id: String,
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
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

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

pub async fn request_fg_receipt(
    pool: &PgPool,
    work_order_id: Uuid,
    req: &RequestFgReceiptRequest,
) -> Result<(), FgReceiptError> {
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

    let mut tx = pool.begin().await?;

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

    tx.commit().await?;
    Ok(())
}
