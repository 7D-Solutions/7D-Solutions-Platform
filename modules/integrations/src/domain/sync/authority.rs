use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub enum AuthoritySide {
    Platform,
    External,
}

impl AuthoritySide {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthoritySide::Platform => "platform",
            AuthoritySide::External => "external",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "platform" => Some(AuthoritySide::Platform),
            "external" => Some(AuthoritySide::External),
            _ => None,
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct AuthorityRow {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    pub authoritative_side: String,
    pub authority_version: i64,
    pub last_flipped_by: Option<String>,
    pub last_flipped_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
