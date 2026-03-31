//! GL inventory posting logic — pure business rules for constructing journal entries
//!
//! Source type branching:
//! - **purchase / sales_order** → COGS path: DR COGS / CR INVENTORY
//! - **production** → WIP path: DR WIP / CR INVENTORY (raw material consumed)
//! - **production receipt** → FG path: DR INVENTORY / CR WIP (finished goods at rolled-up cost)

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use crate::services::journal_service::{process_gl_posting_request, JournalError};
use sqlx::PgPool;

// ============================================================================
// Payload types (mirrors inventory::events::contracts)
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ConsumedLayer {
    pub layer_id: Uuid,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub extended_cost_minor: i64,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct SourceRef {
    pub source_module: String,
    pub source_type: String,
    pub source_id: String,
    pub source_line_id: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ItemIssuedPayload {
    pub issue_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub total_cost_minor: i64,
    pub currency: String,
    pub consumed_layers: Vec<ConsumedLayer>,
    pub source_ref: SourceRef,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ItemReceivedPayload {
    pub receipt_line_id: Uuid,
    pub tenant_id: String,
    pub item_id: Uuid,
    pub sku: String,
    pub warehouse_id: Uuid,
    pub quantity: i64,
    pub unit_cost_minor: i64,
    pub currency: String,
    pub source_type: String,
    pub purchase_order_id: Option<Uuid>,
    pub received_at: DateTime<Utc>,
}

// ============================================================================
// Known source_type values
// ============================================================================

pub const SOURCE_TYPE_PURCHASE: &str = "purchase";
pub const SOURCE_TYPE_SALES_ORDER: &str = "sales_order";
pub const SOURCE_TYPE_PRODUCTION: &str = "production";

// ============================================================================
// Posting functions (testable without NATS)
// ============================================================================

/// Process an item_issued event — COGS path (purchase/sales_order source_type).
///
/// Journal entry: DR COGS / CR INVENTORY
pub async fn process_inventory_cogs_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &ItemIssuedPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_cost_minor as f64 / 100.0;

    let posting = GlPostingRequestV1 {
        posting_date: payload.issued_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::InventoryIssue,
        source_doc_id: payload.issue_line_id.to_string(),
        description: format!(
            "COGS — issued {} units of {} ({})",
            payload.quantity, payload.sku, payload.source_ref.source_id
        ),
        lines: vec![
            JournalLine {
                account_ref: "COGS".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Cost of goods sold — {} units SKU {}",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
            JournalLine {
                account_ref: "INVENTORY".to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Inventory reduction — issued {} units SKU {}",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
        ],
    };

    let subject = format!("inventory.item_issued.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await
}

/// Process an item_issued event — WIP path (production source_type).
///
/// Journal entry: DR WIP / CR INVENTORY (raw material consumed for production)
pub async fn process_inventory_wip_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &ItemIssuedPayload,
) -> Result<Uuid, JournalError> {
    let amount = payload.total_cost_minor as f64 / 100.0;

    let posting = GlPostingRequestV1 {
        posting_date: payload.issued_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ProductionIssue,
        source_doc_id: payload.issue_line_id.to_string(),
        description: format!(
            "WIP — issued {} units of {} to production ({})",
            payload.quantity, payload.sku, payload.source_ref.source_id
        ),
        lines: vec![
            JournalLine {
                account_ref: "WIP".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Work-in-process — {} units SKU {} consumed",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
            JournalLine {
                account_ref: "INVENTORY".to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Inventory reduction — issued {} units SKU {} to production",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
        ],
    };

    let subject = format!("inventory.item_issued.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await
}

/// Process an item_received event for production receipts (FG at rolled-up cost).
///
/// Journal entry: DR INVENTORY / CR WIP (finished goods received)
pub async fn process_production_receipt_posting(
    pool: &PgPool,
    event_id: Uuid,
    tenant_id: &str,
    source_module: &str,
    payload: &ItemReceivedPayload,
) -> Result<Uuid, JournalError> {
    let total_cost_minor = payload.quantity * payload.unit_cost_minor;
    let amount = total_cost_minor as f64 / 100.0;

    let posting = GlPostingRequestV1 {
        posting_date: payload.received_at.format("%Y-%m-%d").to_string(),
        currency: payload.currency.to_uppercase(),
        source_doc_type: SourceDocType::ProductionReceipt,
        source_doc_id: payload.receipt_line_id.to_string(),
        description: format!(
            "FG receipt — {} units of {} at rolled-up cost",
            payload.quantity, payload.sku
        ),
        lines: vec![
            JournalLine {
                account_ref: "INVENTORY".to_string(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Finished goods received — {} units SKU {}",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
            JournalLine {
                account_ref: "WIP".to_string(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "WIP relieved — {} units SKU {} completed",
                    payload.quantity, payload.sku
                )),
                dimensions: None,
            },
        ],
    };

    let subject = format!("inventory.item_received.{}", event_id);

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        source_module,
        &subject,
        &posting,
        None,
    )
    .await
}
