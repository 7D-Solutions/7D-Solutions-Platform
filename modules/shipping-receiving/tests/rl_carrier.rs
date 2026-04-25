//! Integration tests for R&L Carriers CarrierProvider (bd-gaqqv).
//!
//! Tests call real R&L HTTPS endpoints and skip automatically when
//! RL_SANDBOX_API_KEY is absent from the environment.
//!
//! Required env var to enable live tests:
//!   RL_SANDBOX_API_KEY  — R&L sandbox API key
//!   RL_SANDBOX_URL      — optional, defaults to https://api.rlcarriers.com
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p shipping-receiving --test rl_carrier

use shipping_receiving_rs::domain::carrier_providers::{CarrierProviderError, get_provider};

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn rl_config() -> serde_json::Value {
    let api_key = env_nonempty("RL_SANDBOX_API_KEY").unwrap_or_default();
    let base_url = env_nonempty("RL_SANDBOX_URL")
        .unwrap_or_else(|| "https://api.rlcarriers.com".to_string());
    serde_json::json!({
        "api_key": api_key,
        "base_url": base_url
    })
}

// ── 1. Rate quote returns service options ─────────────────────────────────────

#[tokio::test]
async fn rl_rate_quote_returns_services() {
    dotenvy::dotenv().ok();
    if env_nonempty("RL_SANDBOX_API_KEY").is_none() {
        return;
    }

    let provider = get_provider("rl").expect("rl provider must be registered");
    let config = rl_config();

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

    assert!(!quotes.is_empty(), "R&L must return at least one service option");
    assert!(
        quotes.iter().all(|q| q.carrier_code == "rl"),
        "all quotes must carry carrier_code 'rl'"
    );
    assert!(
        quotes.iter().all(|q| q.total_charge_minor > 0),
        "all quotes must have a positive charge"
    );
}

// ── 2. Unknown PRO number maps to NotFound ────────────────────────────────────

#[tokio::test]
async fn rl_tracking_unknown_pro_returns_not_found() {
    dotenvy::dotenv().ok();
    if env_nonempty("RL_SANDBOX_API_KEY").is_none() {
        return;
    }

    let provider = get_provider("rl").expect("rl provider must be registered");
    let config = rl_config();

    let result = provider
        .track("UNKNOWN-PRO-000000", &config)
        .await;

    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_))),
        "expected NotFound for unknown PRO, got: {result:?}"
    );
}
