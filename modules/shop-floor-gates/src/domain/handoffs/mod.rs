pub mod repo;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
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

#[derive(Debug, Deserialize)]
pub struct InitiateHandoffRequest {
    pub work_order_id: Uuid,
    pub source_operation_id: Uuid,
    pub dest_operation_id: Uuid,
    pub initiation_type: Option<String>,
    pub quantity: f64,
    pub unit_of_measure: String,
    pub lot_number: Option<String>,
    pub serial_numbers: Option<Vec<String>>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AcceptHandoffRequest {
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RejectHandoffRequest {
    pub rejection_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CancelHandoffRequest {
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListHandoffsQuery {
    pub status: Option<String>,
    pub work_order_id: Option<Uuid>,
    pub source_operation_id: Option<Uuid>,
    pub dest_operation_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
