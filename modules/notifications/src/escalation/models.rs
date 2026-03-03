use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A tenant-scoped escalation rule.
///
/// When a notification of `source_notification_type` is not acknowledged within
/// `timeout_secs`, an escalation send is created to `target_recipient` on
/// `target_channel`.  Multi-level chains use incrementing `level` values.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EscalationRule {
    pub id: Uuid,
    pub tenant_id: String,
    pub source_notification_type: String,
    pub level: i32,
    pub timeout_secs: i32,
    pub target_channel: String,
    pub target_recipient: String,
    pub priority: String,
    pub created_at: DateTime<Utc>,
}

/// Shape used when inserting a new escalation rule.
#[derive(Debug, Clone)]
pub struct InsertEscalationRule {
    pub tenant_id: String,
    pub source_notification_type: String,
    pub level: i32,
    pub timeout_secs: i32,
    pub target_channel: String,
    pub target_recipient: String,
    pub priority: String,
}

/// Record of an escalation send that was created.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EscalationSend {
    pub id: Uuid,
    pub tenant_id: String,
    pub source_notification_id: Uuid,
    pub escalation_rule_id: Uuid,
    pub level: i32,
    pub target_channel: String,
    pub target_recipient: String,
    pub created_at: DateTime<Utc>,
}
