use super::error::TilledError;
use super::TilledClient;
use serde::{Deserialize, Serialize};

/// Merchant application returned by Tilled's `/v1/applications/{account_id}`.
/// The response structure mirrors OnboardingApplication but is accessed via
/// partner scope with an explicit account_id path parameter.
#[derive(Debug, Clone, Deserialize)]
pub struct MerchantApplication {
    #[serde(default)]
    pub legal_entity: Option<serde_json::Value>,
    #[serde(default)]
    pub validation_errors: Option<Vec<String>>,
    #[serde(default)]
    pub tos_acceptance: Option<bool>,
    #[serde(default)]
    pub pricing_templates: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub bank_verification: Option<serde_json::Value>,
    #[serde(default)]
    pub terms_and_conditions_links: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub canada_visa_mc_processing: Option<bool>,
}

/// Request body for updating a merchant application.
#[derive(Debug, Serialize)]
pub struct UpdateMerchantApplicationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_entity: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tos_acceptance: Option<bool>,
}

/// Response from submitting a merchant application.
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitMerchantApplicationResponse {
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl TilledClient {
    /// Get a merchant application by account ID.
    /// Operates on partner scope.
    pub async fn get_merchant_application(
        &self,
        account_id: &str,
    ) -> Result<MerchantApplication, TilledError> {
        let path = format!("/v1/applications/{account_id}");
        self.get(&path, None).await
    }

    /// Update a merchant application by account ID (PUT).
    /// Operates on partner scope.
    pub async fn update_merchant_application(
        &self,
        account_id: &str,
        request: &UpdateMerchantApplicationRequest,
    ) -> Result<MerchantApplication, TilledError> {
        let path = format!("/v1/applications/{account_id}");
        let url = format!("{}{}", self.config().base_path, path);
        let response = self
            .http_client
            .put(&url)
            .headers(self.build_auth_headers()?)
            .json(request)
            .send()
            .await
            .map_err(|e| TilledError::HttpError(e.to_string()))?;
        self.handle_response(response).await
    }

    /// Submit a merchant application by account ID.
    /// Operates on partner scope.
    pub async fn submit_merchant_application(
        &self,
        account_id: &str,
    ) -> Result<SubmitMerchantApplicationResponse, TilledError> {
        let path = format!("/v1/applications/{account_id}/submit");
        let empty: serde_json::Value = serde_json::json!({});
        self.post(&path, &empty).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merchant_application_deserializes_with_optional_fields() {
        let value = serde_json::json!({
            "legal_entity": {"legal_name": "Acme Inc"},
            "validation_errors": ["missing field"],
            "tos_acceptance": false,
            "updated_at": "2026-01-01T00:00:00Z"
        });
        let app: MerchantApplication = serde_json::from_value(value).unwrap();
        assert!(app.legal_entity.is_some());
        assert_eq!(app.tos_acceptance, Some(false));
    }

    #[test]
    fn merchant_application_deserializes_minimal() {
        let value = serde_json::json!({});
        let app: MerchantApplication = serde_json::from_value(value).unwrap();
        assert!(app.legal_entity.is_none());
    }

    #[test]
    fn update_request_omits_none_fields() {
        let req = UpdateMerchantApplicationRequest {
            legal_entity: Some(serde_json::json!({"legal_name": "New Name"})),
            tos_acceptance: None,
        };
        let value = serde_json::to_value(req).unwrap();
        assert!(value.get("legal_entity").is_some());
        assert!(value.get("tos_acceptance").is_none());
    }
}
