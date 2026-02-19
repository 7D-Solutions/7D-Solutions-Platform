use chrono::{DateTime, NaiveDate, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::PlatformHeaders;
use crate::errors::PlatformClientError;

const DEFAULT_BASE_URL: &str = "http://7d-party:8098";

/// HTTP client for the Party Master service.
#[derive(Debug, Clone)]
pub struct PartyClient {
    base_url: String,
    http: Client,
}

impl PartyClient {
    pub fn new(http: Client) -> Self {
        let base_url = std::env::var("PARTY_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self { base_url, http }
    }

    pub fn with_base_url(http: Client, base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http,
        }
    }

    pub async fn create_company(
        &self,
        headers: &PlatformHeaders,
        req: &CreateCompanyRequest,
    ) -> Result<PartyView, PlatformClientError> {
        let url = format!("{}/api/party/companies", self.base_url);
        tracing::debug!(correlation_id = %headers.correlation_id, url = %url, "party.create_company");

        let mut builder = self.http.post(&url).json(req);
        builder = apply_headers(builder, headers);

        let resp = builder.send().await?;
        parse_response(resp).await
    }

    pub async fn create_individual(
        &self,
        headers: &PlatformHeaders,
        req: &CreateIndividualRequest,
    ) -> Result<PartyView, PlatformClientError> {
        let url = format!("{}/api/party/individuals", self.base_url);
        tracing::debug!(correlation_id = %headers.correlation_id, url = %url, "party.create_individual");

        let mut builder = self.http.post(&url).json(req);
        builder = apply_headers(builder, headers);

        let resp = builder.send().await?;
        parse_response(resp).await
    }

    pub async fn get_party(
        &self,
        headers: &PlatformHeaders,
        party_id: Uuid,
    ) -> Result<PartyView, PlatformClientError> {
        let url = format!("{}/api/party/parties/{}", self.base_url, party_id);
        tracing::debug!(correlation_id = %headers.correlation_id, url = %url, "party.get_party");

        let mut builder = self.http.get(&url);
        builder = apply_headers(builder, headers);

        let resp = builder.send().await?;
        parse_response(resp).await
    }
}

fn apply_headers(
    builder: reqwest::RequestBuilder,
    headers: &PlatformHeaders,
) -> reqwest::RequestBuilder {
    let mut b = builder
        .header("x-app-id", &headers.app_id)
        .header("x-correlation-id", &headers.correlation_id)
        .header("x-actor-id", &headers.actor_id);
    if let Some(ref auth) = headers.authorization {
        b = b.header("Authorization", auth);
    }
    b
}

async fn parse_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, PlatformClientError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(PlatformClientError::api(status.as_u16(), body));
    }
    let bytes = resp.bytes().await?;
    serde_json::from_slice(&bytes).map_err(|e| {
        PlatformClientError::Deserialization(format!(
            "{}: {}",
            e,
            String::from_utf8_lossy(&bytes)
        ))
    })
}

// ============================================================================
// Request / Response types (client-side mirrors)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCompanyRequest {
    pub display_name: String,
    pub legal_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trade_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registration_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_of_incorporation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub industry_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub founded_date: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub employee_count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annual_revenue_cents: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_line1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_line2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIndividualRequest {
    pub display_name: String,
    pub first_name: String,
    pub last_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub middle_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_of_birth: Option<NaiveDate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nationality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub department: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_line1: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_line2: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Composite party view returned by the Party service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyView {
    pub id: Uuid,
    pub app_id: String,
    pub party_type: String,
    pub status: String,
    pub display_name: String,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub website: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub company: Option<PartyCompanyExt>,
    pub individual: Option<PartyIndividualExt>,
    pub external_refs: Vec<ExternalRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyCompanyExt {
    pub party_id: Uuid,
    pub legal_name: String,
    pub trade_name: Option<String>,
    pub registration_number: Option<String>,
    pub tax_id: Option<String>,
    pub country_of_incorporation: Option<String>,
    pub industry_code: Option<String>,
    pub founded_date: Option<NaiveDate>,
    pub employee_count: Option<i32>,
    pub annual_revenue_cents: Option<i64>,
    pub currency: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyIndividualExt {
    pub party_id: Uuid,
    pub first_name: String,
    pub last_name: String,
    pub middle_name: Option<String>,
    pub date_of_birth: Option<NaiveDate>,
    pub tax_id: Option<String>,
    pub nationality: Option<String>,
    pub job_title: Option<String>,
    pub department: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalRef {
    pub id: i64,
    pub party_id: Uuid,
    pub app_id: String,
    pub system: String,
    pub external_id: String,
    pub label: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_company_request_serializes() {
        let req = CreateCompanyRequest {
            display_name: "Acme Corp".into(),
            legal_name: "Acme Corporation Ltd".into(),
            trade_name: None,
            registration_number: None,
            tax_id: None,
            country_of_incorporation: None,
            industry_code: None,
            founded_date: None,
            employee_count: None,
            annual_revenue_cents: None,
            currency: None,
            email: Some("info@acme.com".into()),
            phone: None,
            website: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            metadata: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["display_name"], "Acme Corp");
        assert_eq!(json["legal_name"], "Acme Corporation Ltd");
        assert!(json.get("trade_name").is_none());
    }

    #[test]
    fn create_individual_request_serializes() {
        let req = CreateIndividualRequest {
            display_name: "Jane Doe".into(),
            first_name: "Jane".into(),
            last_name: "Doe".into(),
            middle_name: None,
            date_of_birth: None,
            tax_id: None,
            nationality: None,
            job_title: None,
            department: None,
            email: None,
            phone: None,
            address_line1: None,
            address_line2: None,
            city: None,
            state: None,
            postal_code: None,
            country: None,
            metadata: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["first_name"], "Jane");
        assert_eq!(json["last_name"], "Doe");
    }

    #[test]
    fn party_view_deserializes_from_json() {
        let json = serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "app_id": "test-app",
            "party_type": "company",
            "status": "active",
            "display_name": "Acme Corp",
            "email": null,
            "phone": null,
            "website": null,
            "address_line1": null,
            "address_line2": null,
            "city": null,
            "state": null,
            "postal_code": null,
            "country": null,
            "metadata": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "company": null,
            "individual": null,
            "external_refs": []
        });
        let view: PartyView = serde_json::from_value(json).unwrap();
        assert_eq!(view.display_name, "Acme Corp");
        assert_eq!(view.party_type, "company");
    }

    #[test]
    fn party_client_constructs() {
        let http = Client::new();
        let client = PartyClient::with_base_url(http, "http://localhost:8098");
        assert_eq!(client.base_url, "http://localhost:8098");
    }
}
