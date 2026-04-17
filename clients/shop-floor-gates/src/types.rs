use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Holds ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TravelerHold {
    pub id: Uuid,
    pub tenant_id: String,
    pub hold_number: String,
    pub hold_type: String,
    pub scope: String,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub reason: String,
    pub status: String,
    pub release_authority: String,
    pub placed_by: Uuid,
    pub placed_at: DateTime<Utc>,
    pub released_by: Option<Uuid>,
    pub released_at: Option<DateTime<Utc>>,
    pub release_notes: Option<String>,
    pub cancelled_by: Option<Uuid>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaceHoldRequest {
    pub hold_type: String,
    pub scope: String,
    pub work_order_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Uuid>,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_authority: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReleaseHoldRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CancelHoldRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListHoldsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_order_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

// ── Handoffs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationHandoff {
    pub id: Uuid,
    pub tenant_id: String,
    pub handoff_number: String,
    pub work_order_id: Uuid,
    pub source_operation_id: Uuid,
    pub dest_operation_id: Uuid,
    pub initiation_type: String,
    pub status: String,
    pub quantity: f64,
    pub unit_of_measure: String,
    pub lot_number: Option<String>,
    pub serial_numbers: Option<Vec<String>>,
    pub notes: Option<String>,
    pub initiated_by: Uuid,
    pub initiated_at: DateTime<Utc>,
    pub accepted_by: Option<Uuid>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub rejected_by: Option<Uuid>,
    pub rejected_at: Option<DateTime<Utc>>,
    pub rejection_reason: Option<String>,
    pub cancelled_by: Option<Uuid>,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub cancel_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitiateHandoffRequest {
    pub work_order_id: Uuid,
    pub source_operation_id: Uuid,
    pub dest_operation_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiation_type: Option<String>,
    pub quantity: f64,
    pub unit_of_measure: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lot_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_numbers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AcceptHandoffRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RejectHandoffRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CancelHandoffRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListHandoffsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_order_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest_operation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

// ── Verifications ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationStartVerification {
    pub id: Uuid,
    pub tenant_id: String,
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
    pub status: String,
    pub drawing_verified: bool,
    pub material_verified: bool,
    pub instruction_verified: bool,
    pub operator_id: Uuid,
    pub operator_confirmed_at: Option<DateTime<Utc>>,
    pub verifier_id: Option<Uuid>,
    pub verified_at: Option<DateTime<Utc>>,
    pub skipped_by: Option<Uuid>,
    pub skipped_at: Option<DateTime<Utc>>,
    pub skip_reason: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateVerificationRequest {
    pub work_order_id: Uuid,
    pub operation_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorConfirmRequest {
    pub drawing_verified: bool,
    pub material_verified: bool,
    pub instruction_verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VerifyRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkipVerificationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListVerificationsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_order_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

// ── Signoffs ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signoff {
    pub id: Uuid,
    pub tenant_id: String,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub role: String,
    pub signoff_number: String,
    pub signed_by: Uuid,
    pub signed_at: DateTime<Utc>,
    pub signature_text: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSignoffRequest {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub role: String,
    pub signature_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListSignoffsQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<i64>,
}

// ── Labels ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusLabel {
    pub id: Uuid,
    pub tenant_id: String,
    pub status_key: String,
    pub display_name: String,
    pub color_hex: Option<String>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLabelRequest {
    pub status_key: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_hex: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<i32>,
}
