use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Full row from `inbox_messages`.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct InboxMessage {
    pub id: Uuid,
    pub tenant_id: String,
    pub user_id: String,
    pub notification_id: Uuid,
    pub title: String,
    pub body: Option<String>,
    pub category: Option<String>,
    pub is_read: bool,
    pub is_dismissed: bool,
    pub read_at: Option<DateTime<Utc>>,
    pub dismissed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
