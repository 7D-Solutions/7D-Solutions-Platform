use super::error::TilledError;
use super::TilledClient;
use serde::Deserialize;

/// Health status response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct HealthStatus {
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
}

impl TilledClient {
    /// Check the Tilled API health status.
    pub async fn get_health(&self) -> Result<HealthStatus, TilledError> {
        self.get("/v1/health", None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_status_deserializes_full() {
        let value = serde_json::json!({
            "status": "ok",
            "version": "2026.1.0",
            "environment": "sandbox"
        });
        let health: HealthStatus = serde_json::from_value(value).unwrap();
        assert_eq!(health.status.as_deref(), Some("ok"));
        assert_eq!(health.environment.as_deref(), Some("sandbox"));
    }

    #[test]
    fn health_status_deserializes_minimal() {
        let value = serde_json::json!({});
        let health: HealthStatus = serde_json::from_value(value).unwrap();
        assert!(health.status.is_none());
    }
}
