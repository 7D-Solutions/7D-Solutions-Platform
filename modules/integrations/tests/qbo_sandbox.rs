//! QBO sandbox integration tests (bd-2om69).
//!
//! Run with: QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs -- qbo_sandbox
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

/// Comprehensive sandbox test covering all acceptance criteria.
///
/// Runs as a single test to avoid token refresh races and minimize API calls.
#[tokio::test]
async fn qbo_sandbox_integration() {
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

    // --- 1. Read a customer from sandbox ---
    let cust_resp = client
        .query("SELECT * FROM Customer MAXRESULTS 1")
        .await
        .expect("customer query failed");
    let customers = cust_resp["QueryResponse"]["Customer"]
        .as_array()
        .expect("no Customer array");
    assert!(!customers.is_empty(), "sandbox should have sample customers");

    let cust_id = customers[0]["Id"].as_str().unwrap();
    let cust = client
        .get_entity("Customer", cust_id)
        .await
        .expect("get customer failed");
    assert_eq!(cust["Customer"]["Id"].as_str(), Some(cust_id));
    eprintln!(
        "Read customer: {} (ID {})",
        cust["Customer"]["DisplayName"], cust_id
    );

    // --- 2. Read an invoice with line items ---
    let inv_resp = client
        .query("SELECT * FROM Invoice MAXRESULTS 1")
        .await
        .expect("invoice query failed");
    let invoices = inv_resp["QueryResponse"]["Invoice"]
        .as_array()
        .expect("no Invoice array");
    assert!(!invoices.is_empty(), "sandbox should have sample invoices");

    let inv_id = invoices[0]["Id"].as_str().unwrap();
    let inv = client
        .get_entity("Invoice", inv_id)
        .await
        .expect("get invoice failed");
    let invoice = &inv["Invoice"];
    assert!(invoice["Line"].is_array(), "invoice should have Line items");
    eprintln!(
        "Read invoice {} with {} lines",
        inv_id,
        invoice["Line"].as_array().unwrap().len()
    );

    // --- 3. Query unpaid invoices ---
    let unpaid = client
        .query("SELECT * FROM Invoice WHERE Balance > '0' MAXRESULTS 10")
        .await
        .expect("unpaid invoice query failed");
    let unpaid_count = unpaid["QueryResponse"]["Invoice"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    eprintln!("Unpaid invoices found: {}", unpaid_count);

    // --- 4. Sparse update: ShipDate, ShipMethodRef, TrackingNum ---
    let sync_token = invoice["SyncToken"].as_str().unwrap();
    let update_body = serde_json::json!({
        "Id": inv_id,
        "SyncToken": sync_token,
        "sparse": true,
        "ShipDate": "2026-03-27",
        "TrackingNum": "1Z999AA10123456784",
        "ShipMethodRef": {"value": "FedEx"}
    });
    client
        .update_entity("Invoice", update_body)
        .await
        .expect("sparse update failed");
    eprintln!("Sparse updated invoice {} with shipping fields", inv_id);

    // --- 5. Re-read and verify shipping fields persisted ---
    let re_read = client
        .get_entity("Invoice", inv_id)
        .await
        .expect("re-read invoice failed");
    let ri = &re_read["Invoice"];
    assert_eq!(
        ri["ShipDate"].as_str(),
        Some("2026-03-27"),
        "ShipDate not persisted"
    );
    assert_eq!(
        ri["TrackingNum"].as_str(),
        Some("1Z999AA10123456784"),
        "TrackingNum not persisted"
    );
    assert!(
        ri["ShipMethodRef"]["value"].as_str().is_some(),
        "ShipMethodRef not persisted"
    );
    eprintln!(
        "Verified: ShipDate={}, TrackingNum={}, ShipMethodRef={}",
        ri["ShipDate"], ri["TrackingNum"], ri["ShipMethodRef"]["value"]
    );

    // --- 6. CDC endpoint ---
    let since = chrono::Utc::now() - chrono::Duration::hours(1);
    let cdc = client
        .cdc(&["Customer", "Invoice"], &since)
        .await
        .expect("CDC call failed");

    let cdc_response = cdc["CDCResponse"]
        .as_array()
        .expect("CDCResponse should be array");
    assert!(!cdc_response.is_empty(), "CDC should return entries");

    // Verify full entity payloads (not just IDs)
    let qrs = cdc_response[0]["QueryResponse"]
        .as_array()
        .expect("QueryResponse array");
    let has_entities = qrs.iter().any(|qr| {
        qr.as_object().map_or(false, |obj| {
            obj.keys()
                .any(|k| k == "Invoice" || k == "Customer")
        })
    });
    assert!(has_entities, "CDC should contain full entity payloads");
    eprintln!("CDC returned {} query responses with entity data", qrs.len());
}
