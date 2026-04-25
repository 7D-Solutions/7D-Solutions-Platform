//! Integration tests for XPO Logistics CarrierProvider (bd-owtje).
//!
//! Tests call real XPO HTTPS endpoints and skip automatically when
//! XPO_SANDBOX_API_KEY is absent from the environment.
//!
//! Required env vars to enable live tests:
//!   XPO_SANDBOX_API_KEY  — XPO sandbox API key
//!   XPO_SANDBOX_URL      — optional, defaults to https://apisandbox.xpo.com
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p shipping-receiving-rs --test xpo_carrier

use shipping_receiving_rs::domain::carrier_providers::{CarrierProviderError, get_provider};

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn xpo_config() -> serde_json::Value {
    let api_key = env_nonempty("XPO_SANDBOX_API_KEY").unwrap_or_default();
    let base_url = env_nonempty("XPO_SANDBOX_URL")
        .unwrap_or_else(|| "https://apisandbox.xpo.com".to_string());
    serde_json::json!({
        "api_key": api_key,
        "base_url": base_url
    })
}

fn have_credentials() -> bool {
    env_nonempty("XPO_SANDBOX_API_KEY").is_some()
}

// ── 1. Rate quote returns service options ─────────────────────────────────────

#[tokio::test]
async fn xpo_rate_quote_returns_services() {
    dotenvy::dotenv().ok();
    if !have_credentials() {
        return;
    }

    let provider = get_provider("xpo").expect("xpo provider must be registered");
    let config = xpo_config();

    // Atlanta → San Francisco, 500lb LTL pallet, class 70
    let req = serde_json::json!({
        "origin_zip":    "30301",
        "origin_city":   "Atlanta",
        "origin_state":  "GA",
        "dest_zip":      "94105",
        "dest_city":     "San Francisco",
        "dest_state":    "CA",
        "weight_lbs":    500,
        "freight_class": "70",
        "pieces":        1,
        "description":   "Test LTL Pallet"
    });

    let quotes = provider
        .get_rates(&req, &config)
        .await
        .expect("get_rates should succeed with valid sandbox credentials");

    assert!(!quotes.is_empty(), "XPO must return at least one service option");
    assert!(
        quotes.iter().all(|q| q.carrier_code == "xpo"),
        "all quotes must carry carrier_code 'xpo'"
    );
    assert!(
        quotes.iter().all(|q| q.total_charge_minor > 0),
        "all quotes must have a positive charge"
    );
}

// ── 2. Unknown PRO number maps to NotFound ────────────────────────────────────

#[tokio::test]
async fn xpo_tracking_unknown_returns_not_found() {
    dotenvy::dotenv().ok();
    if !have_credentials() {
        return;
    }

    let provider = get_provider("xpo").expect("xpo provider must be registered");
    let config = xpo_config();

    let result = provider
        .track("UNKNOWN-PRO-000000000", &config)
        .await;

    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_))),
        "expected NotFound for unknown PRO, got: {result:?}"
    );
}
