use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// A dashboard layout configuration owned by a tenant.
#[derive(Debug, Clone, FromRow)]
pub struct DashboardLayout {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub version: i32,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A widget within a dashboard layout.
#[derive(Debug, Clone, FromRow)]
pub struct DashboardWidget {
    pub id: Uuid,
    pub layout_id: Uuid,
    pub tenant_id: String,
    pub widget_type: String,
    pub title: String,
    pub report_query: String,
    pub position_x: i32,
    pub position_y: i32,
    pub width: i32,
    pub height: i32,
    pub display_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a new widget in a layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetInput {
    pub widget_type: String,
    pub title: String,
    pub report_query: String,
    pub position_x: i32,
    pub position_y: i32,
    pub width: i32,
    pub height: i32,
    pub display_config: serde_json::Value,
}

/// Payload emitted in the outbox event after dashboard layout creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardLayoutCreatedPayload {
    pub layout_id: Uuid,
    pub name: String,
    pub widget_count: i32,
}

/// Payload emitted in the outbox event after dashboard layout update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardLayoutUpdatedPayload {
    pub layout_id: Uuid,
    pub name: String,
    pub version: i32,
}
