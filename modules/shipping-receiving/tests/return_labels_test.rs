//! Return label sandbox integration tests (bd-gl17g).
//!
//! Tests call real carrier sandbox APIs and skip automatically when
//! the required sandbox credentials are absent from the environment.
//!
//! Required env vars to enable live tests:
//!   UPS:   UPS_CLIENT_ID, UPS_CLIENT_SECRET, UPS_ACCOUNT_NUMBER
//!   FedEx: FEDEX_CLIENT_ID, FEDEX_CLIENT_SECRET, FEDEX_ACCOUNT_NUMBER
//!   USPS:  USPS_USER_ID
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p shipping-receiving-rs --test return_labels_test

use shipping_receiving_rs::domain::carrier_providers::get_provider;

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

// ── Shared test request ────────────────────────────────────────
//
// Simulates a customer in San Francisco returning a package to our Atlanta
// warehouse. Addresses are already swapped (customer=from, warehouse=to).

fn return_req() -> serde_json::Value {
    serde_json::json!({
        "from_name":    "Customer Returns",
        "from_address": "456 Market St",
        "from_city":    "San Francisco",
        "from_state":   "CA",
        "from_zip":     "94105",
        "to_name":      "7D Warehouse Returns",
        "to_address":   "123 Industrial Blvd",
        "to_city":      "Atlanta",
        "to_state":     "GA",
        "to_zip":       "30301",
        "weight_lbs":   5.0,
        "length_in":    10.0,
        "width_in":     8.0,
        "height_in":    4.0,
        "description":  "Return Shipment",
    })
}

// ── 1. UPS return label ───────────────────────────────────────

#[tokio::test]
async fn ups_return_label_generates_tracking() {
    dotenvy::dotenv().ok();
    if env_nonempty("UPS_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = serde_json::json!({
        "client_id":      env_nonempty("UPS_CLIENT_ID").unwrap_or_default(),
        "client_secret":  env_nonempty("UPS_CLIENT_SECRET").unwrap_or_default(),
        "account_number": env_nonempty("UPS_ACCOUNT_NUMBER").unwrap_or_default(),
        "base_url":       "https://wwwcie.ups.com",
    });

    let result = provider
        .create_return_label(&return_req(), &config)
        .await
        .expect("UPS create_return_label should succeed with valid sandbox credentials");

    assert!(
        !result.tracking_number.is_empty(),
        "UPS return label must return a tracking number"
    );
    assert_eq!(result.carrier_code, "ups", "carrier_code must be 'ups'");
    assert!(
        !result.label_data.is_empty(),
        "UPS return label must include label data"
    );
}

// ── 2. FedEx return label ─────────────────────────────────────

#[tokio::test]
async fn fedex_return_label_generates_tracking() {
    dotenvy::dotenv().ok();
    if env_nonempty("FEDEX_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = serde_json::json!({
        "client_id":      env_nonempty("FEDEX_CLIENT_ID").unwrap_or_default(),
        "client_secret":  env_nonempty("FEDEX_CLIENT_SECRET").unwrap_or_default(),
        "account_number": env_nonempty("FEDEX_ACCOUNT_NUMBER").unwrap_or_default(),
        "base_url":       "https://apis-sandbox.fedex.com",
    });

    let result = provider
        .create_return_label(&return_req(), &config)
        .await
        .expect("FedEx create_return_label should succeed with valid sandbox credentials");

    assert!(
        !result.tracking_number.is_empty(),
        "FedEx return label must return a tracking number"
    );
    assert_eq!(result.carrier_code, "fedex", "carrier_code must be 'fedex'");
    assert!(
        !result.label_data.is_empty(),
        "FedEx return label must include label data"
    );
}

// ── 3. USPS return label ──────────────────────────────────────

#[tokio::test]
async fn usps_return_label_generates_tracking() {
    dotenvy::dotenv().ok();
    if env_nonempty("USPS_USER_ID").is_none() {
        return;
    }

    let provider = get_provider("usps").expect("usps provider must be registered");
    let config = serde_json::json!({
        "user_id":  env_nonempty("USPS_USER_ID").unwrap_or_default(),
        "base_url": "https://secure.shippingapis.com/ShippingAPI.dll",
    });

    let result = provider
        .create_return_label(&return_req(), &config)
        .await
        .expect("USPS create_return_label should succeed with valid sandbox credentials");

    assert!(
        !result.tracking_number.is_empty(),
        "USPS return label must return a tracking number"
    );
    assert_eq!(result.carrier_code, "usps", "carrier_code must be 'usps'");
}

// ── 4. Stub provider returns a return label ───────────────────

#[tokio::test]
async fn stub_return_label_returns_fixed_result() {
    let provider = get_provider("stub").expect("stub provider must be registered");
    let result = provider
        .create_return_label(&serde_json::json!({}), &serde_json::json!({}))
        .await
        .expect("stub create_return_label must not fail");

    assert!(!result.tracking_number.is_empty());
    assert_eq!(result.carrier_code, "stub");
}
