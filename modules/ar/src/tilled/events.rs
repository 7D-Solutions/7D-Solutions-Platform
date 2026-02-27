use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
}

impl TilledClient {
    /// List events with optional query filters.
    pub async fn list_events(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<Event>, TilledError> {
        self.get("/v1/events", filters).await
    }

    /// Fetch a single event by ID.
    pub async fn get_event(&self, event_id: &str) -> Result<Event, TilledError> {
        let path = format!("/v1/events/{event_id}");
        self.get(&path, None).await
    }
}
