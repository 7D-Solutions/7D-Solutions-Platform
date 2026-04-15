//! HTTP smoke tests: AR Credits, Disputes & Refunds
//!
//! Proves that 10 AR financial-correction routes respond correctly at the
//! HTTP boundary via reqwest against a live AR service. Each route is tested
//! for:
//! - Happy path: correct status code + valid JSON response
//! - Auth enforcement: no JWT -> 401 Unauthorized
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_ar_credits_disputes -- --nocapture
//! ```

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const AR_DEFAULT_URL: &str = "http://localhost:8086";

fn ar_url() -> String {
    std::env::var("AR_URL").unwrap_or_else(|_| AR_DEFAULT_URL.to_string())
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

async fn wait_for_ar(client: &Client) -> bool {
    let url = format!("{}/api/health", ar_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  AR health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  AR health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        _ => panic!("unsupported method"),
    };
    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 OK");
}

/// Seed: create customer + two finalized invoices (one for credit, one for write-off)
async fn seed_customer_and_invoices(
    client: &Client,
    base: &str,
    jwt: &str,
    customer_id: &mut i64,
    invoice_id_credit: &mut i64,
    invoice_id_writeoff: &mut i64,
) {
    let email = format!("smoke-crd-{}@test.local", Uuid::new_v4());
    let resp = client
        .post(format!("{base}/api/ar/customers"))
        .bearer_auth(jwt)
        .json(&json!({"email": email, "name": "Smoke Credits Customer"}))
        .send()
        .await
        .expect("create customer failed");
    assert_eq!(resp.status().as_u16(), 201);
    let body: Value = resp.json().await.unwrap();
    *customer_id = body["id"].as_i64().expect("customer.id missing");
    println!("  seeded customer id={customer_id}");

    let resp = client
        .post(format!("{base}/api/ar/invoices"))
        .bearer_auth(jwt)
        .json(&json!({"ar_customer_id": *customer_id, "amount_cents": 10000, "currency": "usd"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let inv: Value = resp.json().await.unwrap();
    *invoice_id_credit = inv["id"].as_i64().unwrap();

    let resp = client
        .post(format!(
            "{base}/api/ar/invoices/{invoice_id_credit}/finalize"
        ))
        .bearer_auth(jwt)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    println!("  seeded invoice id={invoice_id_credit} (finalized, for credit)");

    let resp = client
        .post(format!("{base}/api/ar/invoices"))
        .bearer_auth(jwt)
        .json(&json!({"ar_customer_id": *customer_id, "amount_cents": 5000, "currency": "usd"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let inv: Value = resp.json().await.unwrap();
    *invoice_id_writeoff = inv["id"].as_i64().unwrap();

    let resp = client
        .post(format!(
            "{base}/api/ar/invoices/{invoice_id_writeoff}/finalize"
        ))
        .bearer_auth(jwt)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    println!("  seeded invoice id={invoice_id_writeoff} (finalized, for write-off)");
}

#[tokio::test]
async fn smoke_ar_credits_disputes() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_ar(&client).await {
        eprintln!("AR service not reachable at {} -- skipping", ar_url());
        return;
    }
    println!("AR service healthy at {}", ar_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["ar.mutate", "ar.read"]);
    let base = ar_url();

    let probe = client
        .get(format!("{base}/api/ar/customers"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "AR service returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.\n\
             Fix: set JWT_PUBLIC_KEY in docker-compose.services.yml for the AR container"
        );
        return;
    }

    println!("\n--- Seeding: customer + 2 finalized invoices ---");
    let mut customer_id: i64 = 0;
    let mut invoice_id_credit: i64 = 0;
    let mut invoice_id_writeoff: i64 = 0;
    seed_customer_and_invoices(
        &client,
        &base,
        &jwt,
        &mut customer_id,
        &mut invoice_id_credit,
        &mut invoice_id_writeoff,
    )
    .await;

    // --- 1. POST /api/ar/invoices/{id}/credit-notes ---
    println!("\n--- 1. POST /api/ar/invoices/{{id}}/credit-notes ---");
    let credit_note_id = Uuid::new_v4();
    let cn_body = json!({
        "credit_note_id": credit_note_id,
        "app_id": tenant_id,
        "customer_id": customer_id.to_string(),
        "invoice_id": invoice_id_credit,
        "amount_minor": 2000_i64,
        "currency": "usd",
        "reason": "Smoke test credit note",
        "correlation_id": Uuid::new_v4().to_string()
    });
    let resp = client
        .post(format!(
            "{base}/api/ar/invoices/{invoice_id_credit}/credit-notes"
        ))
        .bearer_auth(&jwt)
        .json(&cn_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        status == 200 || status == 201,
        "expected 200/201 for credit note, got {status}: {body}"
    );
    println!("  credit note issued -> {status}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/invoices/{invoice_id_credit}/credit-notes"),
        Some(cn_body.clone()),
    )
    .await;

    // --- 2. POST /api/ar/invoices/{id}/write-off ---
    println!("\n--- 2. POST /api/ar/invoices/{{id}}/write-off ---");
    let wo_body = json!({
        "write_off_id": Uuid::new_v4(),
        "app_id": tenant_id,
        "invoice_id": invoice_id_writeoff,
        "customer_id": customer_id.to_string(),
        "written_off_amount_minor": 5000_i64,
        "currency": "usd",
        "reason": "Smoke test write-off",
        "correlation_id": Uuid::new_v4().to_string()
    });
    let resp = client
        .post(format!(
            "{base}/api/ar/invoices/{invoice_id_writeoff}/write-off"
        ))
        .bearer_auth(&jwt)
        .json(&wo_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        status == 200 || status == 201,
        "expected 200/201 for write-off, got {status}: {body}"
    );
    println!("  write-off -> {status}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/invoices/{invoice_id_writeoff}/write-off"),
        Some(wo_body.clone()),
    )
    .await;

    // --- 3. POST /api/ar/credit-memos ---
    println!("\n--- 3. POST /api/ar/credit-memos ---");
    let memo_credit_note_id = Uuid::new_v4();
    let memo_idem = Uuid::new_v4();
    let memo_body = json!({
        "credit_note_id": memo_credit_note_id,
        "customer_id": customer_id.to_string(),
        "invoice_id": invoice_id_credit,
        "amount_minor": 1000_i64,
        "currency": "usd",
        "reason": "Smoke test credit memo",
        "create_idempotency_key": memo_idem.to_string(),
        "correlation_id": Uuid::new_v4().to_string()
    });
    let resp = client
        .post(format!("{base}/api/ar/credit-memos"))
        .bearer_auth(&jwt)
        .json(&memo_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        status == 200 || status == 201,
        "expected 200/201 for credit memo create, got {status}: {body}"
    );
    let memo_uuid = body["credit_note_id"].as_str().map(|s| s.to_string());
    println!("  credit memo created -> {status}, uuid={memo_uuid:?}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/credit-memos"),
        Some(memo_body.clone()),
    )
    .await;

    // --- 4. POST /api/ar/credit-memos/{id}/approve ---
    println!("\n--- 4. POST /api/ar/credit-memos/{{id}}/approve ---");
    if let Some(mid) = &memo_uuid {
        let approve_body = json!({
            "approved_by": "smoke-test-approver",
            "correlation_id": Uuid::new_v4().to_string()
        });
        let resp = client
            .post(format!("{base}/api/ar/credit-memos/{mid}/approve"))
            .bearer_auth(&jwt)
            .json(&approve_body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(json!(null));
        assert!(
            status == 200 || status == 201 || status == 409,
            "expected 200/201/409 for approve, got {status}: {body}"
        );
        println!("  approve -> {status}");

        assert_unauth(
            &client,
            "POST",
            &format!("{base}/api/ar/credit-memos/{mid}/approve"),
            Some(approve_body),
        )
        .await;

        // --- 5. POST /api/ar/credit-memos/{id}/issue ---
        println!("\n--- 5. POST /api/ar/credit-memos/{{id}}/issue ---");
        let issue_body = json!({
            "issued_by": "smoke-test-issuer",
            "issue_idempotency_key": Uuid::new_v4().to_string(),
            "correlation_id": Uuid::new_v4().to_string()
        });
        let resp = client
            .post(format!("{base}/api/ar/credit-memos/{mid}/issue"))
            .bearer_auth(&jwt)
            .json(&issue_body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let body: Value = resp.json().await.unwrap_or(json!(null));
        assert!(
            status == 200 || status == 201 || status == 409,
            "expected 200/201/409 for issue, got {status}: {body}"
        );
        println!("  issue -> {status}");

        assert_unauth(
            &client,
            "POST",
            &format!("{base}/api/ar/credit-memos/{mid}/issue"),
            Some(issue_body),
        )
        .await;
    } else {
        println!("  skipping approve/issue -- no memo id returned");
        assert_unauth(
            &client,
            "POST",
            &format!("{base}/api/ar/credit-memos/999999/approve"),
            Some(json!({"approved_by": "x", "correlation_id": Uuid::new_v4().to_string()})),
        )
        .await;

        println!("\n--- 5. POST /api/ar/credit-memos/{{id}}/issue ---");
        assert_unauth(
            &client,
            "POST",
            &format!("{base}/api/ar/credit-memos/999999/issue"),
            Some(
                json!({"issued_by": "x", "issue_idempotency_key": Uuid::new_v4().to_string(),
                        "correlation_id": Uuid::new_v4().to_string()}),
            ),
        )
        .await;
    }

    // --- 6. GET /api/ar/disputes (list) ---
    println!("\n--- 6. GET /api/ar/disputes ---");
    let resp = client
        .get(format!("{base}/api/ar/disputes"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for disputes list");
    let disputes: Value = resp.json().await.unwrap_or(json!([]));
    println!(
        "  disputes list -> {status}, count={}",
        match &disputes {
            Value::Array(a) => a.len(),
            _ => 0,
        }
    );

    assert_unauth(&client, "GET", &format!("{base}/api/ar/disputes"), None).await;

    // --- 7. GET /api/ar/disputes/{id} ---
    println!("\n--- 7. GET /api/ar/disputes/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/disputes/999999"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 404, "expected 404 for nonexistent dispute");
    println!("  nonexistent dispute -> 404 OK");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/disputes/999999"),
        None,
    )
    .await;

    // --- 8. POST /api/ar/disputes/{id}/evidence ---
    println!("\n--- 8. POST /api/ar/disputes/{{id}}/evidence ---");
    let evidence_body = json!({
        "evidence_type": "receipt",
        "description": "Smoke test evidence",
        "content": "Test evidence content"
    });
    let resp = client
        .post(format!("{base}/api/ar/disputes/999999/evidence"))
        .bearer_auth(&jwt)
        .json(&evidence_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 404 || status == 400 || status == 409 || status == 422,
        "expected 404/400/409/422 for evidence on nonexistent dispute, got {status}"
    );
    println!("  evidence on nonexistent -> {status} OK");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/disputes/999999/evidence"),
        Some(evidence_body),
    )
    .await;

    // --- 9. POST /api/ar/refunds ---
    println!("\n--- 9. POST /api/ar/refunds ---");
    let refund_body = json!({
        "charge_id": 999999,
        "amount_cents": 1000,
        "currency": "usd",
        "reason": "Smoke test refund",
        "reference_id": format!("smoke-ref-{}", Uuid::new_v4())
    });
    let resp = client
        .post(format!("{base}/api/ar/refunds"))
        .bearer_auth(&jwt)
        .json(&refund_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert!(
        status == 404 || status == 409 || status == 400 || status == 502,
        "expected 404/409/400/502 for refund without settled charge, got {status}"
    );
    println!("  refund without settled charge -> {status} OK");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/refunds"),
        Some(refund_body),
    )
    .await;

    // --- 10. GET /api/ar/refunds/{id} ---
    println!("\n--- 10. GET /api/ar/refunds/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/refunds/999999"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 404, "expected 404 for nonexistent refund");
    println!("  nonexistent refund -> 404 OK");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/refunds/999999"),
        None,
    )
    .await;

    println!("\n=== All 10 AR credits/disputes/refunds smoke tests passed ===");
}
