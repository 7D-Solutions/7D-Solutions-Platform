//! Integration tests for Old Dominion (ODFL) CarrierProvider (bd-3kp9s).
//!
//! Tests call real ODFL HTTPS endpoints and skip automatically when
//! ODFL_SANDBOX_API_KEY or ODFL_SANDBOX_ACCOUNT is absent from the environment.
//!
//! Required env vars to enable live tests:
//!   ODFL_SANDBOX_API_KEY   — ODFL sandbox API key
//!   ODFL_SANDBOX_ACCOUNT   — ODFL account number
//!   ODFL_SANDBOX_URL       — optional, defaults to https://rest.odfl.com/ODSWS-Sandbox
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p shipping-receiving --test odfl_carrier

use shipping_receiving_rs::domain::carrier_providers::{CarrierProviderError, get_provider};

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn odfl_config() -> serde_json::Value {
    let api_key = env_nonempty("ODFL_SANDBOX_API_KEY").unwrap_or_default();
    let account_number = env_nonempty("ODFL_SANDBOX_ACCOUNT").unwrap_or_default();
    let base_url = env_nonempty("ODFL_SANDBOX_URL")
        .unwrap_or_else(|| "https://rest.odfl.com/ODSWS-Sandbox".to_string());
    serde_json::json!({
        "api_key": api_key,
        "account_number": account_number,
        "base_url": base_url
    })
}

fn have_credentials() -> bool {
    env_nonempty("ODFL_SANDBOX_API_KEY").is_some()
        && env_nonempty("ODFL_SANDBOX_ACCOUNT").is_some()
}

// ── 1. Rate quote returns service options ─────────────────────────────────────

#[tokio::test]
async fn odfl_rate_quote_returns_services() {
    dotenvy::dotenv().ok();
    if !have_credentials() {
        return;
    }

    let provider = get_provider("odfl").expect("odfl provider must be registered");
    let config = odfl_config();

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

    assert!(!quotes.is_empty(), "ODFL must return at least one service option");
    assert!(
        quotes.iter().all(|q| q.carrier_code == "odfl"),
        "all quotes must carry carrier_code 'odfl'"
    );
    assert!(
        quotes.iter().all(|q| q.total_charge_minor > 0),
        "all quotes must have a positive charge"
    );
}

// ── 2. Unknown PRO number maps to NotFound ────────────────────────────────────

#[tokio::test]
async fn odfl_tracking_unknown_returns_not_found() {
    dotenvy::dotenv().ok();
    if !have_credentials() {
        return;
    }

    let provider = get_provider("odfl").expect("odfl provider must be registered");
    let config = odfl_config();

    let result = provider
        .track("UNKNOWN-PRO-000000000", &config)
        .await;

    assert!(
        matches!(result, Err(CarrierProviderError::NotFound(_))),
        "expected NotFound for unknown PRO, got: {result:?}"
    );
}
