use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum OpError {
    #[error("OP order not found: {0}")]
    NotFound(Uuid),

    #[error("Invalid state transition from '{from}' to '{to}'")]
    InvalidTransition { from: String, to: String },

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

impl From<OpError> for platform_http_contracts::ApiError {
    fn from(err: OpError) -> Self {
        match err {
            OpError::NotFound(id) => Self::not_found(format!("OP order {} not found", id)),
            OpError::InvalidTransition { from, to } => Self::new(
                422,
                "invalid_transition",
                format!("Cannot transition from '{}' to '{}'", from, to),
            ),
            OpError::Validation(msg) => Self::new(422, "validation_error", msg),
            OpError::Database(e) => {
                tracing::error!("OP database error: {}", e);
                Self::internal("Internal database error")
            }
        }
    }
}

// ── Status ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpOrderStatus {
    Draft,
    Issued,
    ShippedToVendor,
    AtVendor,
    Returned,
    ReviewInProgress,
    Closed,
    Cancelled,
}

impl OpOrderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Issued => "issued",
            Self::ShippedToVendor => "shipped_to_vendor",
            Self::AtVendor => "at_vendor",
            Self::Returned => "returned",
            Self::ReviewInProgress => "review_in_progress",
            Self::Closed => "closed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "issued" => Some(Self::Issued),
            "shipped_to_vendor" => Some(Self::ShippedToVendor),
            "at_vendor" => Some(Self::AtVendor),
            "returned" => Some(Self::Returned),
            "review_in_progress" => Some(Self::ReviewInProgress),
            "closed" => Some(Self::Closed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Closed | Self::Cancelled)
    }
}

// ── Return Condition ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReturnCondition {
    Good,
    Damaged,
    Discrepancy,
}

impl ReturnCondition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Damaged => "damaged",
            Self::Discrepancy => "discrepancy",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "good" => Some(Self::Good),
            "damaged" => Some(Self::Damaged),
            "discrepancy" => Some(Self::Discrepancy),
            _ => None,
        }
    }
}

// ── Review Outcome ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    Accepted,
    Rejected,
    Conditional,
}

impl ReviewOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Conditional => "conditional",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            "conditional" => Some(Self::Conditional),
            _ => None,
        }
    }
}

// ── OP Order ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpOrder {
    pub op_order_id: Uuid,
    pub tenant_id: String,
    pub op_order_number: String,
    pub status: String,
    pub vendor_id: Option<Uuid>,
    pub service_type: Option<String>,
    pub service_description: Option<String>,
    pub process_spec_ref: Option<String>,
    pub part_number: Option<String>,
    pub part_revision: Option<String>,
    pub quantity_sent: i32,
    pub unit_of_measure: String,
    pub work_order_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub purchase_order_id: Option<Uuid>,
    pub lot_id: Option<Uuid>,
    pub serial_numbers: Vec<String>,
    pub expected_ship_date: Option<NaiveDate>,
    pub expected_return_date: Option<NaiveDate>,
    pub estimated_cost_cents: Option<i64>,
    pub actual_cost_cents: Option<i64>,
    pub notes: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OpOrderDetail {
    #[serde(flatten)]
    pub order: OpOrder,
    pub ship_events: Vec<OpShipEvent>,
    pub return_events: Vec<OpReturnEvent>,
    pub reviews: Vec<OpVendorReview>,
    pub re_identifications: Vec<OpReIdentification>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateOpOrderRequest {
    pub vendor_id: Option<Uuid>,
    pub service_type: Option<String>,
    pub service_description: Option<String>,
    pub process_spec_ref: Option<String>,
    pub part_number: Option<String>,
    pub part_revision: Option<String>,
    pub quantity_sent: i32,
    pub unit_of_measure: Option<String>,
    pub work_order_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub lot_id: Option<Uuid>,
    pub serial_numbers: Option<Vec<String>>,
    pub expected_ship_date: Option<NaiveDate>,
    pub expected_return_date: Option<NaiveDate>,
    pub estimated_cost_cents: Option<i64>,
    pub notes: Option<String>,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateOpOrderRequest {
    pub vendor_id: Option<Uuid>,
    pub service_type: Option<String>,
    pub service_description: Option<String>,
    pub process_spec_ref: Option<String>,
    pub part_number: Option<String>,
    pub part_revision: Option<String>,
    pub quantity_sent: Option<i32>,
    pub unit_of_measure: Option<String>,
    pub work_order_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub lot_id: Option<Uuid>,
    pub serial_numbers: Option<Vec<String>>,
    pub expected_ship_date: Option<NaiveDate>,
    pub expected_return_date: Option<NaiveDate>,
    pub estimated_cost_cents: Option<i64>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IssueOpOrderRequest {
    pub purchase_order_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CancelOpOrderRequest {
    pub reason: Option<String>,
}

// ── Ship Event ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpShipEvent {
    pub id: Uuid,
    pub tenant_id: String,
    pub op_order_id: Uuid,
    pub ship_date: NaiveDate,
    pub quantity_shipped: i32,
    pub unit_of_measure: String,
    pub lot_number: Option<String>,
    pub serial_numbers: Vec<String>,
    pub carrier_name: Option<String>,
    pub tracking_number: Option<String>,
    pub packing_slip_number: Option<String>,
    pub shipped_by: String,
    pub shipping_reference: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateShipEventRequest {
    pub ship_date: NaiveDate,
    pub quantity_shipped: i32,
    pub unit_of_measure: Option<String>,
    pub lot_number: Option<String>,
    pub serial_numbers: Option<Vec<String>>,
    pub carrier_name: Option<String>,
    pub tracking_number: Option<String>,
    pub packing_slip_number: Option<String>,
    pub shipped_by: String,
    pub shipping_reference: Option<Uuid>,
    pub notes: Option<String>,
}

// ── Return Event ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpReturnEvent {
    pub id: Uuid,
    pub tenant_id: String,
    pub op_order_id: Uuid,
    pub received_date: NaiveDate,
    pub quantity_received: i32,
    pub unit_of_measure: String,
    pub condition: String,
    pub discrepancy_notes: Option<String>,
    pub lot_number: Option<String>,
    pub serial_numbers: Vec<String>,
    pub cert_ref: Option<String>,
    pub vendor_packing_slip: Option<String>,
    pub carrier_name: Option<String>,
    pub tracking_number: Option<String>,
    pub re_identification_required: bool,
    pub received_by: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateReturnEventRequest {
    pub received_date: NaiveDate,
    pub quantity_received: i32,
    pub unit_of_measure: Option<String>,
    pub condition: ReturnCondition,
    pub discrepancy_notes: Option<String>,
    pub lot_number: Option<String>,
    pub serial_numbers: Option<Vec<String>>,
    pub cert_ref: Option<String>,
    pub vendor_packing_slip: Option<String>,
    pub carrier_name: Option<String>,
    pub tracking_number: Option<String>,
    pub re_identification_required: Option<bool>,
    pub received_by: String,
    pub notes: Option<String>,
}

// ── Vendor Review ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpVendorReview {
    pub id: Uuid,
    pub tenant_id: String,
    pub op_order_id: Uuid,
    pub return_event_id: Uuid,
    pub outcome: String,
    pub conditions: Option<String>,
    pub rejection_reason: Option<String>,
    pub reviewed_by: String,
    pub reviewed_at: DateTime<Utc>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateReviewRequest {
    pub return_event_id: Uuid,
    pub outcome: ReviewOutcome,
    pub conditions: Option<String>,
    pub rejection_reason: Option<String>,
    /// When outcome=rejected AND rework=true, order returns to at_vendor
    pub rework: Option<bool>,
    pub reviewed_by: String,
    pub reviewed_at: DateTime<Utc>,
    pub notes: Option<String>,
}

// ── Re-Identification ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpReIdentification {
    pub id: Uuid,
    pub tenant_id: String,
    pub op_order_id: Uuid,
    pub return_event_id: Uuid,
    pub old_part_number: String,
    pub old_part_revision: Option<String>,
    pub new_part_number: String,
    pub new_part_revision: Option<String>,
    pub reason: String,
    pub performed_by: String,
    pub performed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateReIdentificationRequest {
    pub return_event_id: Uuid,
    pub old_part_number: String,
    pub old_part_revision: Option<String>,
    pub new_part_number: String,
    pub new_part_revision: Option<String>,
    pub reason: String,
    pub performed_by: String,
    pub performed_at: DateTime<Utc>,
}

// ── Label Types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpStatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_status: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertStatusLabelRequest {
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct OpServiceTypeLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub service_type: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpsertServiceTypeLabelRequest {
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
}

// ── List Query ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ListOpOrdersQuery {
    pub status: Option<String>,
    pub vendor_id: Option<Uuid>,
    pub work_order_id: Option<Uuid>,
    pub service_type: Option<String>,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
