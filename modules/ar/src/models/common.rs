use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use utoipa::ToSchema;

/// Standard error response
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub error: String,
    pub message: String,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}

/// Idempotency key record from ar_idempotency_keys table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct IdempotencyKey {
    pub id: i32,
    pub app_id: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub response_body: JsonValue,
    pub status_code: i32,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

/// Event record from ar_events table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Event {
    pub id: i32,
    pub app_id: String,
    pub event_type: String,
    pub source: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<String>,
    pub payload: Option<JsonValue>,
    pub created_at: NaiveDateTime,
}

/// Query parameters for listing events
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListEventsQuery {
    pub entity_id: Option<String>,
    pub entity_type: Option<String>,
    pub event_type: Option<String>,
    pub source: Option<String>,
    pub start: Option<NaiveDateTime>,
    pub end: Option<NaiveDateTime>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}

/// Dispute record from ar_disputes table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Dispute {
    pub id: i32,
    pub app_id: String,
    pub tilled_dispute_id: String,
    pub tilled_charge_id: Option<String>,
    pub charge_id: Option<i32>,
    pub status: String,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
    pub reason: Option<String>,
    pub reason_code: Option<String>,
    pub evidence_due_by: Option<NaiveDateTime>,
    pub opened_at: Option<NaiveDateTime>,
    pub closed_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Request body for submitting dispute evidence
#[derive(Debug, Deserialize, ToSchema)]
pub struct SubmitDisputeEvidenceRequest {
    pub evidence: JsonValue,
}

/// Query parameters for listing disputes
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListDisputesQuery {
    pub charge_id: Option<i32>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
