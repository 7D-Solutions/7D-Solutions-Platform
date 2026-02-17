//! AR module HTTP client for demo-seed
//!
//! Calls real AR HTTP endpoints to create customers and invoices.
//! All calls are idempotent via correlation_id.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Response from POST /api/ar/customers
#[derive(Debug, Deserialize)]
pub struct CustomerResponse {
    pub id: i32,
    #[allow(dead_code)]
    pub email: Option<String>,
}

/// Response from POST /api/ar/invoices
#[derive(Debug, Deserialize)]
pub struct InvoiceResponse {
    pub id: i32,
    #[allow(dead_code)]
    pub status: String,
    #[allow(dead_code)]
    pub amount_cents: i32,
}

#[derive(Serialize)]
struct CreateCustomerRequest {
    email: Option<String>,
    name: Option<String>,
    external_customer_id: Option<String>,
    metadata: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct CreateInvoiceRequest {
    ar_customer_id: i32,
    amount_cents: i32,
    currency: Option<String>,
    due_at: Option<String>,
    correlation_id: Option<String>,
    billing_period_start: Option<String>,
    billing_period_end: Option<String>,
}

/// Create a customer in AR. Returns the customer's database ID.
pub async fn create_customer(
    client: &reqwest::Client,
    ar_base_url: &str,
    tenant: &str,
    idx: usize,
    correlation_id: &str,
) -> Result<i32> {
    let url = format!("{}/api/ar/customers", ar_base_url);

    let body = CreateCustomerRequest {
        email: Some(format!("demo-customer-{}@{}.example", idx, tenant)),
        name: Some(format!("Demo Customer {} ({})", idx, tenant)),
        external_customer_id: Some(correlation_id.to_string()),
        metadata: Some(serde_json::json!({
            "demo": true,
            "seed_correlation_id": correlation_id,
        })),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("POST /api/ar/customers network error")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("POST /api/ar/customers failed {status}: {text}");
    }

    let customer: CustomerResponse = resp
        .json()
        .await
        .context("Failed to parse customer response")?;

    Ok(customer.id)
}

/// Create and finalize an invoice in AR. Returns the invoice's database ID.
///
/// Finalization is attempted but not required to succeed (e.g., if the invoice
/// is already finalized on a repeat run, it's a no-op).
pub async fn create_and_finalize_invoice(
    client: &reqwest::Client,
    ar_base_url: &str,
    customer_id: i32,
    amount_cents: i32,
    due_days: u32,
    correlation_id: &str,
) -> Result<i32> {
    let url = format!("{}/api/ar/invoices", ar_base_url);
    let now = Utc::now();
    let period_start = now.format("%Y-%m-%dT%H:%M:%S").to_string();
    let period_end = (now + chrono::Duration::days(30))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let due_at = (now + chrono::Duration::days(due_days as i64))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();

    let body = CreateInvoiceRequest {
        ar_customer_id: customer_id,
        amount_cents,
        currency: Some("USD".to_string()),
        due_at: Some(due_at),
        correlation_id: Some(correlation_id.to_string()),
        billing_period_start: Some(period_start),
        billing_period_end: Some(period_end),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .context("POST /api/ar/invoices network error")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("POST /api/ar/invoices failed {status}: {text}");
    }

    let invoice: InvoiceResponse = resp
        .json()
        .await
        .context("Failed to parse invoice response")?;

    let invoice_id = invoice.id;

    // Attempt to finalize (non-fatal if it fails — may already be finalized)
    let finalize_url = format!("{}/api/ar/invoices/{}/finalize", ar_base_url, invoice_id);
    let finalize_resp = client
        .post(&finalize_url)
        .send()
        .await
        .context("POST /api/ar/invoices/{id}/finalize network error")?;

    if !finalize_resp.status().is_success() {
        let status = finalize_resp.status();
        let text = finalize_resp.text().await.unwrap_or_default();
        // Log but don't fail — the invoice was created
        tracing::warn!(
            invoice_id,
            %status,
            "Invoice finalization failed (may already be finalized): {}",
            text
        );
    }

    Ok(invoice_id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ar_module_compiles() {
        // Structural test — real coverage in E2E tests
        assert!(true);
    }
}
