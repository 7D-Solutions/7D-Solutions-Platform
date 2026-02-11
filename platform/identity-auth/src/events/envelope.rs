use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub event_id: Uuid,
    pub event_type: String,
    pub schema_version: String,
    pub occurred_at: DateTime<Utc>,
    pub producer: String,
    pub tenant_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub trace_id: String,
    pub causation_id: Option<Uuid>,
    pub data: T,
}
