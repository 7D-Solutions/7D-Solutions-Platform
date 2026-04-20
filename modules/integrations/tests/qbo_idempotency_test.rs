//! QBO requestid idempotency and entity-creation integration tests (bd-hv1e2).
//!
//! Verifies that repeated create calls with the same `request_id` are treated
//! as idempotent by the Intuit sandbox and do NOT create duplicate entities.
//!
//! Run: QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs --test qbo_idempotency_test --nocapture
//!
//! Requires:
//! - `.env.qbo-sandbox` with QBO_CLIENT_ID, QBO_CLIENT_SECRET, QBO_SANDBOX_BASE
//! - `.qbo-tokens.json` with access_token, refresh_token, realm_id

use integrations_rs::domain::qbo::{
    client::{QboClient, QboCustomerPayload, QboInvoicePayload, QboLineItem, QboPaymentPayload},
    QboError, TokenProvider,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ============================================================================
// Sandbox token provider (self-contained copy)
// ============================================================================

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
        dotenvy::from_path(root.join(".env.qbo-sandbox")).expect(".env.qbo-sandbox");
        let client_id = std::env::var("QBO_CLIENT_ID").expect("QBO_CLIENT_ID");
        let client_secret = std::env::var("QBO_CLIENT_SECRET").expect("QBO_CLIENT_SECRET");
        let tokens_path = root.join(".qbo-tokens.json");
        let content = std::fs::read_to_string(&tokens_path).expect(".qbo-tokens.json");
        let tokens: Value = serde_json::from_str(&content).expect("parse tokens");
        Self {
            access_token: RwLock::new(tokens["access_token"].as_str().expect("access_token").into()),
            refresh_tok: RwLock::new(tokens["refresh_token"].as_str().expect("refresh_token").into()),
            client_id,
            client_secret,
            http: reqwest::Client::new(),
            tokens_path,
        }
    }

    fn realm_id(&self) -> String {
        let content = std::fs::read_to_string(&self.tokens_path).expect("tokens");
        let t: Value = serde_json::from_str(&content).expect("parse");
        t["realm_id"].as_str().expect("realm_id").to_string()
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

        let tr: Value = resp.json().await.map_err(|e| QboError::TokenError(e.to_string()))?;
        let new_at = tr["access_token"].as_str().ok_or_else(|| QboError::TokenError("no access_token".into()))?.to_string();
        let new_rt = tr["refresh_token"].as_str().ok_or_else(|| QboError::TokenError("no refresh_token".into()))?.to_string();

        *self.access_token.write().await = new_at.clone();
        *self.refresh_tok.write().await = new_rt.clone();

        if let Ok(content) = std::fs::read_to_string(&self.tokens_path) {
            if let Ok(mut existing) = serde_json::from_str::<Value>(&content) {
                existing["access_token"] = Value::String(new_at.clone());
                existing["refresh_token"] = Value::String(new_rt);
                let _ = std::fs::write(&self.tokens_path, serde_json::to_string_pretty(&existing).unwrap());
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

// ============================================================================
// Tests
// ============================================================================

/// Verify that repeating a `create_invoice` call with the same `request_id`
/// returns the same QBO entity (Intuit idempotency, no duplicate created).
#[tokio::test]
async fn create_invoice_same_requestid_is_idempotent() {
    if skip_unless_sandbox() {
        eprintln!("Skipping QBO idempotency test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let (client, provider) = make_client();
    provider.refresh_token().await.expect("token refresh failed");

    // Find a customer to attach the invoice to
    let cust_resp = client
        .query("SELECT * FROM Customer MAXRESULTS 1")
        .await
        .expect("customer query failed");
    let cust_id = cust_resp["QueryResponse"]["Customer"][0]["Id"]
        .as_str()
        .expect("no customers in sandbox");

    let rid = Uuid::new_v4();
    let item_ref = std::env::var("QBO_DEFAULT_ITEM_REF").unwrap_or_else(|_| "1".into());

    let payload = QboInvoicePayload {
        customer_ref: cust_id.to_string(),
        line_items: vec![QboLineItem {
            amount: 1.00,
            description: Some(format!("Idempotency test {}", rid)),
            item_ref: Some(item_ref),
        }],
        due_date: None,
        doc_number: Some(format!("IDEM-{}", &rid.to_string()[..8])),
    };

    // First call
    let inv1 = client
        .create_invoice(&payload, rid)
        .await
        .expect("first create_invoice failed");
    let id1 = inv1["Id"].as_str().expect("first invoice Id");
    eprintln!("First create_invoice: Id={}", id1);

    // Second call with the SAME request_id — must return the same entity
    let inv2 = client
        .create_invoice(&payload, rid)
        .await
        .expect("second create_invoice failed");
    let id2 = inv2["Id"].as_str().expect("second invoice Id");
    eprintln!("Second create_invoice: Id={}", id2);

    assert_eq!(
        id1, id2,
        "same requestid must return the same QBO invoice (idempotency broken: {} vs {})",
        id1, id2
    );
}

/// Verify `create_customer` creates a real QBO customer and returns an Id.
#[tokio::test]
async fn create_customer_returns_real_id() {
    if skip_unless_sandbox() {
        eprintln!("Skipping (set QBO_SANDBOX=1)");
        return;
    }

    let (client, provider) = make_client();
    provider.refresh_token().await.expect("token refresh failed");

    let ts = chrono::Utc::now().timestamp();
    let payload = QboCustomerPayload {
        display_name: format!("Idempotency Test Customer {}", ts),
        email: Some("idem-test@example.com".into()),
        company_name: Some("Idem Test Corp".into()),
        currency_ref: None,
    };

    let rid = Uuid::new_v4();
    let customer = client
        .create_customer(&payload, rid)
        .await
        .expect("create_customer failed");

    let cust_id = customer["Id"].as_str().expect("Customer Id missing");
    eprintln!("Created customer Id={}", cust_id);

    // Verify it round-trips
    let re = client
        .get_entity("Customer", cust_id)
        .await
        .expect("get_entity failed");
    assert_eq!(re["Customer"]["Id"].as_str(), Some(cust_id));
    eprintln!("Verified customer round-trip OK");
}

/// Verify `create_payment` creates a real QBO payment and returns an Id.
#[tokio::test]
async fn create_payment_returns_real_id() {
    if skip_unless_sandbox() {
        eprintln!("Skipping (set QBO_SANDBOX=1)");
        return;
    }

    let (client, provider) = make_client();
    provider.refresh_token().await.expect("token refresh failed");

    // Get a customer with an open invoice balance to apply payment against
    let cust_resp = client
        .query("SELECT * FROM Customer MAXRESULTS 1")
        .await
        .expect("customer query");
    let cust_id = cust_resp["QueryResponse"]["Customer"][0]["Id"]
        .as_str()
        .expect("no customers");

    let payload = QboPaymentPayload {
        customer_ref: cust_id.to_string(),
        total_amount: 0.01,
        txn_date: Some(chrono::Utc::now().format("%Y-%m-%d").to_string()),
        currency_ref: None,
        payment_method_ref: None,
        deposit_to_account_ref: None,
    };

    let rid = Uuid::new_v4();
    let payment = client
        .create_payment(&payload, rid)
        .await
        .expect("create_payment failed");

    let pay_id = payment["Id"].as_str().expect("Payment Id missing");
    eprintln!("Created payment Id={}", pay_id);

    // Verify round-trip
    let re = client
        .get_entity("Payment", pay_id)
        .await
        .expect("get_entity Payment failed");
    assert_eq!(re["Payment"]["Id"].as_str(), Some(pay_id));
    eprintln!("Verified payment round-trip OK");
}
