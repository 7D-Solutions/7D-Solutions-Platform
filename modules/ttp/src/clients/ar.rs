/// Typed AR client adapter.
///
/// Wraps `platform-client-ar` generated clients (CustomersClient, InvoicesClient)
/// and preserves the `find_or_create_customer` orchestration logic.
///
/// Endpoints used:
///   GET  {base_url}/api/ar/customers?external_customer_id=...
///   POST {base_url}/api/ar/customers
///   POST {base_url}/api/ar/invoices
///   POST {base_url}/api/ar/invoices/{id}/finalize
use platform_client_ar::{
    CreateCustomerRequest, CreateInvoiceRequest, Customer, CustomersClient,
    FinalizeInvoiceRequest, Invoice, InvoicesClient,
};
use platform_sdk::ClientError;
use uuid::Uuid;

/// Error from the AR client.
#[derive(Debug, thiserror::Error)]
pub enum ArClientError {
    #[error("AR client error: {0}")]
    Client(#[from] ClientError),

    #[error("AR HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}

/// Minimal invoice view used by billing domain.
#[derive(Debug)]
pub struct ArInvoice {
    pub id: i32,
    pub status: String,
}

impl From<Invoice> for ArInvoice {
    fn from(inv: Invoice) -> Self {
        Self {
            id: inv.id,
            status: inv.status,
        }
    }
}

/// HTTP client for AR invoice operations.
///
/// Uses generated typed clients from `platform-client-ar` for create/finalize.
/// The `find_or_create_customer` search step uses a raw HTTP call because the
/// generated `list_customers` endpoint does not return parsed response bodies.
pub struct ArClient {
    customers: CustomersClient,
    invoices: InvoicesClient,
    http: reqwest::Client,
    base_url: String,
}

impl ArClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("build reqwest client for AR");
        let base = base_url.into().trim_end_matches('/').to_string();
        Self {
            customers: CustomersClient::new(http.clone(), &base, ""),
            invoices: InvoicesClient::new(http.clone(), &base, ""),
            http,
            base_url: base,
        }
    }

    /// Find an existing AR customer by external_customer_id (= party_id string),
    /// or create one if absent.
    ///
    /// Uses `GET /api/ar/customers?external_customer_id=<party_id>` to check,
    /// then the generated `CustomersClient::create_customer` to create.
    pub async fn find_or_create_customer(
        &self,
        party_id: Uuid,
        email: &str,
    ) -> Result<i32, ArClientError> {
        // Search by external_customer_id
        let search_url = format!(
            "{}/api/ar/customers?external_customer_id={}",
            self.base_url, party_id
        );
        let resp = self.http.get(&search_url).send().await?;

        if resp.status().as_u16() == 200 {
            let customers: Vec<Customer> = resp.json().await?;
            if let Some(customer) = customers
                .into_iter()
                .find(|c| c.external_customer_id.as_deref() == Some(&party_id.to_string()))
            {
                return Ok(customer.id);
            }
        }

        // Create new AR customer via generated typed client
        let body = CreateCustomerRequest {
            email: Some(email.to_string()),
            name: Some(format!("Party {}", party_id)),
            external_customer_id: Some(party_id.to_string()),
            metadata: None,
            party_id: Some(party_id),
        };
        let customer = self.customers.create_customer(&body).await?;
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
        let body = CreateInvoiceRequest {
            ar_customer_id,
            amount_cents: amount_minor as i32,
            currency: Some(currency.to_string()),
            correlation_id: Some(correlation_id.to_string()),
            party_id: Some(party_id),
            billing_period_end: None,
            billing_period_start: None,
            compliance_codes: None,
            due_at: None,
            line_item_details: None,
            metadata: None,
            status: None,
            subscription_id: None,
        };
        let invoice = self.invoices.create_invoice(&body).await?;
        Ok(invoice.into())
    }

    /// Finalize an existing draft AR invoice (draft -> open).
    pub async fn finalize_invoice(&self, invoice_id: i32) -> Result<ArInvoice, ArClientError> {
        let body = FinalizeInvoiceRequest { paid_at: None };
        let invoice = self.invoices.finalize_invoice(invoice_id, &body).await?;
        Ok(invoice.into())
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
