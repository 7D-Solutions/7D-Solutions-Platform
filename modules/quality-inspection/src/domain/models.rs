use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ============================================================================
// DB row types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct InspectionPlan {
    pub id: Uuid,
    pub tenant_id: String,
    pub part_id: Uuid,
    pub plan_name: String,
    pub revision: String,
    pub status: String,
    pub characteristics: serde_json::Value,
    pub sampling_method: String,
    pub sample_size: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Inspection {
    pub id: Uuid,
    pub tenant_id: String,
    pub plan_id: Option<Uuid>,
    pub lot_id: Option<Uuid>,
    pub inspector_id: Option<Uuid>,
    pub inspection_type: String,
    pub result: String,
    pub notes: Option<String>,
    pub inspected_at: Option<DateTime<Utc>>,
    pub receipt_id: Option<Uuid>,
    pub part_id: Option<Uuid>,
    pub part_revision: Option<String>,
    pub disposition: String,
    pub wo_id: Option<Uuid>,
    pub op_instance_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Request types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Characteristic {
    pub name: String,
    pub characteristic_type: String,
    pub nominal: Option<f64>,
    pub tolerance_low: Option<f64>,
    pub tolerance_high: Option<f64>,
    pub uom: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInspectionPlanRequest {
    pub part_id: Uuid,
    pub plan_name: String,
    pub revision: Option<String>,
    pub characteristics: Vec<Characteristic>,
    pub sampling_method: Option<String>,
    pub sample_size: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct CreateReceivingInspectionRequest {
    pub plan_id: Option<Uuid>,
    pub receipt_id: Option<Uuid>,
    pub lot_id: Option<Uuid>,
    pub part_id: Option<Uuid>,
    pub part_revision: Option<String>,
    pub inspector_id: Option<Uuid>,
    pub result: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateInProcessInspectionRequest {
    pub wo_id: Uuid,
    pub op_instance_id: Uuid,
    pub plan_id: Option<Uuid>,
    pub lot_id: Option<Uuid>,
    pub part_id: Option<Uuid>,
    pub part_revision: Option<String>,
    pub inspector_id: Option<Uuid>,
    pub result: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFinalInspectionRequest {
    pub wo_id: Uuid,
    pub lot_id: Option<Uuid>,
    pub plan_id: Option<Uuid>,
    pub part_id: Option<Uuid>,
    pub part_revision: Option<String>,
    pub inspector_id: Option<Uuid>,
    pub result: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DispositionTransitionRequest {
    pub inspector_id: Option<Uuid>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InspectionsByPartRevQuery {
    pub part_id: Uuid,
    pub part_revision: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InspectionsByReceiptQuery {
    pub receipt_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct InspectionsByWoQuery {
    pub wo_id: Uuid,
    pub inspection_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct InspectionsByLotQuery {
    pub lot_id: Uuid,
}
