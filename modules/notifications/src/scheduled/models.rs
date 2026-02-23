use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Full row from `scheduled_notifications`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ScheduledNotification {
    pub id: Uuid,
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: Value,
    pub deliver_at: DateTime<Utc>,
    pub status: String,
    pub retry_count: i32,
    pub last_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Shape used when inserting a new pending notification.
#[derive(Debug, Clone)]
pub struct InsertPending {
    pub recipient_ref: String,
    pub channel: String,
    pub template_key: String,
    pub payload_json: Value,
    pub deliver_at: DateTime<Utc>,
}
