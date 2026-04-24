//! Integration tests for HttpTokenRefresher UPS + FedEx arms (bd-6rlla).
//!
//! Tests call real provider HTTPS endpoints. They skip automatically when the
//! required sandbox credentials are absent from the environment.
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

// ── 1. UPS refresh rotates refresh token ─────────────────────────────────────

#[tokio::test]
#[serial]
async fn ups_refresh_exchanges_refresh_token() {
    let client_id = match std::env::var("UPS_SANDBOX_CLIENT_ID").ok().filter(|v| !v.is_empty()) {
        Some(v) => v,
        None => return,
    };
    let client_secret =
        match std::env::var("UPS_SANDBOX_CLIENT_SECRET").ok().filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => return,
        };
    let refresh_token_val =
        match std::env::var("UPS_SANDBOX_REFRESH_TOKEN").ok().filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => return,
        };

    // Point the refresher at sandbox endpoint + credentials.
    unsafe {
        std::env::set_var("UPS_CLIENT_ID", &client_id);
        std::env::set_var("UPS_CLIENT_SECRET", &client_secret);
        std::env::set_var(
            "UPS_TOKEN_URL",
            "https://wwwcie.ups.com/security/v1/oauth/refresh",
        );
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
    let client_id =
        match std::env::var("FEDEX_SANDBOX_CLIENT_ID").ok().filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => return,
        };
    let client_secret =
        match std::env::var("FEDEX_SANDBOX_CLIENT_SECRET").ok().filter(|v| !v.is_empty()) {
            Some(v) => v,
            None => return,
        };

    // Point the refresher at sandbox endpoint + credentials.
    unsafe {
        std::env::set_var("FEDEX_CLIENT_ID", &client_id);
        std::env::set_var("FEDEX_CLIENT_SECRET", &client_secret);
        std::env::set_var("FEDEX_TOKEN_URL", "https://apis-sandbox.fedex.com/oauth/token");
    }

    let refresher = make_refresher();
    // FedEx client_credentials: refresh_token arg is unused.
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
