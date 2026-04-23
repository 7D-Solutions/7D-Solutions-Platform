//! Integration tests for the invoice push handler (bd-hzaar).
//!
//! DB-only tests (always run):
//!   - Superseded when authority version has advanced
//!   - Attempt reaches inflight then fails on bad endpoint
//!
//! QBO sandbox tests (require QBO_SANDBOX=1):
//!   - Create → update → void an invoice through push_invoice,
//!     verifying all three action paths write the expected DB state and
//!     return the correct QBO entity.
//!
//! Run DB-only:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_resolve_invoice_test
//!
//! Run with sandbox:
//!   QBO_SANDBOX=1 ./scripts/cargo-slot.sh test -p integrations-rs --test sync_resolve_invoice_test

use std::{path::PathBuf, sync::Arc, time::Duration};

use integrations_rs::domain::{
    qbo::{
        client::{QboClient, QboInvoicePayload, QboLineItem},
        QboError, TokenProvider,
    },
    sync::{
        push_attempts,
        resolve_invoice::{InvoiceAction, InvoicePushOutcome, InvoicePushRequest},
        resolve_service::ResolveService,
    },
};
use serde_json::Value;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use tokio::sync::{OnceCell, RwLock};
use uuid::Uuid;

// ── DB pool ───────────────────────────────────────────────────────────────────

static TEST_POOL: OnceCell<sqlx::PgPool> = OnceCell::const_new();

async fn init_pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run integrations migrations");
    pool
}

async fn setup_db() -> sqlx::PgPool {
    TEST_POOL.get_or_init(init_pool).await.clone()
}

fn unique_app() -> String {
    format!("rinv-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_sync_push_attempts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_sync_authority WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn seed_authority(pool: &sqlx::PgPool, app_id: &str, version: i64) {
    sqlx::query(
        r#"
        INSERT INTO integrations_sync_authority
            (app_id, provider, entity_type, authoritative_side, authority_version)
        VALUES ($1, 'quickbooks', 'invoice', 'platform', $2)
        ON CONFLICT (app_id, provider, entity_type)
        DO UPDATE SET authority_version = $2, updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind(version)
    .execute(pool)
    .await
    .expect("seed_authority");
}

fn minimal_payload(customer_ref: &str) -> QboInvoicePayload {
    QboInvoicePayload {
        customer_ref: customer_ref.to_string(),
        line_items: vec![QboLineItem {
            amount: 100.00,
            description: Some("Push test line".to_string()),
            item_ref: None,
        }],
        due_date: None,
        doc_number: None,
        currency_ref: None,
    }
}

// ── DB-only tests ─────────────────────────────────────────────────────────────

/// When the authority version has advanced since the push was enqueued,
/// push_invoice must record a `superseded` attempt and return Superseded
/// without dispatching any QBO call.
#[tokio::test]
#[serial]
async fn test_superseded_when_authority_advanced() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    seed_authority(&pool, &app_id, 2).await;

    let noop_client = Arc::new(QboClient::new(
        "http://127.0.0.1:0",
        "test-realm",
        Arc::new(NoopTokenProvider),
    ));
    let svc = ResolveService::new(noop_client);

    let req = InvoicePushRequest {
        app_id: app_id.clone(),
        entity_id: "inv-superseded".to_string(),
        authority_version: 1, // stale — authority is at 2
        request_id: Uuid::new_v4(),
        action: InvoiceAction::Create(minimal_payload("1")),
    };

    let outcome = integrations_rs::domain::sync::resolve_invoice::push_invoice(&pool, &svc, req)
        .await
        .expect("push_invoice");

    match outcome {
        InvoicePushOutcome::Superseded(row) => {
            assert_eq!(row.status, "superseded");
            assert_eq!(row.entity_type, "invoice");
            assert_eq!(row.operation, "create");
            assert_eq!(row.authority_version, 1);
            assert!(row.completed_at.is_some());
        }
        InvoicePushOutcome::Succeeded { .. } => panic!("expected Superseded but got Succeeded"),
    }

    let (rows, _) = push_attempts::list_attempts(&pool, &app_id, &Default::default(), 1, 10)
        .await
        .expect("list_attempts");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "superseded");

    cleanup(&pool, &app_id).await;
}

/// When authority version matches, push_invoice must transition through
/// accepted → inflight → failed when QBO is unreachable.
#[tokio::test]
#[serial]
async fn test_attempt_reaches_inflight_then_fails_on_bad_endpoint() {
    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    seed_authority(&pool, &app_id, 1).await;

    let bad_client = Arc::new(QboClient::new(
        "http://127.0.0.1:1", // connection refused
        "test-realm",
        Arc::new(StaticTokenProvider("token".into())),
    ));
    let svc = ResolveService::new(bad_client);

    let req = InvoicePushRequest {
        app_id: app_id.clone(),
        entity_id: "inv-network-fail".to_string(),
        authority_version: 1,
        request_id: Uuid::new_v4(),
        action: InvoiceAction::Create(minimal_payload("1")),
    };

    let result =
        integrations_rs::domain::sync::resolve_invoice::push_invoice(&pool, &svc, req).await;

    assert!(result.is_err(), "expected error from bad endpoint");

    let (rows, _) = push_attempts::list_attempts(&pool, &app_id, &Default::default(), 1, 10)
        .await
        .expect("list_attempts");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, "failed");
    assert!(rows[0].error_message.is_some());

    cleanup(&pool, &app_id).await;
}

// ── Sandbox tests ─────────────────────────────────────────────────────────────

fn skip_unless_sandbox() -> bool {
    std::env::var("QBO_SANDBOX").map_or(true, |v| v != "1")
}

/// Full E2E: create → update → void an invoice through push_invoice,
/// exercising all three action paths against a real Intuit sandbox tenant.
///
/// Runs as a single test to minimise token-refresh races and API call count.
#[tokio::test]
#[serial]
async fn test_sandbox_invoice_create_update_void() {
    if skip_unless_sandbox() {
        eprintln!("Skipping sandbox invoice test (set QBO_SANDBOX=1 to run)");
        return;
    }

    let pool = setup_db().await;
    let app_id = unique_app();
    cleanup(&pool, &app_id).await;

    let provider = Arc::new(SandboxTokenProvider::load());
    provider.refresh_token().await.expect("token refresh");

    let base_url = std::env::var("QBO_SANDBOX_BASE")
        .unwrap_or_else(|_| "https://sandbox-quickbooks.api.intuit.com/v3".into());
    let realm_id = provider.realm_id();
    let client = Arc::new(QboClient::new(&base_url, &realm_id, provider));
    let svc = ResolveService::new(client.clone());

    seed_authority(&pool, &app_id, 1).await;

    // Resolve a customer to attach the invoice to.
    let customer_id = resolve_sandbox_customer(&client).await;
    let item_ref = std::env::var("QBO_DEFAULT_ITEM_REF").unwrap_or_else(|_| "1".into());
    let doc_number = format!("PUSH-{}", &app_id[..8]);

    // ── Create ────────────────────────────────────────────────────────────────
    let payload = QboInvoicePayload {
        customer_ref: customer_id.clone(),
        line_items: vec![QboLineItem {
            amount: 75.50,
            description: Some(format!("Integration push test {}", &app_id[..8])),
            item_ref: Some(item_ref),
        }],
        due_date: Some("2026-12-31".to_string()),
        doc_number: Some(doc_number.clone()),
        currency_ref: None,
    };

    let create_req = InvoicePushRequest {
        app_id: app_id.clone(),
        entity_id: "inv-sandbox-1".to_string(),
        authority_version: 1,
        request_id: Uuid::new_v4(),
        action: InvoiceAction::Create(payload),
    };

    let outcome =
        integrations_rs::domain::sync::resolve_invoice::push_invoice(&pool, &svc, create_req)
            .await
            .expect("create invoice");

    let (qbo_id, sync_token) = match outcome {
        InvoicePushOutcome::Succeeded {
            attempt,
            qbo_entity,
        } => {
            assert_eq!(attempt.status, "succeeded");
            assert_eq!(attempt.operation, "create");
            let id = qbo_entity["Id"].as_str().expect("QBO Id").to_string();
            let st = qbo_entity["SyncToken"]
                .as_str()
                .expect("SyncToken")
                .to_string();
            eprintln!("Created QBO invoice Id={id} DocNumber={doc_number}");
            (id, st)
        }
        InvoicePushOutcome::Superseded(_) => panic!("create should not be superseded"),
    };

    // ── Update ────────────────────────────────────────────────────────────────
    let item_ref2 = std::env::var("QBO_DEFAULT_ITEM_REF").unwrap_or_else(|_| "1".into());
    let updated_payload = QboInvoicePayload {
        customer_ref: customer_id.clone(),
        line_items: vec![QboLineItem {
            amount: 125.00,
            description: Some(format!("Updated push test {}", &app_id[..8])),
            item_ref: Some(item_ref2),
        }],
        due_date: Some("2026-12-31".to_string()),
        doc_number: Some(doc_number.clone()),
        currency_ref: None,
    };

    let update_req = InvoicePushRequest {
        app_id: app_id.clone(),
        entity_id: "inv-sandbox-1".to_string(),
        authority_version: 1,
        request_id: Uuid::new_v4(),
        action: InvoiceAction::Update {
            qbo_id: qbo_id.clone(),
            sync_token: sync_token.clone(),
            payload: updated_payload,
        },
    };

    let outcome =
        integrations_rs::domain::sync::resolve_invoice::push_invoice(&pool, &svc, update_req)
            .await
            .expect("update invoice");

    let sync_token_after_update = match outcome {
        InvoicePushOutcome::Succeeded {
            attempt,
            qbo_entity,
        } => {
            assert_eq!(attempt.status, "succeeded");
            assert_eq!(attempt.operation, "update");
            eprintln!("Updated QBO invoice Id={qbo_id}");
            qbo_entity["SyncToken"]
                .as_str()
                .expect("SyncToken")
                .to_string()
        }
        InvoicePushOutcome::Superseded(_) => panic!("update should not be superseded"),
    };

    // ── Void ──────────────────────────────────────────────────────────────────
    let void_req = InvoicePushRequest {
        app_id: app_id.clone(),
        entity_id: "inv-sandbox-1".to_string(),
        authority_version: 1,
        request_id: Uuid::new_v4(),
        action: InvoiceAction::Void {
            qbo_id: qbo_id.clone(),
            sync_token: sync_token_after_update,
        },
    };

    let outcome =
        integrations_rs::domain::sync::resolve_invoice::push_invoice(&pool, &svc, void_req)
            .await
            .expect("void invoice");

    match outcome {
        InvoicePushOutcome::Succeeded {
            attempt,
            qbo_entity,
        } => {
            assert_eq!(attempt.status, "succeeded");
            assert_eq!(attempt.operation, "void");
            // Voided invoices have Balance=0.
            let balance = qbo_entity["Balance"].as_f64().unwrap_or(1.0);
            assert_eq!(balance, 0.0, "voided invoice must have Balance=0");
            eprintln!("Voided QBO invoice Id={qbo_id}");
        }
        InvoicePushOutcome::Superseded(_) => panic!("void should not be superseded"),
    }

    // Verify all three attempts are in the DB as succeeded.
    let (rows, total) = push_attempts::list_attempts(&pool, &app_id, &Default::default(), 1, 10)
        .await
        .expect("list_attempts");
    assert_eq!(total, 3, "three push attempts must be recorded");
    assert!(
        rows.iter().all(|r| r.status == "succeeded"),
        "all attempts must be succeeded; got: {:?}",
        rows.iter().map(|r| &r.status).collect::<Vec<_>>()
    );

    cleanup(&pool, &app_id).await;
}

// ── Token provider helpers ────────────────────────────────────────────────────

struct NoopTokenProvider;

#[async_trait::async_trait]
impl TokenProvider for NoopTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        panic!("NoopTokenProvider: QBO was called unexpectedly");
    }
    async fn refresh_token(&self) -> Result<String, QboError> {
        panic!("NoopTokenProvider: QBO was called unexpectedly");
    }
}

struct StaticTokenProvider(String);

#[async_trait::async_trait]
impl TokenProvider for StaticTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        Ok(self.0.clone())
    }
    async fn refresh_token(&self) -> Result<String, QboError> {
        Ok(self.0.clone())
    }
}

// ── Sandbox helpers ───────────────────────────────────────────────────────────

/// Fetch the first active customer Id from the sandbox.
async fn resolve_sandbox_customer(client: &QboClient) -> String {
    let customers = client
        .query_all("SELECT * FROM Customer WHERE Active = true", "Customer")
        .await
        .expect("query customers");
    customers
        .first()
        .and_then(|c| c["Id"].as_str())
        .expect("at least one active customer in sandbox")
        .to_string()
}

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
            return Err(QboError::TokenError(format!("Refresh failed: {body}")));
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

        if let Ok(content) = std::fs::read_to_string(&self.tokens_path) {
            if let Ok(mut existing) = serde_json::from_str::<Value>(&content) {
                existing["access_token"] = Value::String(new_at.clone());
                existing["refresh_token"] = Value::String(new_rt);
                let _ = std::fs::write(
                    &self.tokens_path,
                    serde_json::to_string_pretty(&existing).unwrap(),
                );
            }
        }

        Ok(new_at)
    }
}
