use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Event type constants
pub const HOLD_PLACED: &str = "shop_floor_gates.hold_placed";
pub const HOLD_RELEASED: &str = "shop_floor_gates.hold_released";
pub const HOLD_CANCELLED: &str = "shop_floor_gates.hold_cancelled";
pub const HANDOFF_INITIATED: &str = "shop_floor_gates.handoff_initiated";
pub const HANDOFF_ACCEPTED: &str = "shop_floor_gates.handoff_accepted";
pub const HANDOFF_REJECTED: &str = "shop_floor_gates.handoff_rejected";
pub const HANDOFF_CANCELLED: &str = "shop_floor_gates.handoff_cancelled";
pub const VERIFICATION_OPERATOR_CONFIRMED: &str =
    "shop_floor_gates.verification_operator_confirmed";
pub const VERIFICATION_COMPLETED: &str = "shop_floor_gates.verification_completed";
pub const SIGNOFF_RECORDED: &str = "shop_floor_gates.signoff_recorded";

#[derive(Debug, Serialize, Deserialize)]
pub struct HoldPlacedPayload {
    pub tenant_id: String,
    pub hold_id: Uuid,
    pub hold_number: String,
    pub hold_type: String,
    pub scope: String,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub placed_by: Uuid,
    pub placed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HoldReleasedPayload {
    pub tenant_id: String,
    pub hold_id: Uuid,
    pub hold_number: String,
    pub work_order_id: Uuid,
    pub released_by: Uuid,
    pub released_at: DateTime<Utc>,
    pub release_notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HoldCancelledPayload {
    pub tenant_id: String,
    pub hold_id: Uuid,
    pub hold_number: String,
    pub work_order_id: Uuid,
    pub cancelled_by: Uuid,
    pub cancelled_at: DateTime<Utc>,
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandoffInitiatedPayload {
    pub tenant_id: String,
    pub handoff_id: Uuid,
    pub handoff_number: String,
    pub work_order_id: Uuid,
    pub source_operation_id: Uuid,
    pub dest_operation_id: Uuid,
    pub initiation_type: String,
    pub quantity: f64,
    pub unit_of_measure: String,
    pub initiated_by: Uuid,
    pub initiated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandoffAcceptedPayload {
    pub tenant_id: String,
    pub handoff_id: Uuid,
    pub handoff_number: String,
    pub work_order_id: Uuid,
    pub accepted_by: Uuid,
    pub accepted_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandoffRejectedPayload {
    pub tenant_id: String,
    pub handoff_id: Uuid,
    pub handoff_number: String,
    pub work_order_id: Uuid,
    pub rejected_by: Uuid,
    pub rejected_at: DateTime<Utc>,
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HandoffCancelledPayload {
    pub tenant_id: String,
    pub handoff_id: Uuid,
    pub handoff_number: String,
    pub work_order_id: Uuid,
    pub cancelled_by: Uuid,
    pub cancelled_at: DateTime<Utc>,
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationOperatorConfirmedPayload {
    pub tenant_id: String,
    pub verification_id: Uuid,
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
    pub operator_id: Uuid,
    pub confirmed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationCompletedPayload {
    pub tenant_id: String,
    pub verification_id: Uuid,
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
    pub verifier_id: Uuid,
    pub verified_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SignoffRecordedPayload {
    pub tenant_id: String,
    pub signoff_id: Uuid,
    pub signoff_number: String,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub role: String,
    pub signed_by: Uuid,
    pub signed_at: DateTime<Utc>,
}
