//! Checkout session types, validation, and Tilled payment processor integration.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================================
// Request / Response types
// ============================================================================

/// POST /api/payments/checkout-sessions request body
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateCheckoutSessionRequest {
    pub invoice_id: String,
    pub tenant_id: String,
    /// Amount in minor currency units (e.g. cents)
    pub amount: i64,
    pub currency: String,
    /// URL to redirect after successful payment (optional)
    pub return_url: Option<String>,
    /// URL to redirect after cancelled payment (optional)
    pub cancel_url: Option<String>,
    /// Optional idempotency key. If omitted, invoice_id is used as the
    /// natural key — one checkout session per invoice per tenant.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// POST /api/payments/checkout-sessions response
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateCheckoutSessionResponse {
    pub session_id: String,
    pub payment_intent_id: String,
    /// Tilled.js client secret — pass to tilled.js confirmPayment()
    pub client_secret: String,
}

/// GET /api/payments/checkout-sessions/:id response
#[derive(Debug, Serialize, ToSchema)]
pub struct CheckoutSessionStatusResponse {
    pub session_id: String,
    pub status: String,
    pub payment_intent_id: String,
    pub invoice_id: String,
    pub tenant_id: String,
    pub amount: i64,
    pub currency: String,
    /// URL to redirect after successful payment (stored at creation time)
    pub return_url: Option<String>,
    /// URL to redirect after cancelled payment (stored at creation time)
    pub cancel_url: Option<String>,
}

/// GET /api/payments/checkout-sessions/:id/status response (no secrets)
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionStatusPollResponse {
    pub session_id: String,
    pub status: String,
}

// ============================================================================
// URL validation
// ============================================================================

/// Validate that a redirect URL is absolute HTTPS with no injection characters.
/// Enforces: https:// scheme, max 2048 chars, no control characters.
pub fn validate_https_url(url: &str) -> bool {
    if !url.starts_with("https://") {
        return false;
    }
    if url.len() > 2048 {
        return false;
    }
    // Reject control characters (injection prevention)
    if url.chars().any(|c| (c as u32) < 0x20) {
        return false;
    }
    true
}

// ============================================================================
// Tilled API helpers
// ============================================================================

/// Create a PaymentIntent in `requires_payment_method` state (no confirmation).
/// Returns (payment_intent_id, client_secret).
pub async fn create_tilled_payment_intent(
    api_key: &str,
    account_id: &str,
    amount: i64,
    currency: &str,
) -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.tilled.com/v1/payment-intents?tilled_account={}",
        account_id
    );
    let body = serde_json::json!({
        "amount": amount,
        "currency": currency.to_lowercase(),
        "payment_method_types": ["card"],
        "capture_method": "automatic",
    });

    let resp = client
        .post(&url)
        .header("tilled-account", account_id)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Tilled create PI failed ({}): {}", status, text);
    }

    let pi: serde_json::Value = resp.json().await?;
    let pi_id = pi["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Tilled response missing id"))?
        .to_string();
    let client_secret = pi["client_secret"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Tilled response missing client_secret"))?
        .to_string();

    Ok((pi_id, client_secret))
}

/// Query Tilled for current PaymentIntent status and map to our session status string.
pub async fn poll_tilled_intent_status(
    api_key: &str,
    account_id: &str,
    pi_id: &str,
) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.tilled.com/v1/payment-intents/{}?tilled_account={}",
        pi_id, account_id
    );

    let resp = client
        .get(&url)
        .header("tilled-account", account_id)
        .bearer_auth(api_key)
        .send()
        .await?;

    if !resp.status().is_success() {
        anyhow::bail!("Tilled query PI failed ({})", resp.status());
    }

    let pi: serde_json::Value = resp.json().await?;
    let status = match pi["status"].as_str().unwrap_or("unknown") {
        "succeeded" => "completed",
        "canceled" => "canceled",
        "requires_payment_method" | "requires_action" | "processing" | "requires_confirmation" => {
            "presented"
        }
        _ => "created",
    };
    Ok(status.to_string())
}
