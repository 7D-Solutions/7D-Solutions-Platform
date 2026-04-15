//! FedEx carrier adapter integration tests (bd-ttdso).
//!
//! All tests are `#[ignore]` — they call the real FedEx Developer sandbox API
//! and require `FEDEX_CLIENT_ID`, `FEDEX_CLIENT_SECRET`, and
//! `FEDEX_ACCOUNT_NUMBER` to be set in the environment.
//!
//! Run via the carrier-integration CI job, or locally:
//! ```bash
//! FEDEX_CLIENT_ID=<id> FEDEX_CLIENT_SECRET=<secret> FEDEX_ACCOUNT_NUMBER=<acct> \
//!   cargo test -p shipping-receiving-rs -- fedex_carrier --include-ignored --test-threads=1 --nocapture
//! ```

use shipping_receiving_rs::domain::carrier_providers::{get_provider, CarrierProvider};

const FEDEX_SANDBOX_URL: &str = "https://apis-sandbox.fedex.com";

fn sandbox_config() -> serde_json::Value {
    let client_id = std::env::var("FEDEX_CLIENT_ID")
        .expect("FEDEX_CLIENT_ID must be set to run FedEx carrier integration tests");
    let client_secret = std::env::var("FEDEX_CLIENT_SECRET")
        .expect("FEDEX_CLIENT_SECRET must be set to run FedEx carrier integration tests");
    let account_number = std::env::var("FEDEX_ACCOUNT_NUMBER")
        .expect("FEDEX_ACCOUNT_NUMBER must be set to run FedEx carrier integration tests");
    serde_json::json!({
        "client_id":      client_id,
        "client_secret":  client_secret,
        "account_number": account_number,
        "base_url":       FEDEX_SANDBOX_URL,
    })
}

// ── 1. OAuth2: token acquisition succeeds ─────────────────────

#[tokio::test]
#[ignore]
async fn fedex_carrier_oauth_token_acquisition_succeeds() {
    use shipping_receiving_rs::domain::carrier_providers::fedex;

    let config = sandbox_config();
    let client_id = config["client_id"].as_str().expect("client_id required");
    let client_secret = config["client_secret"]
        .as_str()
        .expect("client_secret required");
    let base_url = config["base_url"].as_str().expect("base_url required");

    // Call get_rates with a minimal request — the first thing it does is
    // acquire a token, so a successful response proves OAuth works.
    let _provider = get_provider("fedex").expect("fedex provider must be registered");
    let req = serde_json::json!({
        "origin_zip": "10001",
        "dest_zip":   "90210",
        "weight_lbs": 5.0,
    });

    let _ = fedex::FedexCarrierProvider
        .get_rates(&req, &config)
        .await
        .expect("OAuth token acquisition or rate call failed");

    println!("FedEx OAuth token acquired successfully (base_url={base_url})");
    let _ = (client_id, client_secret); // suppress unused warnings
}

// ── 2. get_rates: at least one rate for a domestic package ────

#[tokio::test]
#[ignore]
async fn fedex_carrier_get_rates_returns_quotes_for_domestic_package() {
    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = sandbox_config();

    let req = serde_json::json!({
        "origin_zip": "10001",
        "dest_zip":   "90210",
        "weight_lbs": 10.0,
        "length_in":  12.0,
        "width_in":   12.0,
        "height_in":  12.0,
    });

    let rates = provider
        .get_rates(&req, &config)
        .await
        .expect("get_rates failed");

    assert!(
        !rates.is_empty(),
        "expected at least one rate quote for 10 lb 12×12×12 10001→90210, got none"
    );

    // Every quote must have consistent metadata.
    for rate in &rates {
        assert_eq!(rate.carrier_code, "fedex");
        assert!(
            !rate.service_level.is_empty(),
            "service_level must be non-empty"
        );
        assert!(rate.total_charge_minor > 0, "charge must be positive");
        assert_eq!(rate.currency, "USD");
    }

    // At least two different service types to verify we handle both Ground
    // and Express response shapes (the FedEx sandbox returns multiple services).
    let ground = rates
        .iter()
        .find(|r| r.service_level.to_lowercase().contains("ground"));
    let express = rates.iter().find(|r| {
        let sl = r.service_level.to_lowercase();
        sl.contains("overnight") || sl.contains("express")
    });

    println!(
        "FedEx rates for 10 lb 12×12×12 10001→90210: {} quote(s)",
        rates.len()
    );
    for r in &rates {
        println!(
            "  {} ({:?} days) — ${:.2}",
            r.service_level,
            r.estimated_days,
            r.total_charge_minor as f64 / 100.0
        );
    }

    // Soft assertions — the sandbox may not return every service type every
    // time, but when it does, the fields must be populated.
    if let Some(g) = ground {
        assert!(
            g.total_charge_minor > 0,
            "FedEx Ground charge must be positive"
        );
    }
    if let Some(e) = express {
        assert!(
            e.total_charge_minor > 0,
            "FedEx Express charge must be positive"
        );
    }
}

// ── 3. create_label: returns a valid FedEx tracking number ────

#[tokio::test]
#[ignore]
async fn fedex_carrier_create_label_returns_tracking_number() {
    let provider = get_provider("fedex").expect("fedex provider must be registered");
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
        "length_in":    12.0,
        "width_in":     12.0,
        "height_in":    12.0,
    });

    let label = provider
        .create_label(&req, &config)
        .await
        .expect("create_label failed");

    assert_eq!(label.carrier_code, "fedex");
    assert!(
        !label.tracking_number.is_empty(),
        "tracking_number must be non-empty"
    );

    // FedEx tracking numbers are 12–34 digits.
    let digits: String = label
        .tracking_number
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    assert!(
        (12..=34).contains(&digits.len()),
        "FedEx tracking number must be 12–34 digits, got '{}' ({} digits)",
        label.tracking_number,
        digits.len()
    );

    assert_eq!(label.label_format, "pdf");

    println!(
        "FedEx label created: tracking={}, label_bytes={}",
        label.tracking_number,
        label.label_data.len()
    );
}

// ── 4. track: label tracking number returns a valid status ────

#[tokio::test]
#[ignore]
async fn fedex_carrier_track_created_label_returns_valid_status() {
    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = sandbox_config();

    // Create a label first so we have a real sandbox tracking number.
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
        "length_in":    12.0,
        "width_in":     12.0,
        "height_in":    12.0,
    });

    let label = provider
        .create_label(&req, &config)
        .await
        .expect("create_label failed (needed to get a tracking number for track test)");

    let tracking_number = label.tracking_number.clone();
    println!("FedEx label tracking number: {tracking_number}");

    let result = provider
        .track(&tracking_number, &config)
        .await
        .expect("track failed");

    assert_eq!(result.carrier_code, "fedex");
    assert_eq!(result.tracking_number, tracking_number);
    assert!(!result.status.is_empty(), "status must be non-empty");

    println!(
        "FedEx tracking {}: status={}, events={}",
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
fn fedex_carrier_registry_resolves_fedex_provider() {
    let provider = get_provider("fedex");
    assert!(
        provider.is_some(),
        "fedex must be registered in get_provider()"
    );
    assert_eq!(provider.expect("fedex provider").carrier_code(), "fedex");
}
