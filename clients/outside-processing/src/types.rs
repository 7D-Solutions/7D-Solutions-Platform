use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpOrderDetail {
    #[serde(flatten)]
    pub order: OpOrder,
    pub ship_events: Vec<OpShipEvent>,
    pub return_events: Vec<OpReturnEvent>,
    pub reviews: Vec<OpVendorReview>,
    pub re_identifications: Vec<OpReIdentification>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueOpOrderRequest {
    pub purchase_order_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelOpOrderRequest {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateReturnEventRequest {
    pub received_date: NaiveDate,
    pub quantity_received: i32,
    pub unit_of_measure: Option<String>,
    pub condition: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateReviewRequest {
    pub return_event_id: Uuid,
    pub outcome: String,
    pub conditions: Option<String>,
    pub rejection_reason: Option<String>,
    pub rework: Option<bool>,
    pub reviewed_by: String,
    pub reviewed_at: DateTime<Utc>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpStatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub canonical_status: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertStatusLabelRequest {
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpServiceTypeLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub service_type: String,
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertServiceTypeLabelRequest {
    pub display_label: String,
    pub description: Option<String>,
    pub updated_by: String,
}
