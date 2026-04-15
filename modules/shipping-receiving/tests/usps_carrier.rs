//! USPS carrier adapter integration tests (bd-1z8bl).
//!
//! All tests are `#[ignore]` — they call the real USPS Web Tools sandbox API
//! and require `USPS_USER_ID` to be set in the environment.
//!
//! Run via the carrier-integration CI job, or locally:
//! ```bash
//! USPS_USER_ID=<your_user_id> \
//!   cargo test -p shipping-receiving-rs -- usps_carrier --include-ignored --test-threads=1 --nocapture
//! ```

use shipping_receiving_rs::domain::carrier_providers::get_provider;

const USPS_SANDBOX_URL: &str = "https://stg-production.shippingapis.com/ShippingAPI.dll";

/// Known USPS sandbox tracking number that returns a valid response on the
/// test endpoint. USPS provides this in their Web Tools documentation.
const USPS_TEST_TRACKING_NUMBER: &str = "9400110200882774868522";

/// Returns `None` (and prints a notice) when `USPS_USER_ID` is absent so that
/// ignored tests skip cleanly rather than panic when run without credentials.
fn sandbox_config() -> Option<serde_json::Value> {
    match std::env::var("USPS_USER_ID") {
        Ok(user_id) => Some(serde_json::json!({
            "user_id": user_id,
            "base_url": USPS_SANDBOX_URL,
        })),
        Err(_) => {
            println!("USPS_USER_ID not set — skipping USPS sandbox tests");
            None
        }
    }
}

// ── 1. get_rates: domestic 10 lb 12×12×12 package ────────────

#[tokio::test]
#[ignore]
async fn usps_carrier_get_rates_returns_quotes_for_domestic_package() {
    let provider = get_provider("usps").expect("usps provider must be registered");
    let Some(config) = sandbox_config() else {
        return;
    };

    let req = serde_json::json!({
        "origin_zip":  "10001",
        "dest_zip":    "90210",
        "weight_lbs":  10,
        "length_in":   12,
        "width_in":    12,
        "height_in":   12,
    });

    let rates = provider
        .get_rates(&req, &config)
        .await
        .expect("get_rates failed");

    assert!(
        !rates.is_empty(),
        "expected at least one rate quote for 10 lb 12×12×12 10001→90210, got none"
    );

    for rate in &rates {
        assert_eq!(rate.carrier_code, "usps");
        assert!(
            !rate.service_level.is_empty(),
            "service_level must be non-empty"
        );
        assert!(rate.total_charge_minor > 0, "charge must be positive");
        assert_eq!(rate.currency, "USD");
    }

    println!(
        "USPS rates for 10 lb 12×12×12 10001→90210: {} quote(s)",
        rates.len()
    );
    for r in &rates {
        println!(
            "  {} — ${:.2}",
            r.service_level,
            r.total_charge_minor as f64 / 100.0
        );
    }
}

// ── 2. create_label: returns tracking number and label bytes ──

#[tokio::test]
#[ignore]
async fn usps_carrier_create_label_returns_tracking_and_label_bytes() {
    let provider = get_provider("usps").expect("usps provider must be registered");
    let Some(config) = sandbox_config() else {
        return;
    };

    let req = serde_json::json!({
        "from_name":    "Acme Corp",
        "from_address": "123 Main St",
        "from_city":    "New York",
        "from_state":   "NY",
        "from_zip":     "10001",
        "to_name":      "Bob Smith",
        "to_address":   "456 Sunset Blvd",
        "to_city":      "Beverly Hills",
        "to_state":     "CA",
        "to_zip":       "90210",
        "weight_lbs":   10,
        "length_in":    12,
        "width_in":     12,
        "height_in":    12,
    });

    let label = provider
        .create_label(&req, &config)
        .await
        .expect("create_label failed");

    assert_eq!(label.carrier_code, "usps");
    assert!(
        !label.tracking_number.is_empty(),
        "tracking_number must be non-empty"
    );
    assert!(
        !label.label_data.is_empty(),
        "label_data (base64 PDF) must be non-empty"
    );
    assert_eq!(label.label_format, "pdf");

    println!(
        "USPS label created: tracking={}, label_bytes={}",
        label.tracking_number,
        label.label_data.len()
    );
}

// ── 3. track: known test tracking number returns valid status ─

#[tokio::test]
#[ignore]
async fn usps_carrier_track_known_number_returns_valid_status() {
    let provider = get_provider("usps").expect("usps provider must be registered");
    let Some(config) = sandbox_config() else {
        return;
    };

    let result = provider
        .track(USPS_TEST_TRACKING_NUMBER, &config)
        .await
        .expect("track failed");

    assert_eq!(result.carrier_code, "usps");
    assert_eq!(result.tracking_number, USPS_TEST_TRACKING_NUMBER);
    assert!(!result.status.is_empty(), "status must be non-empty");

    println!(
        "USPS tracking {}: status={}, events={}",
        result.tracking_number,
        result.status,
        result.events.len()
    );
    for event in &result.events {
        println!(
            "  [{}] {} — {}",
            event.timestamp,
            event.description,
            event.location.as_deref().unwrap_or("—")
        );
    }
}

// ── Registry check (non-ignored, always runs) ─────────────────

#[test]
fn usps_carrier_registry_resolves_usps_provider() {
    let provider = get_provider("usps");
    assert!(
        provider.is_some(),
        "usps must be registered in get_provider()"
    );
    assert_eq!(provider.expect("usps provider").carrier_code(), "usps");
}
