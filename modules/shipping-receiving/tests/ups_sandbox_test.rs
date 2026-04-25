//! UPS sandbox integration tests (bd-a2rrm).
//!
//! Tests call the real UPS CIE sandbox API and skip automatically when
//! UPS_CLIENT_ID is absent from the environment.
//!
//! Required env vars to enable live tests:
//!   UPS_CLIENT_ID      — UPS OAuth2 client id
//!   UPS_CLIENT_SECRET  — UPS OAuth2 client secret
//!   UPS_ACCOUNT_NUMBER — UPS shipper account number
//!
//! Run:
//!   UPS_CLIENT_ID=<id> UPS_CLIENT_SECRET=<secret> UPS_ACCOUNT_NUMBER=<acct> \
//!     ./scripts/cargo-slot.sh test -p shipping-receiving --test ups_sandbox_test

use shipping_receiving_rs::domain::carrier_providers::get_provider;

const UPS_SANDBOX_URL: &str = "https://wwwcie.ups.com";

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn sandbox_config() -> serde_json::Value {
    serde_json::json!({
        "client_id":      env_nonempty("UPS_CLIENT_ID").unwrap_or_default(),
        "client_secret":  env_nonempty("UPS_CLIENT_SECRET").unwrap_or_default(),
        "account_number": env_nonempty("UPS_ACCOUNT_NUMBER").unwrap_or_default(),
        "base_url":       UPS_SANDBOX_URL,
    })
}

// ── 1. Rate includes UPS Ground ───────────────────────────────────────────────

#[tokio::test]
async fn ups_sandbox_rate_returns_ground() {
    dotenvy::dotenv().ok();
    if env_nonempty("UPS_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("ups").expect("ups provider must be registered");
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
        .expect("get_rates failed — check UPS sandbox credentials");

    assert!(!rates.is_empty(), "UPS sandbox returned no rate quotes");
    assert!(
        rates.iter().all(|r| r.carrier_code == "ups"),
        "all quotes must carry carrier_code 'ups'"
    );

    let ground = rates
        .iter()
        .find(|r| r.service_level.to_lowercase().contains("ground"));

    assert!(
        ground.is_some(),
        "UPS sandbox must include a Ground service for Atlanta→SF 10 lb; got: {:?}",
        rates.iter().map(|r| &r.service_level).collect::<Vec<_>>()
    );
    assert!(
        ground.unwrap().total_charge_minor > 0,
        "UPS Ground charge must be positive"
    );

    println!("UPS rates for 10 lb 30301→94105: {} quote(s)", rates.len());
    for r in &rates {
        println!(
            "  {} — ${:.2} ({:?} days)",
            r.service_level,
            r.total_charge_minor as f64 / 100.0,
            r.estimated_days
        );
    }
}

// ── 2. Create label — tracking number starts with "1Z" ───────────────────────

#[tokio::test]
async fn ups_sandbox_create_label_returns_tracking() {
    dotenvy::dotenv().ok();
    if env_nonempty("UPS_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("ups").expect("ups provider must be registered");
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
        .expect("create_label failed — check UPS sandbox credentials");

    assert_eq!(label.carrier_code, "ups");
    assert!(
        label.tracking_number.starts_with("1Z"),
        "UPS tracking numbers must start with '1Z', got: {}",
        label.tracking_number
    );
    assert!(!label.label_data.is_empty(), "label_data must be non-empty");

    println!(
        "UPS label created: tracking={}, label_bytes={}",
        label.tracking_number,
        label.label_data.len()
    );
}

// ── 3. Unknown tracking number returns an error ───────────────────────────────

#[tokio::test]
async fn ups_sandbox_tracking_unknown_returns_not_found() {
    dotenvy::dotenv().ok();
    if env_nonempty("UPS_CLIENT_ID").is_none() {
        return;
    }

    let provider = get_provider("ups").expect("ups provider must be registered");
    let config = sandbox_config();

    // Structurally valid 1Z format but will never match a real sandbox shipment.
    let result = provider.track("1ZZZZZZZ0000000000", &config).await;

    assert!(
        result.is_err(),
        "tracking an unknown number must return an error, got success: {result:?}"
    );

    println!(
        "UPS unknown tracking correctly rejected: {:?}",
        result.unwrap_err()
    );
}
