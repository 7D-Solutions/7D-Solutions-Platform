use chrono::NaiveDateTime;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use super::PlatformHeaders;
use crate::errors::PlatformClientError;

const DEFAULT_BASE_URL: &str = "http://7d-ar:8086";

/// HTTP client for the AR (Accounts Receivable) service.
#[derive(Debug, Clone)]
pub struct ArClient {
    base_url: String,
    http: Client,
}

impl ArClient {
    pub fn new(http: Client) -> Self {
        let base_url = std::env::var("AR_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        Self { base_url, http }
    }

    pub fn with_base_url(http: Client, base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            http,
        }
    }

    pub async fn create_customer(
        &self,
        headers: &PlatformHeaders,
        req: &CreateCustomerRequest,
    ) -> Result<ArCustomer, PlatformClientError> {
        let url = format!("{}/api/ar/customers", self.base_url);
        tracing::debug!(correlation_id = %headers.correlation_id, url = %url, "ar.create_customer");

        let mut builder = self.http.post(&url).json(req);
        builder = apply_headers(builder, headers);

        let resp = builder.send().await?;
        parse_response(resp).await
    }

    pub async fn create_invoice(
        &self,
        headers: &PlatformHeaders,
        req: &CreateInvoiceRequest,
    ) -> Result<ArInvoice, PlatformClientError> {
        let url = format!("{}/api/ar/invoices", self.base_url);
        tracing::debug!(correlation_id = %headers.correlation_id, url = %url, "ar.create_invoice");

        let mut builder = self.http.post(&url).json(req);
        builder = apply_headers(builder, headers);

        let resp = builder.send().await?;
        parse_response(resp).await
    }

    pub async fn get_customer_by_external_id(
        &self,
        headers: &PlatformHeaders,
        external_id: &str,
    ) -> Result<Option<ArCustomer>, PlatformClientError> {
        let url = format!(
            "{}/api/ar/customers?external_customer_id={}",
            self.base_url,
            urlencoding(external_id),
        );
        tracing::debug!(
            correlation_id = %headers.correlation_id,
            url = %url,
            "ar.get_customer_by_external_id"
        );

        let mut builder = self.http.get(&url);
        builder = apply_headers(builder, headers);

        let resp = builder.send().await?;
        let customers: Vec<ArCustomer> = parse_response(resp).await?;
        Ok(customers.into_iter().next())
    }
}

/// Minimal URL-encoding for query parameter values.
fn urlencoding(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('+', "%2B")
        .replace('#', "%23")
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
pub struct CreateCustomerRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub external_customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInvoiceRequest {
    pub ar_customer_id: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub amount_cents: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due_at: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_period_start: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub billing_period_end: Option<NaiveDateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_item_details: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compliance_codes: Option<JsonValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub party_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArCustomer {
    pub id: i32,
    pub app_id: String,
    pub external_customer_id: Option<String>,
    pub tilled_customer_id: Option<String>,
    pub status: String,
    pub email: String,
    pub name: Option<String>,
    pub default_payment_method_id: Option<String>,
    pub payment_method_type: Option<String>,
    pub metadata: Option<JsonValue>,
    pub update_source: Option<String>,
    pub updated_by: Option<String>,
    pub delinquent_since: Option<NaiveDateTime>,
    pub grace_period_end: Option<NaiveDateTime>,
    pub next_retry_at: Option<NaiveDateTime>,
    pub retry_attempt_count: i32,
    pub party_id: Option<Uuid>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArInvoice {
    pub id: i32,
    pub app_id: String,
    pub tilled_invoice_id: String,
    pub ar_customer_id: i32,
    pub subscription_id: Option<i32>,
    pub status: String,
    pub amount_cents: i32,
    pub currency: String,
    pub due_at: Option<NaiveDateTime>,
    pub paid_at: Option<NaiveDateTime>,
    pub hosted_url: Option<String>,
    pub metadata: Option<JsonValue>,
    pub billing_period_start: Option<NaiveDateTime>,
    pub billing_period_end: Option<NaiveDateTime>,
    pub line_item_details: Option<JsonValue>,
    pub compliance_codes: Option<JsonValue>,
    pub correlation_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    pub party_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_customer_request_serializes() {
        let req = CreateCustomerRequest {
            email: Some("test@example.com".into()),
            name: Some("Test Customer".into()),
            external_customer_id: Some("ext-001".into()),
            metadata: None,
            party_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["email"], "test@example.com");
        assert_eq!(json["name"], "Test Customer");
        assert!(json.get("metadata").is_none());
    }

    #[test]
    fn create_invoice_request_serializes() {
        let req = CreateInvoiceRequest {
            ar_customer_id: 42,
            subscription_id: None,
            status: None,
            amount_cents: 10000,
            currency: Some("USD".into()),
            due_at: None,
            metadata: None,
            billing_period_start: None,
            billing_period_end: None,
            line_item_details: None,
            compliance_codes: None,
            correlation_id: None,
            party_id: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["ar_customer_id"], 42);
        assert_eq!(json["amount_cents"], 10000);
    }

    #[test]
    fn ar_customer_deserializes() {
        let json = serde_json::json!({
            "id": 1,
            "app_id": "test-app",
            "external_customer_id": "ext-001",
            "tilled_customer_id": null,
            "status": "active",
            "email": "test@example.com",
            "name": "Test Customer",
            "default_payment_method_id": null,
            "payment_method_type": null,
            "metadata": null,
            "update_source": null,
            "updated_by": null,
            "delinquent_since": null,
            "grace_period_end": null,
            "next_retry_at": null,
            "retry_attempt_count": 0,
            "party_id": null,
            "created_at": "2026-01-01T00:00:00",
            "updated_at": "2026-01-01T00:00:00"
        });
        let customer: ArCustomer = serde_json::from_value(json).unwrap();
        assert_eq!(customer.id, 1);
        assert_eq!(customer.email, "test@example.com");
    }

    #[test]
    fn ar_client_constructs() {
        let http = Client::new();
        let client = ArClient::with_base_url(http, "http://localhost:8086");
        assert_eq!(client.base_url, "http://localhost:8086");
    }

    #[test]
    fn urlencoding_handles_special_chars() {
        assert_eq!(urlencoding("hello world"), "hello%20world");
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
    }
}
