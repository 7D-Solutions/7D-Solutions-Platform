//! Sandbox test helpers: retry policy, unique data generators, cleanup.

use std::time::Duration;
use uuid::Uuid;

/// Bounded retry policy for transient errors (network, 5xx).
/// 3 attempts with exponential backoff: 1s, 2s, 4s.
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_secs(1),
        }
    }
}

impl RetryPolicy {
    /// More patient retry for endpoints prone to Cloudflare rate limiting (429/1015).
    /// 5 attempts with 3s base: 3s, 6s, 12s, 24s, 48s window.
    pub fn patient() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_secs(3),
        }
    }
}

impl RetryPolicy {
    /// Execute an async closure with bounded retries on transient errors.
    /// Retries on: network errors (reqwest), HTTP 5xx, HTTP 429.
    /// Fails fast on: 4xx (except 429), parse errors, config errors.
    pub async fn execute<F, Fut, T, E>(&self, mut f: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display + IsTransient,
    {
        for attempt in 0..self.max_attempts {
            match f().await {
                Ok(val) => return Ok(val),
                Err(e) => {
                    if !e.is_transient() || attempt + 1 == self.max_attempts {
                        return Err(e);
                    }
                    let delay = self.base_delay * 2u32.pow(attempt);
                    eprintln!(
                        "[sandbox-retry] attempt {}/{} failed ({}), retrying in {:?}",
                        attempt + 1,
                        self.max_attempts,
                        e,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                }
            }
        }
        unreachable!("loop always returns")
    }
}

/// Trait to classify errors as transient (retryable) vs permanent.
pub trait IsTransient {
    fn is_transient(&self) -> bool;
}

impl IsTransient for ar_rs::tilled::error::TilledError {
    fn is_transient(&self) -> bool {
        use ar_rs::tilled::error::TilledError;
        match self {
            TilledError::HttpError(_) => true,
            TilledError::ApiError { status_code, .. } => *status_code >= 500 || *status_code == 429,
            TilledError::ConfigError(_)
            | TilledError::ParseError(_)
            | TilledError::ValidationError(_)
            | TilledError::WebhookVerificationFailed => false,
        }
    }
}

/// Generate a unique email that won't collide across parallel test runs.
pub fn unique_email() -> String {
    format!("sandbox-test-{}@7d-test.example.com", Uuid::new_v4())
}

/// Generate a unique reference ID for test objects.
pub fn unique_ref() -> String {
    format!("sandbox-ref-{}", Uuid::new_v4())
}

/// Generate unique metadata for test objects.
pub fn unique_metadata() -> serde_json::Value {
    serde_json::json!({
        "test_run": Uuid::new_v4().to_string(),
        "harness": "tilled_sandbox"
    })
}

/// Generate a minimal 1x1 valid PNG (white pixel) for evidence upload tests.
pub fn minimal_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

/// Best-effort cleanup of a Tilled customer by ID.
/// Logs but does not panic on failure.
pub async fn cleanup_customer(client: &ar_rs::tilled::TilledClient, customer_id: &str) {
    match client.delete_customer(customer_id).await {
        Ok(_) => eprintln!("[sandbox-cleanup] deleted customer {customer_id}"),
        Err(e) => eprintln!("[sandbox-cleanup] could not delete customer {customer_id}: {e}"),
    }
}

/// Best-effort detach of a payment method by ID.
pub async fn cleanup_payment_method(client: &ar_rs::tilled::TilledClient, pm_id: &str) {
    match client.detach_payment_method(pm_id).await {
        Ok(_) => eprintln!("[sandbox-cleanup] detached payment method {pm_id}"),
        Err(e) => eprintln!("[sandbox-cleanup] could not detach pm {pm_id}: {e}"),
    }
}

/// Create a test payment method via the Tilled API using sandbox test card.
/// Tilled.js is the normal path, but sandbox allows raw card details server-side.
/// Returns the payment method ID on success.
pub async fn create_test_payment_method(
    secret_key: &str,
    account_id: &str,
    base_url: &str,
) -> Result<ar_rs::tilled::types::PaymentMethod, ar_rs::tilled::error::TilledError> {
    let http = reqwest::Client::new();
    let body = serde_json::json!({
        "type": "card",
        "card": {
            "number": "4111111111111111",
            "exp_month": 12,
            "exp_year": 2030,
            "cvv": "123"
        },
        "billing_details": {
            "name": format!("Sandbox Test {}", Uuid::new_v4()),
            "address": {
                "country": "US",
                "zip": "90210"
            }
        }
    });

    let resp = http
        .post(format!("{base_url}/v1/payment-methods"))
        .header("Authorization", format!("Bearer {secret_key}"))
        .header("tilled-account", account_id)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| ar_rs::tilled::error::TilledError::HttpError(e.to_string()))?;

    let status = resp.status();
    if status.is_success() {
        resp.json()
            .await
            .map_err(|e| ar_rs::tilled::error::TilledError::ParseError(e.to_string()))
    } else {
        let text = resp.text().await.unwrap_or_else(|_| "unknown".to_string());
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: status.as_u16(),
            message: text,
        })
    }
}

/// Try to create a test payment method — returns None if cards aren't enabled
/// on this sandbox account (instead of panicking).
pub async fn try_create_test_payment_method(
    secret_key: &str,
    account_id: &str,
    base_url: &str,
) -> Option<ar_rs::tilled::types::PaymentMethod> {
    match create_test_payment_method(secret_key, account_id, base_url).await {
        Ok(pm) => Some(pm),
        Err(ar_rs::tilled::error::TilledError::ApiError {
            status_code: 400,
            ref message,
        }) if message.contains("not enabled") => {
            eprintln!("SKIP: card payment methods not enabled on this sandbox account");
            None
        }
        Err(e) => panic!("create_test_payment_method failed unexpectedly: {e}"),
    }
}

/// Best-effort cancel of a subscription by ID.
pub async fn cleanup_subscription(client: &ar_rs::tilled::TilledClient, sub_id: &str) {
    match client.cancel_subscription(sub_id).await {
        Ok(_) => eprintln!("[sandbox-cleanup] canceled subscription {sub_id}"),
        Err(e) => eprintln!("[sandbox-cleanup] could not cancel sub {sub_id}: {e}"),
    }
}
