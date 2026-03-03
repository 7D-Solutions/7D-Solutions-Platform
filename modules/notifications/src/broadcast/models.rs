use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Audience type for a broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudienceType {
    AllTenant,
    Role,
}

impl AudienceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            AudienceType::AllTenant => "all_tenant",
            AudienceType::Role => "role",
        }
    }
}

/// Full row from `broadcasts`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Broadcast {
    pub id: Uuid,
    pub tenant_id: String,
    pub idempotency_key: String,
    pub audience_type: String,
    pub audience_filter: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub channel: String,
    pub status: String,
    pub recipient_count: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full row from `broadcast_recipients`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BroadcastRecipient {
    pub id: Uuid,
    pub broadcast_id: Uuid,
    pub tenant_id: String,
    pub user_id: String,
    pub created_at: DateTime<Utc>,
}

/// Shape used when creating a new broadcast.
#[derive(Debug, Clone)]
pub struct CreateBroadcast {
    pub tenant_id: String,
    pub idempotency_key: String,
    pub audience_type: AudienceType,
    pub audience_filter: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub channel: String,
}

/// Result of a broadcast fan-out operation.
#[derive(Debug, Clone)]
pub struct BroadcastResult {
    pub broadcast: Broadcast,
    pub recipients_created: usize,
    pub was_duplicate: bool,
}
