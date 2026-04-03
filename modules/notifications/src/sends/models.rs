use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// A notification send request record.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct NotificationSend {
    pub id: Uuid,
    pub tenant_id: String,
    pub template_key: Option<String>,
    pub template_version: Option<i32>,
    pub channel: String,
    pub recipients: serde_json::Value,
    pub payload_json: serde_json::Value,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub rendered_hash: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for POST /notifications/send.
///
/// Two modes:
/// 1. **Template-based**: provide `template_key` + `payload_json` — the platform
///    resolves the template and renders subject/body server-side.
/// 2. **Pre-rendered**: provide `rendered_subject` + `rendered_body` — the platform
///    sends the content as-is, skipping template resolution.
///
/// `template_key` is required in mode 1 but optional in mode 2.
/// Sending neither `template_key` nor pre-rendered content returns 400.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SendRequest {
    pub template_key: Option<String>,
    pub channel: String,
    pub recipients: Vec<String>,
    #[serde(default = "default_empty_object")]
    pub payload_json: serde_json::Value,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    /// Pre-rendered subject line. When provided with `rendered_body`, skips
    /// template resolution.
    pub rendered_subject: Option<String>,
    /// Pre-rendered HTML body. When provided with `rendered_subject`, skips
    /// template resolution.
    pub rendered_body: Option<String>,
}

fn default_empty_object() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// A delivery receipt row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct DeliveryReceipt {
    pub id: Uuid,
    pub tenant_id: String,
    pub send_id: Uuid,
    pub recipient: String,
    pub channel: String,
    pub status: String,
    pub provider_id: Option<String>,
    pub attempt_count: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub succeeded_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub error_class: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Query parameters for GET /deliveries.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeliveryQuery {
    pub correlation_id: Option<String>,
    pub recipient: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Response for GET /notifications/{id}.
#[derive(Debug, Serialize)]
pub struct SendDetail {
    pub send: NotificationSend,
    pub receipts: Vec<DeliveryReceipt>,
}
