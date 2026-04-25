//! Label reprint integration tests.
//!
//! Tests run against real carrier sandboxes when the corresponding env vars are
//! set. Each carrier test is independently gated — set only the vars for the
//! carriers you have sandbox credentials for.
//!
//! Real-service note: the invalid-tracking tests use tracking numbers that no
//! sandbox will recognise. Carriers return 404 or an error for unknown tracking
//! numbers. This is intentional — we are testing the platform error-handling
//! path, not a real label. No mocks are needed because the carrier itself is
//! the system under test.

use shipping_receiving_rs::domain::carrier_providers::{
    get_provider, CarrierProviderError,
};

// ── helpers ───────────────────────────────────────────────────

fn pdf_magic(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%PDF")
}

fn sandbox_config(vars: &[(&str, &str)]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (k, v) in vars {
        map.insert(k.to_string(), serde_json::Value::String(v.to_string()));
    }
    serde_json::Value::Object(map)
}

// ── Stub — always runs ────────────────────────────────────────

#[tokio::test]
async fn stub_fetch_label_returns_pdf_magic_bytes() {
    let provider = get_provider("stub").expect("stub provider not found");
    let config = serde_json::json!({});
    let result = provider
        .fetch_label("STUB-TRACK-001", &config)
        .await
        .expect("stub fetch_label failed");
    assert!(!result.pdf_bytes.is_empty(), "pdf_bytes should not be empty");
    assert!(
        pdf_magic(&result.pdf_bytes),
        "stub bytes should start with %PDF"
    );
    assert_eq!(result.content_type, "application/pdf");
    assert_eq!(result.carrier_reference, "STUB-TRACK-001");
}

#[tokio::test]
async fn stub_fetch_label_not_found_returns_not_found_error() {
    let provider = get_provider("stub").expect("stub provider not found");
    let config = serde_json::json!({});
    let result = provider.fetch_label("NOTFOUND-123", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_))),
        "expected NotFound, got: {result:?}"
    );
}

// ── UPS sandbox ───────────────────────────────────────────────

#[tokio::test]
async fn ups_reprint_invalid_tracking_returns_not_found_or_carrier_error() {
    let client_id = match std::env::var("UPS_CLIENT_ID") {
        Ok(v) => v,
        Err(_) => return,
    };
    let client_secret = std::env::var("UPS_CLIENT_SECRET").unwrap_or_default();
    let account_number = std::env::var("UPS_ACCOUNT_NUMBER").unwrap_or_default();
    let config = sandbox_config(&[
        ("client_id", &client_id),
        ("client_secret", &client_secret),
        ("account_number", &account_number),
        ("base_url", "https://wwwcie.ups.com"),
    ]);
    let provider = get_provider("ups").expect("ups provider not found");
    let result = provider.fetch_label("1Z999AA10123456784", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown UPS tracking number, got: {result:?}"
    );
}

// ── FedEx sandbox ─────────────────────────────────────────────

#[tokio::test]
async fn fedex_reprint_invalid_tracking_returns_not_found_or_carrier_error() {
    let client_id = match std::env::var("FEDEX_CLIENT_ID") {
        Ok(v) => v,
        Err(_) => return,
    };
    let client_secret = std::env::var("FEDEX_CLIENT_SECRET").unwrap_or_default();
    let account_number = std::env::var("FEDEX_ACCOUNT_NUMBER").unwrap_or_default();
    let config = sandbox_config(&[
        ("client_id", &client_id),
        ("client_secret", &client_secret),
        ("account_number", &account_number),
        ("base_url", "https://apis-sandbox.fedex.com"),
    ]);
    let provider = get_provider("fedex").expect("fedex provider not found");
    let result = provider.fetch_label("9999999999999", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown FedEx tracking number, got: {result:?}"
    );
}

// ── USPS ──────────────────────────────────────────────────────

#[tokio::test]
async fn usps_reprint_missing_rest_token_returns_credentials_error() {
    let provider = get_provider("usps").expect("usps provider not found");
    let config = serde_json::json!({ "user_id": "TEST" });
    let result = provider.fetch_label("9400111899223397596405", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::CredentialsError(_))),
        "expected CredentialsError when rest_access_token missing, got: {result:?}"
    );
}

#[tokio::test]
async fn usps_reprint_invalid_tracking_returns_not_found_or_carrier_error() {
    let token = match std::env::var("USPS_REST_ACCESS_TOKEN") {
        Ok(v) => v,
        Err(_) => return,
    };
    let config = serde_json::json!({
        "user_id": "TEST",
        "rest_base_url": "https://api.usps.com",
        "rest_access_token": token,
    });
    let provider = get_provider("usps").expect("usps provider not found");
    let result = provider.fetch_label("9400000000000000000000", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown USPS tracking number, got: {result:?}"
    );
}

// ── R&L sandbox ───────────────────────────────────────────────

#[tokio::test]
async fn rl_reprint_invalid_pro_returns_not_found_or_carrier_error() {
    let api_key = match std::env::var("RL_API_KEY") {
        Ok(v) => v,
        Err(_) => return,
    };
    let config = sandbox_config(&[("api_key", &api_key)]);
    let provider = get_provider("rl").expect("rl provider not found");
    let result = provider.fetch_label("000-00000", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown R&L PRO, got: {result:?}"
    );
}

// ── XPO sandbox ───────────────────────────────────────────────

#[tokio::test]
async fn xpo_reprint_invalid_pro_returns_not_found_or_carrier_error() {
    let api_key = match std::env::var("XPO_API_KEY") {
        Ok(v) => v,
        Err(_) => return,
    };
    let config = sandbox_config(&[("api_key", &api_key)]);
    let provider = get_provider("xpo").expect("xpo provider not found");
    let result = provider.fetch_label("000000000", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown XPO PRO, got: {result:?}"
    );
}

// ── ODFL sandbox ──────────────────────────────────────────────

#[tokio::test]
async fn odfl_reprint_invalid_pro_returns_not_found_or_carrier_error() {
    let api_key = match std::env::var("ODFL_API_KEY") {
        Ok(v) => v,
        Err(_) => return,
    };
    let account_number = std::env::var("ODFL_ACCOUNT_NUMBER").unwrap_or_default();
    let config = sandbox_config(&[("api_key", &api_key), ("account_number", &account_number)]);
    let provider = get_provider("odfl").expect("odfl provider not found");
    let result = provider.fetch_label("00000000000", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown ODFL PRO, got: {result:?}"
    );
}

// ── Saia sandbox ──────────────────────────────────────────────

#[tokio::test]
async fn saia_reprint_invalid_pro_returns_not_found_or_carrier_error() {
    let api_key = match std::env::var("SAIA_API_KEY") {
        Ok(v) => v,
        Err(_) => return,
    };
    let account_number = std::env::var("SAIA_ACCOUNT_NUMBER").unwrap_or_default();
    let config = sandbox_config(&[("api_key", &api_key), ("account_number", &account_number)]);
    let provider = get_provider("saia").expect("saia provider not found");
    let result = provider.fetch_label("000000000", &config).await;
    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_)) | Err(CarrierProviderError::ApiError(_))),
        "expected NotFound or ApiError for unknown Saia PRO, got: {result:?}"
    );
}
