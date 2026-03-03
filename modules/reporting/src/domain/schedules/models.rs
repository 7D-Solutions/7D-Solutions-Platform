use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleStatus {
    Active,
    Paused,
    Disabled,
}

impl ScheduleStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Disabled => "disabled",
        }
    }
}

impl std::fmt::Display for ScheduleStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryChannel {
    Email,
    Webhook,
    Sftp,
}

impl DeliveryChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::Webhook => "webhook",
            Self::Sftp => "sftp",
        }
    }
}

impl std::fmt::Display for DeliveryChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct DeliverySchedule {
    pub id: Uuid,
    pub tenant_id: String,
    pub report_id: String,
    pub schedule_name: String,
    pub cron_expr: Option<String>,
    pub interval_secs: Option<i32>,
    pub delivery_channel: String,
    pub recipient: String,
    pub format: String,
    pub status: String,
    pub idempotency_key: Option<String>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub next_trigger_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ScheduleExecution {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub tenant_id: String,
    pub export_run_id: Option<Uuid>,
    pub status: String,
    pub error_message: Option<String>,
    pub triggered_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Payload emitted in the outbox when a schedule is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleCreatedPayload {
    pub schedule_id: Uuid,
    pub report_id: String,
    pub delivery_channel: String,
    pub recipient: String,
    pub format: String,
}

/// Payload emitted in the outbox when a schedule trigger fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleTriggeredPayload {
    pub schedule_id: Uuid,
    pub execution_id: Uuid,
    pub export_run_id: Uuid,
    pub report_id: String,
    pub delivery_channel: String,
    pub recipient: String,
}
