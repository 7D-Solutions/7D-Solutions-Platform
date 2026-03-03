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
    pub attempted_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub failed_at: Option<DateTime<Utc>>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub abandoned_at: Option<DateTime<Utc>>,
    pub replay_generation: i32,
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
