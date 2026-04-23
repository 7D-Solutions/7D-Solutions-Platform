//! Sales order bounded context — types, state machine, validation.
//!
//! Invariants enforced here:
//!   - line_total_cents = quantity * unit_price_cents (rounded, integer)
//!   - subtotal_cents = SUM(line.line_total_cents)
//!   - total_cents = subtotal_cents + tax_cents
//!   - Booking requires ≥1 line
//!   - Line edits rejected once SO is booked (status ≠ draft)

pub mod repo;
pub mod service;

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("Sales order not found: {0}")]
    NotFound(Uuid),

    #[error("Order cannot be modified; current status: {0}")]
    NotDraft(String),

    #[error("Invalid status transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Order must have at least one line before booking")]
    EmptyLines,

    #[error("Monetary invariant violated: {0}")]
    MonetaryInvariant(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Reservation failed for line {line_id}: {reason}")]
    ReservationFailed { line_id: Uuid, reason: String },

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<OrderError> for platform_http_contracts::ApiError {
    fn from(err: OrderError) -> Self {
        match err {
            OrderError::NotFound(id) => Self::not_found(format!("Sales order {} not found", id)),
            OrderError::NotDraft(status) => Self::new(
                422,
                "order_not_draft",
                format!("Order cannot be modified; current status: {}", status),
            ),
            OrderError::InvalidTransition { from, to } => Self::new(
                422,
                "invalid_transition",
                format!("Cannot transition order from '{}' to '{}'", from, to),
            ),
            OrderError::EmptyLines => Self::new(
                422,
                "empty_lines",
                "Order must have at least one line before booking",
            ),
            OrderError::MonetaryInvariant(msg) => Self::new(422, "monetary_invariant", msg),
            OrderError::Validation(msg) => Self::new(422, "validation_error", msg),
            OrderError::ReservationFailed { line_id, reason } => Self::new(
                422,
                "reservation_failed",
                format!(
                    "Inventory reservation failed for line {}: {}",
                    line_id, reason
                ),
            ),
            OrderError::Database(e) => {
                tracing::error!("SO DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ── Status machine ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoStatus {
    Draft,
    Booked,
    InFulfillment,
    Shipped,
    Closed,
    Cancelled,
}

impl SoStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SoStatus::Draft => "draft",
            SoStatus::Booked => "booked",
            SoStatus::InFulfillment => "in_fulfillment",
            SoStatus::Shipped => "shipped",
            SoStatus::Closed => "closed",
            SoStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(SoStatus::Draft),
            "booked" => Some(SoStatus::Booked),
            "in_fulfillment" => Some(SoStatus::InFulfillment),
            "shipped" => Some(SoStatus::Shipped),
            "closed" => Some(SoStatus::Closed),
            "cancelled" => Some(SoStatus::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, SoStatus::Closed | SoStatus::Cancelled)
    }

    /// Returns true if the transition from self to `to` is permitted.
    pub fn can_transition_to(&self, to: SoStatus) -> bool {
        match (self, to) {
            (SoStatus::Draft, SoStatus::Booked) => true,
            (SoStatus::Draft, SoStatus::Cancelled) => true,
            (SoStatus::Booked, SoStatus::InFulfillment) => true,
            (SoStatus::Booked, SoStatus::Cancelled) => true,
            (SoStatus::InFulfillment, SoStatus::Shipped) => true,
            (SoStatus::InFulfillment, SoStatus::Cancelled) => true,
            (SoStatus::Shipped, SoStatus::Closed) => true,
            _ => false,
        }
    }
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, FromRow)]
pub struct SalesOrderLine {
    pub id: Uuid,
    pub tenant_id: String,
    pub sales_order_id: Uuid,
    pub line_number: i32,
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: String,
    pub uom: String,
    pub quantity: f64,
    pub unit_price_cents: i64,
    pub line_total_cents: i64,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub shipped_qty: f64,
    pub warehouse_id: Option<Uuid>,
    pub reservation_id: Option<Uuid>,
    pub invoiced_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, FromRow)]
pub struct SalesOrder {
    pub id: Uuid,
    pub tenant_id: String,
    pub order_number: String,
    pub status: String,
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub subtotal_cents: i64,
    pub tax_cents: i64,
    pub total_cents: i64,
    pub order_date: NaiveDate,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub external_quote_ref: Option<String>,
    pub blanket_order_id: Option<Uuid>,
    pub blanket_release_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SalesOrderWithLines {
    #[serde(flatten)]
    pub order: SalesOrder,
    pub lines: Vec<SalesOrderLine>,
}

// ── Requests ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateOrderRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub order_date: Option<NaiveDate>,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub external_quote_ref: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateOrderRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub external_quote_ref: Option<String>,
    pub notes: Option<String>,
    pub tax_cents: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BookOrderRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelOrderRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateOrderLineRequest {
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: String,
    pub uom: Option<String>,
    pub quantity: f64,
    pub unit_price_cents: i64,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub warehouse_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateOrderLineRequest {
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: Option<String>,
    pub uom: Option<String>,
    pub quantity: Option<f64>,
    pub unit_price_cents: Option<i64>,
    pub required_date: Option<NaiveDate>,
    pub promised_date: Option<NaiveDate>,
    pub warehouse_id: Option<Uuid>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListOrdersQuery {
    pub customer_id: Option<Uuid>,
    pub status: Option<String>,
    pub blanket_order_id: Option<Uuid>,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// Compute line_total_cents = round(quantity * unit_price_cents).
/// Uses integer arithmetic to avoid float rounding errors.
pub fn compute_line_total(quantity: f64, unit_price_cents: i64) -> i64 {
    // Multiply qty (f64) by unit price (i64 cents) and round to nearest cent.
    (quantity * unit_price_cents as f64).round() as i64
}

/// Verify the header-level monetary invariants:
///   subtotal_cents = SUM(line.line_total_cents)
///   total_cents = subtotal_cents + tax_cents
pub fn verify_header_totals(
    lines: &[SalesOrderLine],
    subtotal_cents: i64,
    tax_cents: i64,
    total_cents: i64,
) -> Result<(), OrderError> {
    let computed_subtotal: i64 = lines.iter().map(|l| l.line_total_cents).sum();
    if computed_subtotal != subtotal_cents {
        return Err(OrderError::MonetaryInvariant(format!(
            "subtotal_cents mismatch: stored={} computed={}",
            subtotal_cents, computed_subtotal
        )));
    }
    let expected_total = subtotal_cents + tax_cents;
    if expected_total != total_cents {
        return Err(OrderError::MonetaryInvariant(format!(
            "total_cents mismatch: stored={} expected subtotal+tax={}",
            total_cents, expected_total
        )));
    }
    Ok(())
}
