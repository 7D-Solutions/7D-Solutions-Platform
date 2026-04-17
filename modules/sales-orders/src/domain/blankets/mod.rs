//! Blanket order bounded context — types, state machine, validation.

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
pub enum BlanketError {
    #[error("Blanket order not found: {0}")]
    NotFound(Uuid),

    #[error("Blanket order cannot be modified; current status: {0}")]
    NotEditable(String),

    #[error("Invalid status transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("Release quantity {requested} exceeds remaining committed qty {remaining}")]
    OverDraw { requested: f64, remaining: f64 },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<BlanketError> for platform_http_contracts::ApiError {
    fn from(err: BlanketError) -> Self {
        match err {
            BlanketError::NotFound(id) => {
                Self::not_found(format!("Blanket order {} not found", id))
            }
            BlanketError::NotEditable(status) => Self::new(
                422,
                "blanket_not_editable",
                format!("Blanket order cannot be modified; status: {}", status),
            ),
            BlanketError::InvalidTransition { from, to } => Self::new(
                422,
                "invalid_transition",
                format!("Cannot transition blanket from '{}' to '{}'", from, to),
            ),
            BlanketError::OverDraw { requested, remaining } => Self::new(
                422,
                "over_draw",
                format!("Release qty {:.4} exceeds remaining {:.4}", requested, remaining),
            ),
            BlanketError::Validation(msg) => Self::new(422, "validation_error", msg),
            BlanketError::Database(e) => {
                tracing::error!("Blanket DB error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ── Status machines ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlanketStatus {
    Draft,
    Active,
    Expired,
    Cancelled,
    Closed,
}

impl BlanketStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlanketStatus::Draft => "draft",
            BlanketStatus::Active => "active",
            BlanketStatus::Expired => "expired",
            BlanketStatus::Cancelled => "cancelled",
            BlanketStatus::Closed => "closed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(BlanketStatus::Draft),
            "active" => Some(BlanketStatus::Active),
            "expired" => Some(BlanketStatus::Expired),
            "cancelled" => Some(BlanketStatus::Cancelled),
            "closed" => Some(BlanketStatus::Closed),
            _ => None,
        }
    }

    pub fn can_transition_to(&self, to: BlanketStatus) -> bool {
        match (self, to) {
            (BlanketStatus::Draft, BlanketStatus::Active) => true,
            (BlanketStatus::Draft, BlanketStatus::Cancelled) => true,
            (BlanketStatus::Active, BlanketStatus::Expired) => true,
            (BlanketStatus::Active, BlanketStatus::Cancelled) => true,
            (BlanketStatus::Active, BlanketStatus::Closed) => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseStatus {
    Pending,
    Released,
    Shipped,
    Cancelled,
}

impl ReleaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReleaseStatus::Pending => "pending",
            ReleaseStatus::Released => "released",
            ReleaseStatus::Shipped => "shipped",
            ReleaseStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(ReleaseStatus::Pending),
            "released" => Some(ReleaseStatus::Released),
            "shipped" => Some(ReleaseStatus::Shipped),
            "cancelled" => Some(ReleaseStatus::Cancelled),
            _ => None,
        }
    }
}

// ── Domain types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, FromRow)]
pub struct BlanketOrder {
    pub id: Uuid,
    pub tenant_id: String,
    pub blanket_number: String,
    pub status: String,
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub committed_cents: i64,
    pub released_cents: i64,
    pub effective_date: NaiveDate,
    pub expiry_date: Option<NaiveDate>,
    pub notes: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, FromRow)]
pub struct BlanketOrderLine {
    pub id: Uuid,
    pub tenant_id: String,
    pub blanket_order_id: Uuid,
    pub line_number: i32,
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: String,
    pub uom: String,
    pub committed_qty: f64,
    pub released_qty: f64,
    pub unit_price_cents: i64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, FromRow)]
pub struct BlanketOrderRelease {
    pub id: Uuid,
    pub tenant_id: String,
    pub blanket_order_id: Uuid,
    pub blanket_line_id: Uuid,
    pub sales_order_id: Option<Uuid>,
    pub status: String,
    pub release_qty: f64,
    pub release_date: NaiveDate,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BlanketOrderWithLines {
    #[serde(flatten)]
    pub order: BlanketOrder,
    pub lines: Vec<BlanketOrderLine>,
}

// ── Requests ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBlanketRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub currency: String,
    pub effective_date: Option<NaiveDate>,
    pub expiry_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateBlanketRequest {
    pub customer_id: Option<Uuid>,
    pub party_id: Option<Uuid>,
    pub expiry_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ActivateBlanketRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBlanketLineRequest {
    pub item_id: Option<Uuid>,
    pub part_number: Option<String>,
    pub description: String,
    pub uom: Option<String>,
    pub committed_qty: f64,
    pub unit_price_cents: i64,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateReleaseRequest {
    pub blanket_line_id: Uuid,
    pub release_qty: f64,
    pub release_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListBlanketsQuery {
    pub customer_id: Option<Uuid>,
    pub status: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}
