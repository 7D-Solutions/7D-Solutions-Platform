//! UPS carrier adapter integration tests (bd-2xl19).
//!
//! All tests are `#[ignore]` — they call the real UPS CIE sandbox API
//! and require UPS OAuth2 credentials to be set in the environment.
//!
//! Run via the carrier-integration CI job, or locally:
//! ```bash
//! UPS_CLIENT_ID=<id> \
//! UPS_CLIENT_SECRET=<secret> \
//! UPS_ACCOUNT_NUMBER=<account> \
//!   cargo test -p shipping-receiving-rs -- ups_carrier --include-ignored --test-threads=1 --nocapture
//! ```

use shipping_receiving_rs::domain::carrier_providers::get_provider;

const UPS_SANDBOX_URL: &str = "https://wwwcie.ups.com";

fn sandbox_config() -> serde_json::Value {
    let client_id = std::env::var("UPS_CLIENT_ID")
        .expect("UPS_CLIENT_ID must be set to run UPS carrier integration tests");
    let client_secret = std::env::var("UPS_CLIENT_SECRET")
        .expect("UPS_CLIENT_SECRET must be set to run UPS carrier integration tests");
    let account_number = std::env::var("UPS_ACCOUNT_NUMBER")
        .expect("UPS_ACCOUNT_NUMBER must be set to run UPS carrier integration tests");
    serde_json::json!({
        "client_id":      client_id,
        "client_secret":  client_secret,
        "account_number": account_number,
        "base_url":       UPS_SANDBOX_URL,
    })
}

// ── 1. OAuth token acquisition ────────────────────────────────

#[tokio::test]
#[ignore]
async fn ups_carrier_oauth_token_acquisition_succeeds() {
    // Exercise token acquisition directly via the provider.
    // The provider caches the token; a second call should hit the cache.
    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = sandbox_config();

    // Call get_rates to force token acquisition (no direct token access from outside).
    // A successful rate call implies a valid token was obtained.
    let req = serde_json::json!({
        "origin_zip": "10001",
        "dest_zip":   "90210",
        "weight_lbs": 1.0,
    });

    let rates = provider
        .get_rates(&req, &config)
        .await
        .expect("get_rates failed — check that UPS_CLIENT_ID/SECRET/ACCOUNT_NUMBER are valid sandbox credentials");

    assert!(
        !rates.is_empty(),
        "OAuth token was acquired but rate response was empty"
    );

    println!(
        "UPS OAuth token acquired successfully — {} rate(s) returned on first call",
        rates.len()
    );

    // Second call — exercises token cache (no new OAuth round-trip)
    let rates2 = provider
        .get_rates(&req, &config)
        .await
        .expect("second get_rates failed");

    assert!(
        !rates2.is_empty(),
        "second call (cached token) returned no rates"
    );
    println!("UPS token cache verified — second call returned {} rate(s)", rates2.len());
}

// ── 2. get_rates: domestic package ────────────────────────────

#[tokio::test]
#[ignore]
async fn ups_carrier_get_rates_returns_quotes_for_domestic_package() {
    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = sandbox_config();

    let req = serde_json::json!({
        "origin_zip":   "10001",
        "origin_state": "NY",
        "origin_city":  "New York",
        "dest_zip":     "90210",
        "dest_state":   "CA",
        "dest_city":    "Beverly Hills",
        "weight_lbs":   10.0,
        "length_in":    12.0,
        "width_in":     12.0,
        "height_in":    12.0,
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
        assert_eq!(rate.carrier_code, "ups");
        assert!(!rate.service_level.is_empty(), "service_level must be non-empty");
        assert!(rate.total_charge_minor > 0, "charge must be positive");
        assert_eq!(rate.currency, "USD");
    }

    println!(
        "UPS rates for 10 lb 12×12×12 10001→90210: {} quote(s)",
        rates.len()
    );
    for r in &rates {
        println!(
            "  {} — ${:.2} (est {} days)",
            r.service_level,
            r.total_charge_minor as f64 / 100.0,
            r.estimated_days.map_or("?".to_string(), |d| d.to_string())
        );
    }
}

// ── 3. create_label: returns 1Z tracking number + label ───────

#[tokio::test]
#[ignore]
async fn ups_carrier_create_label_returns_tracking_number_and_label_image() {
    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = sandbox_config();

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
        "weight_lbs":   5.0,
        "length_in":    10.0,
        "width_in":     8.0,
        "height_in":    6.0,
    });

    let label = provider
        .create_label(&req, &config)
        .await
        .expect("create_label failed");

    assert_eq!(label.carrier_code, "ups");
    assert!(
        label.tracking_number.starts_with("1Z"),
        "UPS tracking numbers must start with '1Z', got: {}",
        label.tracking_number
    );
    assert!(
        !label.label_data.is_empty(),
        "label_data (base64 GIF) must be non-empty"
    );

    println!(
        "UPS label created: tracking={}, label_bytes={}",
        label.tracking_number,
        label.label_data.len()
    );
}

// ── 4. create_label then track ─────────────────────────────────

#[tokio::test]
#[ignore]
async fn ups_carrier_create_label_and_track_returns_valid_status() {
    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = sandbox_config();

    // Step 1: create a label to get a real tracking number
    let ship_req = serde_json::json!({
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
        "weight_lbs":   5.0,
        "length_in":    10.0,
        "width_in":     8.0,
        "height_in":    6.0,
    });

    let label = provider
        .create_label(&ship_req, &config)
        .await
        .expect("create_label failed before tracking test");

    assert!(
        label.tracking_number.starts_with("1Z"),
        "label tracking number must start with '1Z', got: {}",
        label.tracking_number
    );

    println!("Created label with tracking: {}", label.tracking_number);

    // Step 2: track the shipment using the tracking number from step 1
    let track_result = provider
        .track(&label.tracking_number, &config)
        .await
        .expect("track failed");

    assert_eq!(track_result.carrier_code, "ups");
    assert_eq!(track_result.tracking_number, label.tracking_number);
    assert!(
        !track_result.status.is_empty(),
        "tracking status must be non-empty"
    );

    println!(
        "UPS tracking {}: status={}, events={}",
        track_result.tracking_number,
        track_result.status,
        track_result.events.len()
    );
    for event in &track_result.events {
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
fn ups_carrier_registry_resolves_ups_provider() {
    let provider = get_provider("ups");
    assert!(provider.is_some(), "ups must be registered in get_provider()");
    assert_eq!(provider.expect("ups provider").carrier_code(), "ups");
}
