//! QBO invoice sparse-update integration test (bd-ym43b).
//!
//! Run with: QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs --test qbo_update_invoice
//!
//! Requires:
//! - `.env.qbo-sandbox` with QBO_CLIENT_ID, QBO_CLIENT_SECRET, QBO_SANDBOX_BASE
//! - `.qbo-tokens.json` with access_token, refresh_token, realm_id

use integrations_rs::domain::qbo::{client::QboClient, QboError, TokenProvider};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

struct SandboxTokenProvider {
    access_token: RwLock<String>,
    refresh_tok: RwLock<String>,
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
    tokens_path: PathBuf,
}

impl SandboxTokenProvider {
    fn load() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        dotenvy::from_path(root.join(".env.qbo-sandbox")).expect(".env.qbo-sandbox not found");

        let client_id = std::env::var("QBO_CLIENT_ID").expect("QBO_CLIENT_ID");
        let client_secret = std::env::var("QBO_CLIENT_SECRET").expect("QBO_CLIENT_SECRET");

        let tokens_path = root.join(".qbo-tokens.json");
        let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json");
        let tokens: Value = serde_json::from_str(&content).expect("invalid tokens JSON");

        Self {
            access_token: RwLock::new(tokens["access_token"].as_str().unwrap().into()),
            refresh_tok: RwLock::new(tokens["refresh_token"].as_str().unwrap().into()),
            client_id,
            client_secret,
            http: reqwest::Client::new(),
            tokens_path,
        }
    }

    fn realm_id(&self) -> String {
        let content = std::fs::read_to_string(&self.tokens_path).unwrap();
        let t: Value = serde_json::from_str(&content).unwrap();
        t["realm_id"].as_str().unwrap().to_string()
    }
}

#[async_trait::async_trait]
impl TokenProvider for SandboxTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok(self.access_token.read().await.clone())
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        let rt = self.refresh_tok.read().await.clone();

        let resp = self
            .http
            .post("https://oauth.platform.intuit.com/oauth2/v1/tokens/bearer")
            .basic_auth(&self.client_id, Some(&self.client_secret))
            .form(&[("grant_type", "refresh_token"), ("refresh_token", &rt)])
            .send()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(QboError::TokenError(format!("Refresh failed: {}", body)));
        }

        let tr: Value = resp
            .json()
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))?;

        let new_at = tr["access_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no access_token".into()))?
            .to_string();
        let new_rt = tr["refresh_token"]
            .as_str()
            .ok_or_else(|| QboError::TokenError("no refresh_token".into()))?
            .to_string();

        *self.access_token.write().await = new_at.clone();
        *self.refresh_tok.write().await = new_rt.clone();

        // Persist to file so next run has valid tokens
        if let Ok(content) = std::fs::read_to_string(&self.tokens_path) {
            if let Ok(mut existing) = serde_json::from_str::<Value>(&content) {
                existing["access_token"] = Value::String(new_at.clone());
                existing["refresh_token"] = Value::String(new_rt);
                if let Some(v) = tr.get("expires_in") {
                    existing["expires_in"] = v.clone();
                }
                if let Some(v) = tr.get("x_refresh_token_expires_in") {
                    existing["x_refresh_token_expires_in"] = v.clone();
                }
                let _ = std::fs::write(
                    &self.tokens_path,
                    serde_json::to_string_pretty(&existing).unwrap(),
                );
            }
        }

        Ok(new_at)
    }
}

fn skip_unless_sandbox() -> bool {
    std::env::var("QBO_SANDBOX").map_or(true, |v| v != "1")
}

fn make_client() -> (QboClient, Arc<SandboxTokenProvider>) {
    let provider = Arc::new(SandboxTokenProvider::load());
    let base_url = std::env::var("QBO_SANDBOX_BASE")
        .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into());
    let realm_id = provider.realm_id();
    let client = QboClient::new(&base_url, &realm_id, provider.clone());
    (client, provider)
}

/// Test sparse update of invoice shipping fields via QboClient.
///
/// This validates the core update_entity flow that the HTTP handler depends on.
#[tokio::test]
async fn qbo_update_invoice_shipping_fields() {
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let (client, provider) = make_client();

    // Proactively refresh — access token is almost certainly expired (60 min TTL)
    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");
    eprintln!("Token refreshed successfully");

    // 1. Query for an invoice
    let inv_resp = client
        .query("SELECT * FROM Invoice MAXRESULTS 1")
        .await
        .expect("invoice query failed");
    let invoices = inv_resp["QueryResponse"]["Invoice"]
        .as_array()
        .expect("no Invoice array");
    assert!(!invoices.is_empty(), "sandbox should have sample invoices");

    let inv_id = invoices[0]["Id"].as_str().unwrap();
    eprintln!("Found invoice ID: {}", inv_id);

    // 2. Read current invoice
    let inv = client
        .get_entity("Invoice", inv_id)
        .await
        .expect("get invoice failed");
    let sync_token = inv["Invoice"]["SyncToken"].as_str().unwrap();
    eprintln!("Current SyncToken: {}", sync_token);

    // 3. Build sparse update with shipping fields
    let test_ship_date = "2026-03-29";
    let test_tracking = format!("1ZTEST{}", chrono::Utc::now().timestamp() % 100000);
    let test_carrier = "FedEx";

    let update_body = serde_json::json!({
        "Id": inv_id,
        "SyncToken": sync_token,
        "sparse": true,
        "ShipDate": test_ship_date,
        "TrackingNum": test_tracking,
        "ShipMethodRef": {"value": test_carrier}
    });

    // 4. Call update_entity (same flow as HTTP handler)
    let result = client
        .update_entity("Invoice", update_body)
        .await
        .expect("sparse update failed");

    let updated = &result["Invoice"];
    let new_sync_token = updated["SyncToken"].as_str().unwrap();
    eprintln!(
        "Update succeeded. New SyncToken: {} (was {})",
        new_sync_token, sync_token
    );

    // 5. Verify updated values in response
    assert_eq!(
        updated["ShipDate"].as_str(),
        Some(test_ship_date),
        "ShipDate not in response"
    );
    assert_eq!(
        updated["TrackingNum"].as_str(),
        Some(test_tracking.as_str()),
        "TrackingNum not in response"
    );
    assert!(
        updated["ShipMethodRef"]["value"].as_str().is_some(),
        "ShipMethodRef not in response"
    );

    // 6. Re-read to confirm persistence
    let re_read = client
        .get_entity("Invoice", inv_id)
        .await
        .expect("re-read invoice failed");
    let ri = &re_read["Invoice"];

    assert_eq!(
        ri["ShipDate"].as_str(),
        Some(test_ship_date),
        "ShipDate not persisted"
    );
    assert_eq!(
        ri["TrackingNum"].as_str(),
        Some(test_tracking.as_str()),
        "TrackingNum not persisted"
    );
    assert!(
        ri["ShipMethodRef"]["value"].as_str().is_some(),
        "ShipMethodRef not persisted"
    );

    eprintln!(
        "Verified persistence: ShipDate={}, TrackingNum={}, ShipMethodRef={}",
        ri["ShipDate"], ri["TrackingNum"], ri["ShipMethodRef"]["value"]
    );
}

/// Test that partial updates work (only some fields provided).
#[tokio::test]
async fn qbo_update_invoice_partial_fields() {
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO sandbox test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let (client, provider) = make_client();

    provider
        .refresh_token()
        .await
        .expect("token refresh via OAuth failed");

    // Query for invoice
    let inv_resp = client
        .query("SELECT * FROM Invoice MAXRESULTS 1")
        .await
        .expect("invoice query failed");
    let invoices = inv_resp["QueryResponse"]["Invoice"]
        .as_array()
        .expect("no Invoice array");
    let inv_id = invoices[0]["Id"].as_str().unwrap();

    let inv = client
        .get_entity("Invoice", inv_id)
        .await
        .expect("get invoice failed");
    let sync_token = inv["Invoice"]["SyncToken"].as_str().unwrap();

    // Update only TrackingNum (partial update)
    let test_tracking = format!("PARTIAL{}", chrono::Utc::now().timestamp() % 100000);
    let update_body = serde_json::json!({
        "Id": inv_id,
        "SyncToken": sync_token,
        "sparse": true,
        "TrackingNum": test_tracking,
    });

    let result = client
        .update_entity("Invoice", update_body)
        .await
        .expect("partial update failed");

    assert_eq!(
        result["Invoice"]["TrackingNum"].as_str(),
        Some(test_tracking.as_str()),
        "TrackingNum not updated"
    );
    eprintln!("Partial update succeeded: TrackingNum={}", test_tracking);
}
