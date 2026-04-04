//! Billing rates and billing runs domain models.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Billing Rate
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct BillingRate {
    pub id: Uuid,
    pub app_id: String,
    pub name: String,
    pub rate_cents_per_hour: i32,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBillingRateRequest {
    pub app_id: String,
    pub name: String,
    pub rate_cents_per_hour: i32,
}

// ============================================================================
// Billing Run
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct BillingRun {
    pub id: Uuid,
    pub app_id: String,
    pub ar_customer_id: i32,
    pub from_date: NaiveDate,
    pub to_date: NaiveDate,
    pub amount_cents: i64,
    pub ar_invoice_id: Option<i32>,
    pub idempotency_key: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBillingRunRequest {
    pub app_id: String,
    pub ar_customer_id: i32,
    pub from_date: NaiveDate,
    pub to_date: NaiveDate,
}

/// A single billing line item computed from an entry + its rate.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BillingLineItem {
    pub entry_id: Uuid,
    pub minutes: i32,
    pub rate_cents_per_hour: i32,
    pub amount_cents: i64,
    pub description: Option<String>,
}

/// Result returned by `create_billing_run`.
///
/// When `already_ran = true`, the caller should use the existing `run`
/// without creating a duplicate AR invoice (idempotency).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BillingRunResult {
    pub run: BillingRun,
    pub line_items: Vec<BillingLineItem>,
    pub already_ran: bool,
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug, Error)]
pub enum BillingError {
    #[error("Validation error: {0}")]
    Validation(String),

    #[error("No billable entries found for the specified period")]
    NoBillableEntries,

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// Internal query row
// ============================================================================

/// Raw row from the billable entries query.
#[derive(Debug, sqlx::FromRow)]
pub(in crate::domain::billing) struct BillableEntryRow {
    pub entry_id: Uuid,
    pub minutes: i32,
    pub rate_cents_per_hour: i32,
    pub description: Option<String>,
}
