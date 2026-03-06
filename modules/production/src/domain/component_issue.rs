use serde::Deserialize;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::outbox::enqueue_event;
use crate::events::{self, ComponentIssueItem, ProductionEventType};

// ============================================================================
// Request type
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct RequestComponentIssueRequest {
    pub tenant_id: String,
    pub items: Vec<ComponentIssueItemInput>,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
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

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Service
// ============================================================================

pub async fn request_component_issue(
    pool: &PgPool,
    work_order_id: Uuid,
    req: &RequestComponentIssueRequest,
) -> Result<(), ComponentIssueError> {
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

    let mut tx = pool.begin().await?;

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

    tx.commit().await?;
    Ok(())
}
