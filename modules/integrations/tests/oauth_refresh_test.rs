//! Integration tests for HttpTokenRefresher UPS + FedEx arms (bd-6rlla).
//!
//! Tests call real provider HTTPS endpoints and skip automatically when the
//! required credentials are absent from the environment.
//!
//! Required env vars to enable live tests:
//!   UPS:   UPS_CLIENT_ID, UPS_CLIENT_SECRET, UPS_TOKEN_URL (sandbox URL),
//!          UPS_SANDBOX_REFRESH_TOKEN
//!   FedEx: FEDEX_CLIENT_ID, FEDEX_CLIENT_SECRET, FEDEX_TOKEN_URL (sandbox URL)
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test oauth_refresh_test

use integrations_rs::domain::oauth::refresh::{HttpTokenRefresher, TokenRefresher};
use serial_test::serial;

fn make_refresher() -> HttpTokenRefresher {
    HttpTokenRefresher {
        client: reqwest::Client::new(),
        qbo_client_id: String::new(),
        qbo_client_secret: String::new(),
        qbo_token_url: String::new(),
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

// ── 1. UPS refresh rotates refresh token ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn ups_refresh_exchanges_refresh_token() {
    // Skip unless all required sandbox credentials are present in the environment.
    let _client_id = match env_nonempty("UPS_CLIENT_ID") {
        Some(v) => v,
        None => return,
    };
    let _client_secret = match env_nonempty("UPS_CLIENT_SECRET") {
        Some(v) => v,
        None => return,
    };
    let refresh_token_val = match env_nonempty("UPS_SANDBOX_REFRESH_TOKEN") {
        Some(v) => v,
        None => return,
    };
    // UPS_TOKEN_URL must point to the sandbox endpoint; skip if not configured.
    if env_nonempty("UPS_TOKEN_URL").is_none() {
        return;
    }

    let refresher = make_refresher();
    let result = refresher.refresh_token("ups", &refresh_token_val).await;

    assert!(
        result.is_ok(),
        "UPS refresh must succeed; got: {:?}",
        result.err()
    );
    assert!(
        !result.unwrap().access_token.is_empty(),
        "access_token must be non-empty"
    );
}

// ── 2. FedEx client_credentials mints fresh access token ─────────────────────

#[tokio::test]
#[serial]
async fn fedex_refresh_mints_new_access_token() {
    // Skip unless all required sandbox credentials are present in the environment.
    let _client_id = match env_nonempty("FEDEX_CLIENT_ID") {
        Some(v) => v,
        None => return,
    };
    let _client_secret = match env_nonempty("FEDEX_CLIENT_SECRET") {
        Some(v) => v,
        None => return,
    };
    // FEDEX_TOKEN_URL must point to the sandbox endpoint; skip if not configured.
    if env_nonempty("FEDEX_TOKEN_URL").is_none() {
        return;
    }

    let refresher = make_refresher();
    // FedEx client_credentials: refresh_token arg is unused by the FedEx arm.
    let result = refresher.refresh_token("fedex", "").await;

    assert!(
        result.is_ok(),
        "FedEx refresh must succeed; got: {:?}",
        result.err()
    );
    assert!(
        !result.unwrap().access_token.is_empty(),
        "access_token must be non-empty"
    );
}

// ── 3. Unknown provider returns unsupported-provider error ────────────────────

#[tokio::test]
#[serial]
async fn unknown_provider_returns_error() {
    let refresher = make_refresher();
    let result = refresher.refresh_token("shippo", "any-token").await;
    assert!(result.is_err(), "unknown provider must return Err");
    let msg = result.unwrap_err();
    assert!(
        msg.contains("shippo"),
        "error message must name the provider; got: {}",
        msg
    );
}
