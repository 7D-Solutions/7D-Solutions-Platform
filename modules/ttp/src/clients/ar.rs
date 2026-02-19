/// HTTP client for the AR module — creates and finalizes invoices.
///
/// TTP uses AR as its invoicing backend. Each billing run item maps to one
/// AR invoice per party, created with a stable idempotency key so reruns
/// are safe.
///
/// Endpoints used:
///   POST {base_url}/api/ar/customers          — ensure AR customer exists for party
///   POST {base_url}/api/ar/invoices            — create draft invoice
///   POST {base_url}/api/ar/invoices/{id}/finalize — move draft → open
///
/// AR uses integer IDs for customers and invoices internally.
/// The `external_customer_id` field is used to store the party_id (UUID),
/// allowing idempotent lookup-or-create for AR customers.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Error from the AR client.
#[derive(Debug, thiserror::Error)]
pub enum ArClientError {
    #[error("AR HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("AR returned unexpected status {status} for {operation}")]
    UnexpectedStatus { operation: String, status: u16, body: String },
}

// ---------------------------------------------------------------------------
// Request / response shapes (mirror AR model types)
// ---------------------------------------------------------------------------

/// Minimal fields needed to find-or-create an AR customer for a party.
#[derive(Serialize)]
struct CreateCustomerRequest {
    pub email: String,
    pub name: Option<String>,
    pub external_customer_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ArCustomer {
    pub id: i32,
    pub external_customer_id: Option<String>,
}

/// Request body for creating a draft AR invoice.
#[derive(Serialize)]
struct CreateInvoiceRequest {
    pub ar_customer_id: i32,
    /// Amount in the currency's minor unit (cents for USD).
    pub amount_cents: i64,
    pub currency: String,
    pub correlation_id: Option<String>,
    pub party_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ArInvoice {
    pub id: i32,
    pub status: String,
}

/// Request body for finalizing an invoice (draft → open).
#[derive(Serialize)]
struct FinalizeInvoiceRequest {}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client for AR invoice operations.
#[derive(Clone)]
pub struct ArClient {
    http: reqwest::Client,
    base_url: String,
}

impl ArClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("build reqwest client for AR");

        Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    /// Find an existing AR customer by external_customer_id (= party_id string),
    /// or create one if absent.
    ///
    /// Uses `GET /api/ar/customers?external_customer_id=<party_id>` to check,
    /// then `POST /api/ar/customers` to create.
    pub async fn find_or_create_customer(
        &self,
        party_id: Uuid,
        email: &str,
    ) -> Result<i32, ArClientError> {
        // Search by external_customer_id = party_id
        let search_url = format!(
            "{}/api/ar/customers?external_customer_id={}",
            self.base_url, party_id
        );
        let resp = self.http.get(&search_url).send().await?;

        if resp.status().as_u16() == 200 {
            let customers: Vec<ArCustomer> = resp.json().await?;
            if let Some(customer) = customers.into_iter().find(|c| {
                c.external_customer_id.as_deref() == Some(&party_id.to_string())
            }) {
                return Ok(customer.id);
            }
        }

        // Create new AR customer
        let create_url = format!("{}/api/ar/customers", self.base_url);
        let body = CreateCustomerRequest {
            email: email.to_string(),
            name: Some(format!("Party {}", party_id)),
            external_customer_id: Some(party_id.to_string()),
        };

        let resp = self.http.post(&create_url).json(&body).send().await?;
        let status = resp.status().as_u16();
        if status != 201 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ArClientError::UnexpectedStatus {
                operation: "create_customer".to_string(),
                status,
                body: body_text,
            });
        }

        let customer: ArCustomer = resp.json().await?;
        Ok(customer.id)
    }

    /// Create a draft invoice in AR.
    ///
    /// `correlation_id` doubles as the idempotency key; if AR already has a
    /// draft invoice with this correlation_id it will be returned unchanged.
    pub async fn create_invoice(
        &self,
        ar_customer_id: i32,
        amount_minor: i64,
        currency: &str,
        correlation_id: &str,
        party_id: Uuid,
    ) -> Result<ArInvoice, ArClientError> {
        let url = format!("{}/api/ar/invoices", self.base_url);
        let body = CreateInvoiceRequest {
            ar_customer_id,
            amount_cents: amount_minor,
            currency: currency.to_string(),
            correlation_id: Some(correlation_id.to_string()),
            party_id: Some(party_id),
        };

        let resp = self
            .http
            .post(&url)
            .header("Idempotency-Key", correlation_id)
            .json(&body)
            .send()
            .await?;

        let status = resp.status().as_u16();
        if status != 201 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ArClientError::UnexpectedStatus {
                operation: "create_invoice".to_string(),
                status,
                body: body_text,
            });
        }

        let invoice: ArInvoice = resp.json().await?;
        Ok(invoice)
    }

    /// Finalize an existing draft AR invoice (draft → open).
    pub async fn finalize_invoice(&self, invoice_id: i32) -> Result<ArInvoice, ArClientError> {
        let url = format!("{}/api/ar/invoices/{}/finalize", self.base_url, invoice_id);
        let body = FinalizeInvoiceRequest {};

        let resp = self.http.post(&url).json(&body).send().await?;

        let status = resp.status().as_u16();
        if status != 200 {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(ArClientError::UnexpectedStatus {
                operation: format!("finalize_invoice({})", invoice_id),
                status,
                body: body_text,
            });
        }

        let invoice: ArInvoice = resp.json().await?;
        Ok(invoice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_strips_trailing_slash() {
        let client = ArClient::new("http://localhost:8086/");
        assert_eq!(client.base_url, "http://localhost:8086");
    }
}
