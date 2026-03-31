use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::idempotency::{check_idempotency, store_idempotency_key, IdempotencyError};
use crate::domain::outbox::enqueue_event;
use crate::events::{self, ComponentIssueItem, ProductionEventType};

// ============================================================================
// Request type
// ============================================================================

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct RequestComponentIssueRequest {
    pub tenant_id: String,
    pub items: Vec<ComponentIssueItemInput>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ComponentIssueItemInput {
    pub item_id: Uuid,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub currency: String,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum ComponentIssueError {
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
pub async fn request_component_issue(
    pool: &PgPool,
    work_order_id: Uuid,
    req: &RequestComponentIssueRequest,
) -> Result<bool, ComponentIssueError> {
    if req.tenant_id.trim().is_empty() {
        return Err(ComponentIssueError::Validation(
            "tenant_id is required".to_string(),
        ));
    }
    if req.items.is_empty() {
        return Err(ComponentIssueError::Validation(
            "items must not be empty".to_string(),
        ));
    }
    for (i, item) in req.items.iter().enumerate() {
        if item.quantity <= 0 {
            return Err(ComponentIssueError::Validation(format!(
                "items[{}].quantity must be > 0",
                i
            )));
        }
        if item.currency.trim().is_empty() {
            return Err(ComponentIssueError::Validation(format!(
                "items[{}].currency is required",
                i
            )));
        }
    }

    let request_hash = serde_json::to_string(req)
        .map_err(|e| ComponentIssueError::Database(sqlx::Error::Protocol(e.to_string())))?;

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
                return Err(ComponentIssueError::ConflictingIdempotencyKey);
            }
            Err(IdempotencyError::Database(e)) => return Err(ComponentIssueError::Database(e)),
            Err(IdempotencyError::Json(e)) => {
                return Err(ComponentIssueError::Database(sqlx::Error::Protocol(
                    e.to_string(),
                )));
            }
        }
    }

    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT status, order_number FROM work_orders WHERE work_order_id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(work_order_id)
    .bind(&req.tenant_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(ComponentIssueError::NotFound)?;

    if row.0 != "released" {
        return Err(ComponentIssueError::NotReleased);
    }

    let order_number = row.1;
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let items: Vec<ComponentIssueItem> = req
        .items
        .iter()
        .map(|i| ComponentIssueItem {
            item_id: i.item_id,
            warehouse_id: i.warehouse_id,
            quantity: i.quantity,
            currency: i.currency.clone(),
        })
        .collect();

    enqueue_event(
        &mut tx,
        &req.tenant_id,
        ProductionEventType::ComponentIssueRequested,
        "work_order",
        &work_order_id.to_string(),
        &events::build_component_issue_requested_envelope(
            work_order_id,
            req.tenant_id.clone(),
            order_number,
            items,
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

    tx.commit().await?;
    Ok(false)
}
