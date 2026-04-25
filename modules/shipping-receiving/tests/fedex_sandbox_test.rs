//! FedEx sandbox integration tests (bd-a2rrm).
//!
//! Tests call the real FedEx Developer sandbox API and skip automatically when
//! FEDEX_CLIENT_ID is absent from the environment.
//!
//! Required env vars to enable live tests:
//!   FEDEX_CLIENT_ID      — FedEx OAuth2 client id
//!   FEDEX_CLIENT_SECRET  — FedEx OAuth2 client secret
//!   FEDEX_ACCOUNT_NUMBER — FedEx account number
//!
//! Run:
//!   FEDEX_CLIENT_ID=<id> FEDEX_CLIENT_SECRET=<secret> FEDEX_ACCOUNT_NUMBER=<acct> \
//!     ./scripts/cargo-slot.sh test -p shipping-receiving --test fedex_sandbox_test

use shipping_receiving_rs::domain::carrier_providers::get_provider;

const FEDEX_SANDBOX_URL: &str = "https://apis-sandbox.fedex.com";

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn sandbox_config() -> serde_json::Value {
    serde_json::json!({
        "client_id":      env_nonempty("FEDEX_CLIENT_ID").unwrap_or_default(),
        "client_secret":  env_nonempty("FEDEX_CLIENT_SECRET").unwrap_or_default(),
        "account_number": env_nonempty("FEDEX_ACCOUNT_NUMBER").unwrap_or_default(),
        "base_url":       FEDEX_SANDBOX_URL,
    })
}

// ── 1. Rate includes FedEx Ground ─────────────────────────────────────────────

#[tokio::test]
async fn fedex_sandbox_rate_returns_ground() {
    dotenvy::dotenv().ok();
    if env_nonempty("FEDEX_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = sandbox_config();

    // Atlanta → San Francisco, 10 lb package (per bead spec).
    let req = serde_json::json!({
        "origin_zip":   "30301",
        "origin_city":  "Atlanta",
        "origin_state": "GA",
        "dest_zip":     "94105",
        "dest_city":    "San Francisco",
        "dest_state":   "CA",
        "weight_lbs":   10.0,
        "length_in":    12.0,
        "width_in":     12.0,
        "height_in":    12.0,
    });

    let rates = provider
        .get_rates(&req, &config)
        .await
        .expect("get_rates failed — check FedEx sandbox credentials");

    assert!(!rates.is_empty(), "FedEx sandbox returned no rate quotes");
    assert!(
        rates.iter().all(|r| r.carrier_code == "fedex"),
        "all quotes must carry carrier_code 'fedex'"
    );

    let ground = rates
        .iter()
        .find(|r| r.service_level.to_lowercase().contains("ground"));

    assert!(
        ground.is_some(),
        "FedEx sandbox must include a Ground service for Atlanta→SF 10 lb; got: {:?}",
        rates.iter().map(|r| &r.service_level).collect::<Vec<_>>()
    );
    assert!(
        ground.unwrap().total_charge_minor > 0,
        "FedEx Ground charge must be positive"
    );

    println!("FedEx rates for 10 lb 30301→94105: {} quote(s)", rates.len());
    for r in &rates {
        println!(
            "  {} — ${:.2} ({:?} days)",
            r.service_level,
            r.total_charge_minor as f64 / 100.0,
            r.estimated_days
        );
    }
}

// ── 2. Create label — tracking number is 12 digits ───────────────────────────

#[tokio::test]
async fn fedex_sandbox_create_label_returns_tracking() {
    dotenvy::dotenv().ok();
    if env_nonempty("FEDEX_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = sandbox_config();

    let req = serde_json::json!({
        "from_name":    "Acme Corp",
        "from_address": "101 Peachtree St",
        "from_city":    "Atlanta",
        "from_state":   "GA",
        "from_zip":     "30301",
        "to_name":      "Bob Smith",
        "to_address":   "1 Market St",
        "to_city":      "San Francisco",
        "to_state":     "CA",
        "to_zip":       "94105",
        "weight_lbs":   10.0,
        "length_in":    12.0,
        "width_in":     12.0,
        "height_in":    12.0,
    });

    let label = provider
        .create_label(&req, &config)
        .await
        .expect("create_label failed — check FedEx sandbox credentials");

    assert_eq!(label.carrier_code, "fedex");
    assert!(
        !label.tracking_number.is_empty(),
        "tracking_number must be non-empty"
    );

    let digits: String = label
        .tracking_number
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    assert_eq!(
        digits.len(),
        12,
        "FedEx tracking number must be exactly 12 digits, got '{}' ({} digits)",
        label.tracking_number,
        digits.len()
    );
    assert!(!label.label_data.is_empty(), "label_data must be non-empty");

    println!(
        "FedEx label created: tracking={}, label_bytes={}",
        label.tracking_number,
        label.label_data.len()
    );
}

// ── 3. Unknown tracking number returns an error ───────────────────────────────

#[tokio::test]
async fn fedex_sandbox_tracking_unknown_returns_not_found() {
    dotenvy::dotenv().ok();
    if env_nonempty("FEDEX_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("fedex").expect("fedex provider must be registered");
    let config = sandbox_config();

    // 12 digits of zeros — structurally plausible but will never match a sandbox shipment.
    let result = provider.track("000000000000", &config).await;

    assert!(
        result.is_err(),
        "tracking an unknown number must return an error, got success: {result:?}"
    );

    println!(
        "FedEx unknown tracking correctly rejected: {:?}",
        result.unwrap_err()
    );
}
