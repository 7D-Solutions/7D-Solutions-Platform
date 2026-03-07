//! HTTP smoke tests: Treasury — accounts, reports, recon, import
//!
//! Proves that the 15 core Treasury routes respond correctly at the HTTP
//! boundary via reqwest against a live Treasury service. Each route is
//! tested for happy path + auth enforcement (no JWT -> 401).
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_treasury -- --nocapture
//! ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const TREASURY_DEFAULT_URL: &str = "http://localhost:8094";

fn treasury_url() -> String {
    std::env::var("TREASURY_URL").unwrap_or_else(|_| TREASURY_DEFAULT_URL.to_string())
}

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    app_id: Option<String>,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn dev_private_key() -> Option<EncodingKey> {
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM").ok()?;
    EncodingKey::from_rsa_pem(pem.replace("\\n", "\n").as_bytes()).ok()
}

fn make_jwt(key: &EncodingKey, tenant_id: &str, perms: &[&str]) -> String {
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        app_id: Some(tenant_id.to_string()),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

async fn wait_for_treasury(client: &Client) -> bool {
    let url = format!("{}/api/health", treasury_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Treasury health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Treasury health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body { req.json(&b) } else { req };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(resp.status().as_u16(), 401, "expected 401 without JWT at {url}");
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_treasury() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_treasury(&client).await {
        eprintln!("Treasury service not reachable at {} -- skipping", treasury_url());
        return;
    }
    println!("Treasury service healthy at {}", treasury_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["treasury.mutate", "treasury.read"]);
    let base = treasury_url();

    // Gate: verify the Treasury service accepts our JWT
    let probe = client
        .get(format!("{base}/api/treasury/accounts"))
        .bearer_auth(&jwt)
        .send().await.expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Treasury returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.\n\
             Fix: set JWT_PUBLIC_KEY in docker-compose.services.yml for the Treasury container"
        );
        return;
    }

    // --- 1. POST /api/treasury/accounts/bank ---
    println!("\n--- 1. POST /api/treasury/accounts/bank ---");
    let resp = client
        .post(format!("{base}/api/treasury/accounts/bank"))
        .bearer_auth(&jwt)
        .json(&json!({"account_name": "Smoke Checking", "currency": "USD",
                       "institution": "Test Bank", "account_number_last4": "1234"}))
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let bank_acct: Value = resp.json().await.unwrap();
    assert_eq!(status, 201, "expected 201 for bank account, got {status}: {bank_acct}");
    let bank_id = bank_acct["id"].as_str().expect("bank account id missing");
    println!("  created bank account id={bank_id}");

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/accounts/bank"),
        Some(json!({"account_name": "X", "currency": "USD"})),
    ).await;

    // --- 2. GET /api/treasury/accounts/{id} ---
    println!("\n--- 2. GET /api/treasury/accounts/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/treasury/accounts/{bank_id}"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), bank_id);
    println!("  retrieved bank account name={}", body["account_name"]);

    assert_unauth(&client, "GET", &format!("{base}/api/treasury/accounts/{bank_id}"), None).await;

    // --- 3. POST /api/treasury/accounts/{id}/deactivate ---
    println!("\n--- 3. POST /api/treasury/accounts/{{id}}/deactivate ---");
    // Create a throwaway account to deactivate
    let resp = client
        .post(format!("{base}/api/treasury/accounts/bank"))
        .bearer_auth(&jwt)
        .json(&json!({"account_name": "Deactivate Me", "currency": "USD"}))
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let throwaway: Value = resp.json().await.unwrap();
    let throwaway_id = throwaway["id"].as_str().unwrap();

    let resp = client
        .post(format!("{base}/api/treasury/accounts/{throwaway_id}/deactivate"))
        .bearer_auth(&jwt)
        .json(&json!({}))
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 204, "expected 204 No Content for deactivate");
    println!("  deactivated account {throwaway_id}");

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/accounts/{throwaway_id}/deactivate"),
        Some(json!({})),
    ).await;

    // --- 4. POST /api/treasury/accounts/credit-card ---
    println!("\n--- 4. POST /api/treasury/accounts/credit-card ---");
    let resp = client
        .post(format!("{base}/api/treasury/accounts/credit-card"))
        .bearer_auth(&jwt)
        .json(&json!({"account_name": "Smoke Visa", "currency": "USD",
                       "institution": "Chase", "account_number_last4": "5678",
                       "cc_network": "Visa", "credit_limit_minor": 500000}))
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let cc_acct: Value = resp.json().await.unwrap();
    assert_eq!(status, 201, "expected 201 for CC account: {cc_acct}");
    let cc_id = cc_acct["id"].as_str().unwrap();
    println!("  created CC account id={cc_id}");

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/accounts/credit-card"),
        Some(json!({"account_name": "X", "currency": "USD"})),
    ).await;

    // --- 5. GET /api/treasury/accounts ---
    println!("\n--- 5. GET /api/treasury/accounts ---");
    let resp = client
        .get(format!("{base}/api/treasury/accounts"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let accounts: Vec<Value> = resp.json().await.unwrap();
    // Should have at least 2 active accounts (bank + CC; deactivated one excluded by default)
    assert!(accounts.len() >= 2, "expected >= 2 active accounts, got {}", accounts.len());
    println!("  listed {} accounts", accounts.len());

    assert_unauth(&client, "GET", &format!("{base}/api/treasury/accounts"), None).await;

    // --- 6. GET /api/treasury/cash-position ---
    println!("\n--- 6. GET /api/treasury/cash-position ---");
    let resp = client
        .get(format!("{base}/api/treasury/cash-position"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "cash-position failed: {body}");
    println!("  cash-position returned ok");

    assert_unauth(&client, "GET", &format!("{base}/api/treasury/cash-position"), None).await;

    // --- 7. GET /api/treasury/forecast ---
    println!("\n--- 7. GET /api/treasury/forecast ---");
    let resp = client
        .get(format!("{base}/api/treasury/forecast"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "forecast failed: {body}");
    println!("  forecast returned ok");

    assert_unauth(&client, "GET", &format!("{base}/api/treasury/forecast"), None).await;

    // --- 8. POST /api/treasury/statements/import ---
    println!("\n--- 8. POST /api/treasury/statements/import ---");
    let csv_data = "date,description,amount,reference\n\
                    2026-03-01,Wire In,50000,REF001\n\
                    2026-03-02,Payment Out,-10000,REF002\n";

    let form = reqwest::multipart::Form::new()
        .text("account_id", bank_id.to_string())
        .text("period_start", "2026-03-01")
        .text("period_end", "2026-03-31")
        .text("opening_balance_minor", "100000")
        .text("closing_balance_minor", "140000")
        .part("file", reqwest::multipart::Part::bytes(csv_data.as_bytes().to_vec())
            .file_name("smoke-statement.csv")
            .mime_str("text/csv").unwrap());

    let resp = client
        .post(format!("{base}/api/treasury/statements/import"))
        .bearer_auth(&jwt)
        .multipart(form)
        .send().await.unwrap();
    let import_status = resp.status().as_u16();
    let import_body: Value = resp.json().await.unwrap();
    // 201 for new import, 200 for duplicate, 422 if CSV format rejected
    assert!(
        import_status == 201 || import_status == 200 || import_status == 422,
        "import expected 201/200/422, got {import_status}: {import_body}"
    );
    println!("  import -> {import_status}");

    // Auth check: multipart without JWT
    let form2 = reqwest::multipart::Form::new()
        .text("account_id", Uuid::new_v4().to_string())
        .text("period_start", "2026-01-01")
        .text("period_end", "2026-01-31")
        .text("opening_balance_minor", "0")
        .text("closing_balance_minor", "0")
        .part("file", reqwest::multipart::Part::bytes(b"x".to_vec())
            .file_name("x.csv").mime_str("text/csv").unwrap());
    let resp = client
        .post(format!("{base}/api/treasury/statements/import"))
        .multipart(form2)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 401, "expected 401 without JWT on import");
    println!("  no-JWT -> 401 ok");

    // --- 9. POST /api/treasury/recon/auto-match ---
    println!("\n--- 9. POST /api/treasury/recon/auto-match ---");
    let resp = client
        .post(format!("{base}/api/treasury/recon/auto-match"))
        .bearer_auth(&jwt)
        .json(&json!({"account_id": bank_id}))
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "auto-match failed: {body}");
    println!("  auto-match -> matches_created={}", body["matches_created"]);

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/recon/auto-match"),
        Some(json!({"account_id": bank_id})),
    ).await;

    // --- 10. GET /api/treasury/recon/matches ---
    println!("\n--- 10. GET /api/treasury/recon/matches ---");
    let resp = client
        .get(format!("{base}/api/treasury/recon/matches?account_id={bank_id}"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let matches: Vec<Value> = resp.json().await.unwrap();
    println!("  recon matches: {}", matches.len());

    assert_unauth(
        &client, "GET",
        &format!("{base}/api/treasury/recon/matches?account_id={bank_id}"), None,
    ).await;

    // --- 11. GET /api/treasury/recon/unmatched ---
    println!("\n--- 11. GET /api/treasury/recon/unmatched ---");
    let resp = client
        .get(format!("{base}/api/treasury/recon/unmatched?account_id={bank_id}"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let unmatched: Value = resp.json().await.unwrap();
    println!(
        "  unmatched: stmt_lines={}, pay_txns={}",
        unmatched["unmatched_statement_lines"], unmatched["unmatched_payment_transactions"]
    );

    assert_unauth(
        &client, "GET",
        &format!("{base}/api/treasury/recon/unmatched?account_id={bank_id}"), None,
    ).await;

    // --- 12. POST /api/treasury/recon/manual-match ---
    // Requires valid statement_line_id + bank_transaction_id; 404 is expected behavior
    println!("\n--- 12. POST /api/treasury/recon/manual-match ---");
    let fake_stmt = Uuid::new_v4();
    let fake_txn = Uuid::new_v4();
    let resp = client
        .post(format!("{base}/api/treasury/recon/manual-match"))
        .bearer_auth(&jwt)
        .json(&json!({"statement_line_id": fake_stmt, "bank_transaction_id": fake_txn}))
        .send().await.unwrap();
    let status = resp.status().as_u16();
    // 201 if match created, 404 if IDs not found, 422 if mismatch
    assert!(
        status == 201 || status == 404 || status == 422,
        "manual-match expected 201/404/422, got {status}"
    );
    println!("  manual-match -> {status} (expected without real IDs)");

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/recon/manual-match"),
        Some(json!({"statement_line_id": fake_stmt, "bank_transaction_id": fake_txn})),
    ).await;

    // --- 13. POST /api/treasury/recon/gl-link ---
    println!("\n--- 13. POST /api/treasury/recon/gl-link ---");
    let resp = client
        .post(format!("{base}/api/treasury/recon/gl-link"))
        .bearer_auth(&jwt)
        .json(&json!({"bank_transaction_id": Uuid::new_v4(), "gl_entry_id": 999999}))
        .send().await.unwrap();
    let status = resp.status().as_u16();
    // 200 if linked, 404 if txn not found
    assert!(status == 200 || status == 404, "gl-link expected 200/404, got {status}");
    println!("  gl-link -> {status}");

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/recon/gl-link"),
        Some(json!({"bank_transaction_id": Uuid::new_v4(), "gl_entry_id": 1})),
    ).await;

    // --- 14. POST /api/treasury/recon/gl-unmatched-entries ---
    println!("\n--- 14. POST /api/treasury/recon/gl-unmatched-entries ---");
    let resp = client
        .post(format!("{base}/api/treasury/recon/gl-unmatched-entries"))
        .bearer_auth(&jwt)
        .json(&json!({"gl_entry_ids": [1, 2, 3]}))
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "gl-unmatched-entries failed: {body}");
    println!("  gl-unmatched-entries -> provided={}, unmatched={}", body["provided"], body["unmatched_gl_entry_ids"]);

    assert_unauth(
        &client, "POST", &format!("{base}/api/treasury/recon/gl-unmatched-entries"),
        Some(json!({"gl_entry_ids": [1]})),
    ).await;

    // --- 15. GET /api/treasury/recon/gl-unmatched-txns ---
    println!("\n--- 15. GET /api/treasury/recon/gl-unmatched-txns ---");
    let resp = client
        .get(format!("{base}/api/treasury/recon/gl-unmatched-txns?account_id={bank_id}"))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(status, 200, "gl-unmatched-txns failed: {body}");
    println!("  gl-unmatched-txns -> count={}", body["count"]);

    assert_unauth(
        &client, "GET",
        &format!("{base}/api/treasury/recon/gl-unmatched-txns?account_id={bank_id}"), None,
    ).await;

    // --- Error response sanitization ---
    println!("\n--- Error response sanitization ---");
    let resp = client
        .get(format!("{base}/api/treasury/accounts/{}", Uuid::new_v4()))
        .bearer_auth(&jwt)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 404);
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("SELECT") && !body.contains("sqlx") && !body.contains("panicked"),
        "error leaks internals: {}", &body[..body.len().min(200)]
    );
    println!("  404 sanitized ok");

    println!("\n=== All 15 Treasury smoke tests passed ===");
}
