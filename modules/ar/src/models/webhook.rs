use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// Webhook status enum (matches ar_webhooks_status in schema)
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "ar_webhooks_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum WebhookStatus {
    Received,
    Processing,
    Processed,
    Failed,
}

/// Webhook record from ar_webhooks table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: i32,
    pub app_id: String,
    pub event_id: String,
    pub event_type: String,
    pub status: WebhookStatus,
    pub error: Option<String>,
    pub payload: Option<JsonValue>,
    pub attempt_count: i32,
    pub last_attempt_at: Option<NaiveDateTime>,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub dead_at: Option<NaiveDateTime>,
    pub error_code: Option<String>,
    pub received_at: NaiveDateTime,
    pub processed_at: Option<NaiveDateTime>,
}

/// Webhook attempt record from ar_webhook_attempts table
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookAttempt {
    pub id: i32,
    pub app_id: String,
    pub event_id: String,
    pub attempt_number: i32,
    pub status: String,
    pub next_attempt_at: Option<NaiveDateTime>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

/// Incoming Tilled webhook event payload
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TilledWebhookEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: JsonValue,
    pub created_at: i64,
    pub livemode: Option<bool>,
}

/// Request to replay a failed webhook
#[derive(Debug, Deserialize)]
pub struct ReplayWebhookRequest {
    pub force: Option<bool>,
}

/// Query parameters for listing webhooks
#[derive(Debug, Deserialize)]
pub struct ListWebhooksQuery {
    pub event_type: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i32>,
    pub offset: Option<i32>,
}
