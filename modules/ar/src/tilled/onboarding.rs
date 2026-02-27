use super::error::TilledError;
use super::TilledClient;
use serde::{Deserialize, Serialize};

/// Onboarding application returned by Tilled's `/v1/onboarding` endpoint.
/// Uses `serde_json::Value` for nested structures since the schema is large
/// and variable (legal_entity, pricing_templates, bank_verification, etc.).
#[derive(Debug, Clone, Deserialize)]
pub struct OnboardingApplication {
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

/// Request body for updating an onboarding application.
#[derive(Debug, Serialize)]
pub struct UpdateOnboardingRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_entity: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tos_acceptance: Option<bool>,
}

/// Response from submitting an onboarding application.
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitOnboardingResponse {
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl TilledClient {
    /// Get the onboarding application for the current account.
    /// Operates on merchant scope (tilled-account = merchant's own account).
    pub async fn get_onboarding(&self) -> Result<OnboardingApplication, TilledError> {
        self.get("/v1/onboarding", None).await
    }

    /// Update the onboarding application for the current account.
    pub async fn update_onboarding(
        &self,
        request: &UpdateOnboardingRequest,
    ) -> Result<OnboardingApplication, TilledError> {
        self.post("/v1/onboarding", request).await
    }

    /// Submit the onboarding application for the current account.
    pub async fn submit_onboarding(&self) -> Result<SubmitOnboardingResponse, TilledError> {
        let empty: serde_json::Value = serde_json::json!({});
        self.post("/v1/onboarding/submit", &empty).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_application_deserializes_with_optional_fields() {
        let value = serde_json::json!({
            "legal_entity": {"legal_name": "Test Corp"},
            "validation_errors": ["field required"],
            "tos_acceptance": true,
            "updated_at": "2026-01-01T00:00:00Z"
        });
        let app: OnboardingApplication = serde_json::from_value(value).unwrap();
        assert!(app.legal_entity.is_some());
        assert_eq!(app.validation_errors.as_ref().unwrap().len(), 1);
        assert_eq!(app.tos_acceptance, Some(true));
    }

    #[test]
    fn onboarding_application_deserializes_minimal() {
        let value = serde_json::json!({});
        let app: OnboardingApplication = serde_json::from_value(value).unwrap();
        assert!(app.legal_entity.is_none());
        assert!(app.validation_errors.is_none());
    }

    #[test]
    fn update_onboarding_request_omits_none_fields() {
        let req = UpdateOnboardingRequest {
            legal_entity: None,
            tos_acceptance: Some(true),
        };
        let value = serde_json::to_value(req).unwrap();
        assert!(value.get("legal_entity").is_none());
        assert_eq!(value.get("tos_acceptance").unwrap(), true);
    }
}
