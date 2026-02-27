use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Terminal reader response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct TerminalReader {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub serial_number: Option<String>,
    #[serde(default)]
    pub device_type: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub ip_address: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Connection status for a terminal reader.
#[derive(Debug, Clone, Deserialize)]
pub struct TerminalReaderConnectionStatus {
    #[serde(default)]
    pub connected: Option<bool>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub last_seen_at: Option<String>,
}

/// Request body for updating a terminal reader.
#[derive(Debug, Serialize)]
pub struct UpdateTerminalReaderRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl TilledClient {
    /// List terminal readers with optional filters.
    pub async fn list_terminal_readers(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<TerminalReader>, TilledError> {
        self.get("/v1/terminal-readers", filters).await
    }

    /// Get a terminal reader by ID.
    pub async fn get_terminal_reader(&self, reader_id: &str) -> Result<TerminalReader, TilledError> {
        let path = format!("/v1/terminal-readers/{reader_id}");
        self.get(&path, None).await
    }

    /// Get the connection status of a terminal reader.
    pub async fn get_terminal_reader_status(
        &self,
        reader_id: &str,
    ) -> Result<TerminalReaderConnectionStatus, TilledError> {
        let path = format!("/v1/terminal-readers/{reader_id}/status");
        self.get(&path, None).await
    }

    /// Update a terminal reader's label.
    pub async fn update_terminal_reader(
        &self,
        reader_id: &str,
        label: Option<String>,
    ) -> Result<TerminalReader, TilledError> {
        let path = format!("/v1/terminal-readers/{reader_id}");
        let request = UpdateTerminalReaderRequest { label };
        self.patch(&path, &request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_reader_deserializes_full() {
        let value = serde_json::json!({
            "id": "tr_123",
            "label": "Front Counter",
            "serial_number": "SN-12345",
            "device_type": "verifone_p400",
            "status": "online",
            "ip_address": "192.168.1.100",
            "account_id": "acct_456",
            "created_at": "2026-01-01T00:00:00Z"
        });
        let reader: TerminalReader = serde_json::from_value(value).unwrap();
        assert_eq!(reader.id, "tr_123");
        assert_eq!(reader.label.as_deref(), Some("Front Counter"));
        assert_eq!(reader.device_type.as_deref(), Some("verifone_p400"));
        assert_eq!(reader.status.as_deref(), Some("online"));
    }

    #[test]
    fn terminal_reader_deserializes_minimal() {
        let value = serde_json::json!({"id": "tr_min"});
        let reader: TerminalReader = serde_json::from_value(value).unwrap();
        assert_eq!(reader.id, "tr_min");
        assert!(reader.label.is_none());
        assert!(reader.status.is_none());
    }

    #[test]
    fn connection_status_deserializes() {
        let value = serde_json::json!({
            "connected": true,
            "status": "online",
            "last_seen_at": "2026-01-15T12:00:00Z"
        });
        let status: TerminalReaderConnectionStatus = serde_json::from_value(value).unwrap();
        assert_eq!(status.connected, Some(true));
        assert_eq!(status.status.as_deref(), Some("online"));
    }

    #[test]
    fn update_request_omits_none_label() {
        let req = UpdateTerminalReaderRequest { label: None };
        let value = serde_json::to_value(req).unwrap();
        assert!(value.get("label").is_none());
    }

    #[test]
    fn update_request_includes_label() {
        let req = UpdateTerminalReaderRequest {
            label: Some("Back Office".to_string()),
        };
        let value = serde_json::to_value(req).unwrap();
        assert_eq!(value.get("label").unwrap(), "Back Office");
    }
}
