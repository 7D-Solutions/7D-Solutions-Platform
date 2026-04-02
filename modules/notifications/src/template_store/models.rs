use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Stored notification template row.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct NotificationTemplate {
    pub id: Uuid,
    pub tenant_id: String,
    pub template_key: String,
    pub version: i32,
    pub channel: String,
    pub subject: String,
    pub body: String,
    pub required_vars: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
}

/// Input for creating/publishing a new template version.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateTemplate {
    pub template_key: String,
    pub channel: String,
    pub subject: String,
    pub body: String,
    pub required_vars: Vec<String>,
}

/// Response for GET /templates/{key} — latest + version history.
#[derive(Debug, Serialize)]
pub struct TemplateDetail {
    pub latest: NotificationTemplate,
    pub versions: Vec<TemplateVersionSummary>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TemplateVersionSummary {
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
}
