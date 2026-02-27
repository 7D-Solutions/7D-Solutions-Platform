use super::error::TilledError;
use super::types::ListResponse;
use super::TilledClient;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Report run response from Tilled API.
#[derive(Debug, Clone, Deserialize)]
pub struct ReportRun {
    pub id: String,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(rename = "type")]
    pub report_type: String,
    pub status: String,
    #[serde(default)]
    pub failure_message: Option<String>,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Parameters for creating a report run.
#[derive(Debug, Clone, Serialize)]
pub struct CreateReportRunParams {
    #[serde(rename = "type")]
    pub report_type: String,
    pub parameters: ReportRunParameters,
}

/// Date range parameters for a report run.
#[derive(Debug, Clone, Serialize)]
pub struct ReportRunParameters {
    pub start_at: String,
    pub end_at: String,
}

impl TilledClient {
    /// Create a report run. Requires partner scope.
    pub async fn create_report_run(
        &self,
        params: &CreateReportRunParams,
    ) -> Result<ReportRun, TilledError> {
        self.post("/v1/report-runs", params).await
    }

    /// List report runs with optional filters. Requires partner scope.
    pub async fn list_report_runs(
        &self,
        filters: Option<HashMap<String, String>>,
    ) -> Result<ListResponse<ReportRun>, TilledError> {
        self.get("/v1/report-runs", filters).await
    }

    /// Get a report run by ID. Requires partner scope.
    pub async fn get_report_run(&self, id: &str) -> Result<ReportRun, TilledError> {
        let path = format!("/v1/report-runs/{id}");
        self.get(&path, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_run_deserializes_full() {
        let value = serde_json::json!({
            "id": "frr_123",
            "account_id": "acct_456",
            "type": "payments_summary_1",
            "status": "finished",
            "parameters": {"start_at": "2026-01-01", "end_at": "2026-02-01"},
            "result": {"id": "file_789", "type": "csv"},
            "created_at": "2026-01-01T00:00:00Z"
        });
        let run: ReportRun = serde_json::from_value(value).unwrap();
        assert_eq!(run.id, "frr_123");
        assert_eq!(run.report_type, "payments_summary_1");
        assert_eq!(run.status, "finished");
        assert!(run.result.is_some());
    }

    #[test]
    fn report_run_deserializes_minimal() {
        let value = serde_json::json!({
            "id": "frr_min",
            "type": "fees_summary_1",
            "status": "queued"
        });
        let run: ReportRun = serde_json::from_value(value).unwrap();
        assert_eq!(run.id, "frr_min");
        assert!(run.result.is_none());
        assert!(run.failure_message.is_none());
    }

    #[test]
    fn create_params_serializes_correctly() {
        let params = CreateReportRunParams {
            report_type: "payments_summary_1".to_string(),
            parameters: ReportRunParameters {
                start_at: "2026-01-01T00:00:00.000Z".to_string(),
                end_at: "2026-02-01T00:00:00.000Z".to_string(),
            },
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["type"], "payments_summary_1");
        assert_eq!(json["parameters"]["start_at"], "2026-01-01T00:00:00.000Z");
    }
}
