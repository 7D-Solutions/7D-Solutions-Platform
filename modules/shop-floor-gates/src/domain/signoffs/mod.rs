pub mod repo;
pub mod service;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Allowed entity types for signoffs
pub const VALID_ENTITY_TYPES: &[&str] = &[
    "work_order",
    "operation",
    "operation_handoff",
    "operation_start_verification",
    "traveler_hold",
];

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
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

#[derive(Debug, Deserialize)]
pub struct RecordSignoffRequest {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub role: String,
    pub signature_text: String,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListSignoffsQuery {
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    pub role: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}
