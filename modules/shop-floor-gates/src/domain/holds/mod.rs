pub mod repo;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
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

#[derive(Debug, Deserialize)]
pub struct PlaceHoldRequest {
    pub hold_type: String,
    pub scope: String,
    pub work_order_id: Uuid,
    pub operation_id: Option<Uuid>,
    pub reason: String,
    pub release_authority: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReleaseHoldRequest {
    pub release_notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CancelHoldRequest {
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListHoldsQuery {
    pub status: Option<String>,
    pub hold_type: Option<String>,
    pub work_order_id: Option<Uuid>,
    pub operation_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
