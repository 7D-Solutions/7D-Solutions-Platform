//! USPS new-API sandbox integration tests (bd-a2rrm).
//!
//! Tests call the USPS OAuth2 REST API (api.usps.com) directly — not through
//! the legacy `UspsCarrierProvider`, which still uses the Web Tools XML API.
//! These tests document and verify the new API contract that the module will
//! migrate to; a future bead will wire UspsCarrierProvider to use this API.
//!
//! Tests skip automatically when USPS_CLIENT_ID is absent.
//!
//! Required env vars to enable live tests:
//!   USPS_CLIENT_ID     — USPS OAuth2 client id
//!   USPS_CLIENT_SECRET — USPS OAuth2 client secret
//!
//! Run:
//!   USPS_CLIENT_ID=<id> USPS_CLIENT_SECRET=<secret> \
//!     ./scripts/cargo-slot.sh test -p shipping-receiving --test usps_sandbox_test

use reqwest::Client;
use serde_json::Value;

const USPS_API_BASE: &str = "https://api.usps.com";

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Acquire a Bearer token via OAuth2 client_credentials.
async fn usps_token(client: &Client, client_id: &str, client_secret: &str) -> String {
    let resp = client
        .post(format!("{USPS_API_BASE}/oauth2/v3/token"))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("scope", "prices tracking"),
        ])
        .send()
        .await
        .expect("USPS OAuth token request failed");

    assert!(
        resp.status().is_success(),
        "USPS OAuth token endpoint returned HTTP {}: {}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );

    let json: Value = resp.json().await.expect("USPS token response is not JSON");
    json["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .expect("USPS token response missing access_token")
        .to_string()
}

// ── 1. Rate returns Priority Mail ─────────────────────────────────────────────

#[tokio::test]
async fn usps_sandbox_rate_returns_priority_mail() {
    dotenvy::dotenv().ok();
    let (Some(client_id), Some(client_secret)) = (
        env_nonempty("USPS_CLIENT_ID"),
        env_nonempty("USPS_CLIENT_SECRET"),
    ) else {
        return;
    };

    let client = Client::new();
    let token = usps_token(&client, &client_id, &client_secret).await;

    // POST /prices/v3/base-rates/search — domestic parcel, Atlanta→SF, 10 lb.
    let body = serde_json::json!({
        "originZIPCode":               "30301",
        "destinationZIPCode":          "94105",
        "weight":                      10.0,
        "length":                      12.0,
        "width":                       12.0,
        "height":                      12.0,
        "mailClass":                   "PRIORITY_MAIL",
        "processingCategory":          "NON_MACHINABLE",
        "destinationEntryFacilityType": "NONE",
        "rateIndicator":               "DR",
        "priceType":                   "RETAIL",
    });

    let resp = client
        .post(format!("{USPS_API_BASE}/prices/v3/base-rates/search"))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .expect("USPS prices request failed");

    let status = resp.status();
    let text = resp.text().await.expect("USPS prices response read failed");

    assert!(
        status.is_success(),
        "USPS /prices/v3/base-rates/search returned HTTP {status}: {text}"
    );

    let json: Value = serde_json::from_str(&text)
        .unwrap_or_else(|_| panic!("USPS prices response is not JSON: {text}"));

    // USPS returns a `rates` array; each entry has a `description` or `mailClass`.
    let rates = json["rates"]
        .as_array()
        .or_else(|| json.as_array())
        .expect("USPS prices response missing 'rates' array");

    assert!(
        !rates.is_empty(),
        "USPS prices API returned no rate entries; full response: {json}"
    );

    // At least one entry must reference Priority Mail.
    let has_priority = rates.iter().any(|r| {
        let desc = r["description"]
            .as_str()
            .or_else(|| r["mailClass"].as_str())
            .unwrap_or("");
        desc.to_lowercase().contains("priority")
    });

    assert!(
        has_priority,
        "USPS rates response must include a Priority Mail entry; got: {rates:?}"
    );

    println!(
        "USPS /prices/v3/base-rates/search: {} rate(s) returned",
        rates.len()
    );
    for r in rates {
        let desc = r["description"]
            .as_str()
            .or_else(|| r["mailClass"].as_str())
            .unwrap_or("?");
        let price = r["price"].as_f64().unwrap_or(0.0);
        println!("  {desc} — ${price:.2}");
    }
}

// ── 2. Unknown tracking number returns an error ───────────────────────────────

#[tokio::test]
async fn usps_sandbox_tracking_unknown_returns_not_found() {
    dotenvy::dotenv().ok();
    let (Some(client_id), Some(client_secret)) = (
        env_nonempty("USPS_CLIENT_ID"),
        env_nonempty("USPS_CLIENT_SECRET"),
    ) else {
        return;
    };

    let client = Client::new();
    let token = usps_token(&client, &client_id, &client_secret).await;

    // GET /tracking/v3/tracking/{trackingNumber} — bogus number, expect 404.
    let resp = client
        .get(format!(
            "{USPS_API_BASE}/tracking/v3/tracking/9999999999999999999999"
        ))
        .bearer_auth(&token)
        .header("Accept", "application/json")
        .send()
        .await
        .expect("USPS tracking request failed");

    let status = resp.status();
    assert!(
        !status.is_success(),
        "USPS tracking for unknown number must not succeed (got HTTP {status})"
    );

    println!("USPS unknown tracking correctly rejected with HTTP {status}");
}
