//! HTTP smoke tests: AR Customer + Invoice CRUD
//!
//! Proves that the 10 core AR routes respond correctly at the HTTP boundary
//! via reqwest against a live AR service. Each route is tested for:
//! - Happy path: correct status code + valid JSON response
//! - Auth enforcement: no JWT → 401 Unauthorized
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_ar_customer_invoice -- --nocapture
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

// ---------------------------------------------------------------------------
// JWT helpers — sign tokens with the dev private key from .env
// ---------------------------------------------------------------------------

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
    let pem = pem.replace("\\n", "\n");
    EncodingKey::from_rsa_pem(pem.as_bytes()).ok()
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

// ---------------------------------------------------------------------------
// Service readiness
// ---------------------------------------------------------------------------

async fn wait_for_ar(client: &Client) -> bool {
    let url = format!("{}/api/health", ar_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  AR health attempt {}/15: status {}", attempt, r.status()),
            Err(e) => eprintln!("  AR health attempt {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn smoke_ar_customer_invoice() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_ar(&client).await {
        eprintln!("AR service not reachable at {} — skipping", ar_url());
        return;
    }
    println!("AR service healthy at {}", ar_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set — skipping (cannot sign test JWTs)");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["ar.mutate", "ar.read"]);
    let base = ar_url();

    // Gate: verify the AR service accepts our JWT (has JWT_PUBLIC_KEY configured).
    // If not, the service returns 401 for all requests regardless of JWT validity.
    {
        let probe = client
            .get(format!("{}/api/ar/customers", base))
            .bearer_auth(&jwt)
            .send()
            .await
            .expect("JWT probe request failed");
        if probe.status().as_u16() == 401 {
            eprintln!(
                "AR service at {} returns 401 even with valid JWT — \
                 JWT_PUBLIC_KEY likely not configured. Skipping.\n\
                 Fix: set JWT_PUBLIC_KEY in the AR container env (see docker-compose.services.yml)",
                base
            );
            return;
        }
    }

    // =================================================================
    // 1. POST /api/ar/customers — create customer (happy path)
    // =================================================================
    println!("\n--- 1. POST /api/ar/customers ---");
    let email = format!("smoke-{}@test.local", Uuid::new_v4());
    let resp = client
        .post(format!("{}/api/ar/customers", base))
        .bearer_auth(&jwt)
        .json(&json!({
            "email": email,
            "name": "Smoke Test Customer"
        }))
        .send()
        .await
        .expect("create customer request failed");

    assert_eq!(resp.status().as_u16(), 201, "expected 201 Created");
    let customer: Value = resp.json().await.expect("invalid JSON from create customer");
    let customer_id = customer["id"].as_i64().expect("customer.id missing");
    println!("  created customer id={}", customer_id);

    // 1b. Auth enforcement: no JWT → 401
    let resp = client
        .post(format!("{}/api/ar/customers", base))
        .json(&json!({"email": "no-jwt@test.local", "name": "No JWT"}))
        .send()
        .await
        .expect("unauth create customer request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT, got {}",
        resp.status()
    );
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 2. GET /api/ar/customers/{id} — retrieve customer
    // =================================================================
    println!("\n--- 2. GET /api/ar/customers/{{id}} ---");
    let resp = client
        .get(format!("{}/api/ar/customers/{}", base, customer_id))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get customer request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK");
    let body: Value = resp.json().await.expect("invalid JSON from get customer");
    assert_eq!(body["id"].as_i64().unwrap(), customer_id);
    assert_eq!(body["email"].as_str().unwrap(), email);
    println!("  retrieved customer email={}", email);

    // 2b. Auth enforcement
    let resp = client
        .get(format!("{}/api/ar/customers/{}", base, customer_id))
        .send()
        .await
        .expect("unauth get customer request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 3. POST /api/ar/invoices — create invoice
    // =================================================================
    println!("\n--- 3. POST /api/ar/invoices ---");
    let resp = client
        .post(format!("{}/api/ar/invoices", base))
        .bearer_auth(&jwt)
        .json(&json!({
            "ar_customer_id": customer_id,
            "amount_cents": 5000,
            "currency": "usd"
        }))
        .send()
        .await
        .expect("create invoice request failed");

    assert_eq!(resp.status().as_u16(), 201, "expected 201 Created");
    let invoice: Value = resp.json().await.expect("invalid JSON from create invoice");
    let invoice_id = invoice["id"].as_i64().expect("invoice.id missing");
    assert_eq!(invoice["amount_cents"].as_i64().unwrap(), 5000);
    assert_eq!(invoice["status"].as_str().unwrap(), "draft");
    println!("  created invoice id={}, amount=5000, status=draft", invoice_id);

    // 3b. Auth enforcement
    let resp = client
        .post(format!("{}/api/ar/invoices", base))
        .json(&json!({"ar_customer_id": customer_id, "amount_cents": 100}))
        .send()
        .await
        .expect("unauth create invoice request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 4. GET /api/ar/invoices/{id} — retrieve invoice
    // =================================================================
    println!("\n--- 4. GET /api/ar/invoices/{{id}} ---");
    let resp = client
        .get(format!("{}/api/ar/invoices/{}", base, invoice_id))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get invoice request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: Value = resp.json().await.expect("invalid JSON from get invoice");
    assert_eq!(body["id"].as_i64().unwrap(), invoice_id);
    assert_eq!(body["ar_customer_id"].as_i64().unwrap(), customer_id);
    println!("  retrieved invoice id={}", invoice_id);

    // 4b. Auth enforcement
    let resp = client
        .get(format!("{}/api/ar/invoices/{}", base, invoice_id))
        .send()
        .await
        .expect("unauth get invoice request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 5. POST /api/ar/invoices/{id}/finalize — finalize invoice
    // =================================================================
    println!("\n--- 5. POST /api/ar/invoices/{{id}}/finalize ---");
    let resp = client
        .post(format!("{}/api/ar/invoices/{}/finalize", base, invoice_id))
        .bearer_auth(&jwt)
        .json(&json!({}))
        .send()
        .await
        .expect("finalize invoice request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 OK for finalize");
    let body: Value = resp.json().await.expect("invalid JSON from finalize invoice");
    assert_eq!(body["status"].as_str().unwrap(), "open");
    println!("  finalized invoice → status=open");

    // 5b. Auth enforcement
    let resp = client
        .post(format!("{}/api/ar/invoices/{}/finalize", base, invoice_id))
        .json(&json!({}))
        .send()
        .await
        .expect("unauth finalize request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 6. POST /api/ar/invoices/{id}/bill-usage — bill usage to invoice
    // =================================================================
    println!("\n--- 6. POST /api/ar/invoices/{{id}}/bill-usage ---");

    // Create a second draft invoice for bill-usage (finalized one is already open)
    let resp = client
        .post(format!("{}/api/ar/invoices", base))
        .bearer_auth(&jwt)
        .json(&json!({
            "ar_customer_id": customer_id,
            "amount_cents": 0,
            "currency": "usd"
        }))
        .send()
        .await
        .expect("create invoice for bill-usage failed");
    assert_eq!(resp.status().as_u16(), 201);
    let bill_invoice: Value = resp.json().await.unwrap();
    let bill_invoice_id = bill_invoice["id"].as_i64().unwrap();

    let now = Utc::now();
    let period_start = (now - chrono::Duration::days(30)).to_rfc3339();
    let period_end = now.to_rfc3339();

    let resp = client
        .post(format!(
            "{}/api/ar/invoices/{}/bill-usage",
            base, bill_invoice_id
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "customer_id": customer_id,
            "period_start": period_start,
            "period_end": period_end,
            "correlation_id": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .expect("bill-usage request failed");

    // 200 with billed_count (may be 0 if no usage records exist — that's fine)
    assert_eq!(resp.status().as_u16(), 200, "expected 200 for bill-usage");
    let body: Value = resp.json().await.expect("invalid JSON from bill-usage");
    assert!(
        body.get("billed_count").is_some(),
        "bill-usage response must include billed_count"
    );
    println!(
        "  bill-usage → billed_count={}, total={}",
        body["billed_count"], body["total_amount_minor"]
    );

    // 6b. Auth enforcement
    let resp = client
        .post(format!(
            "{}/api/ar/invoices/{}/bill-usage",
            base, bill_invoice_id
        ))
        .json(&json!({
            "customer_id": customer_id,
            "period_start": period_start,
            "period_end": period_end,
            "correlation_id": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .expect("unauth bill-usage request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 7. POST /api/ar/charges — create charge
    //    Note: requires customer to have a default_payment_method_id.
    //    Without it, we get 409 Conflict. That's correct behavior — we
    //    verify the charge route accepts the request shape and validates.
    // =================================================================
    println!("\n--- 7. POST /api/ar/charges ---");
    let reference_id = format!("smoke-ref-{}", Uuid::new_v4());
    let resp = client
        .post(format!("{}/api/ar/charges", base))
        .bearer_auth(&jwt)
        .json(&json!({
            "ar_customer_id": customer_id,
            "amount_cents": 2500,
            "reason": "Smoke test charge",
            "reference_id": reference_id,
            "currency": "usd"
        }))
        .send()
        .await
        .expect("create charge request failed");

    // 201 if customer has payment method, 409 if not — both prove the route works
    let status = resp.status().as_u16();
    assert!(
        status == 201 || status == 409,
        "expected 201 or 409 for create charge, got {}",
        status
    );
    let charge_body: Value = resp.json().await.expect("invalid JSON from create charge");
    if status == 201 {
        let charge_id = charge_body["id"].as_i64().expect("charge.id missing");
        println!("  created charge id={}", charge_id);
    } else {
        println!(
            "  409 Conflict (no payment method) — expected for smoke test: {}",
            charge_body["message"]
                .as_str()
                .unwrap_or("no message field")
        );
    }

    // 7b. Auth enforcement
    let resp = client
        .post(format!("{}/api/ar/charges", base))
        .json(&json!({
            "ar_customer_id": customer_id,
            "amount_cents": 100,
            "reason": "no auth",
            "reference_id": format!("no-auth-{}", Uuid::new_v4())
        }))
        .send()
        .await
        .expect("unauth create charge request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 8. GET /api/ar/charges/{id} — retrieve charge
    //    Use list endpoint to find a charge ID if one was created
    // =================================================================
    println!("\n--- 8. GET /api/ar/charges/{{id}} ---");
    let resp = client
        .get(format!(
            "{}/api/ar/charges?customer_id={}",
            base, customer_id
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("list charges request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let charges: Vec<Value> = resp.json().await.expect("invalid JSON from list charges");

    if let Some(first_charge) = charges.first() {
        let charge_id = first_charge["id"].as_i64().unwrap();
        let resp = client
            .get(format!("{}/api/ar/charges/{}", base, charge_id))
            .bearer_auth(&jwt)
            .send()
            .await
            .expect("get charge request failed");
        assert_eq!(resp.status().as_u16(), 200);
        let body: Value = resp.json().await.expect("invalid JSON from get charge");
        assert_eq!(body["id"].as_i64().unwrap(), charge_id);
        println!("  retrieved charge id={}", charge_id);
    } else {
        println!("  no charges to retrieve (customer had no payment method)");
        // Verify the route still responds correctly for non-existent ID
        let resp = client
            .get(format!("{}/api/ar/charges/999999", base))
            .bearer_auth(&jwt)
            .send()
            .await
            .expect("get nonexistent charge request failed");
        assert_eq!(resp.status().as_u16(), 404);
        println!("  GET nonexistent charge → 404 ✓");
    }

    // 8b. Auth enforcement
    let resp = client
        .get(format!("{}/api/ar/charges/1", base))
        .send()
        .await
        .expect("unauth get charge request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 9. POST /api/ar/charges/{id}/capture — capture charge
    //    Requires charge in "authorized" status with a provider ID.
    //    Our smoke test charge (if it exists) is "pending" → expect 400 or 404.
    // =================================================================
    println!("\n--- 9. POST /api/ar/charges/{{id}}/capture ---");
    if let Some(first_charge) = charges.first() {
        let charge_id = first_charge["id"].as_i64().unwrap();
        let resp = client
            .post(format!("{}/api/ar/charges/{}/capture", base, charge_id))
            .bearer_auth(&jwt)
            .json(&json!({}))
            .send()
            .await
            .expect("capture charge request failed");

        // 400 (not authorized status) proves the route works and validates state
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 400 || status == 409,
            "expected 200/400/409 for capture, got {}",
            status
        );
        println!("  capture charge → {} (expected for pending charge)", status);
    } else {
        // No charges exist — capture a nonexistent charge → 404
        let resp = client
            .post(format!("{}/api/ar/charges/999999/capture", base))
            .bearer_auth(&jwt)
            .json(&json!({}))
            .send()
            .await
            .expect("capture nonexistent charge request failed");
        assert_eq!(resp.status().as_u16(), 404);
        println!("  capture nonexistent → 404 ✓");
    }

    // 9b. Auth enforcement
    let resp = client
        .post(format!("{}/api/ar/charges/1/capture", base))
        .json(&json!({}))
        .send()
        .await
        .expect("unauth capture request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // 10. POST /api/ar/usage — record usage
    // =================================================================
    println!("\n--- 10. POST /api/ar/usage ---");
    let idempotency_key = Uuid::new_v4();
    let resp = client
        .post(format!("{}/api/ar/usage", base))
        .bearer_auth(&jwt)
        .json(&json!({
            "idempotency_key": idempotency_key,
            "customer_id": customer_id.to_string(),
            "metric_name": "api_calls",
            "quantity": 42.0,
            "unit": "calls",
            "unit_price_minor": 10,
            "period_start": period_start,
            "period_end": period_end
        }))
        .send()
        .await
        .expect("capture usage request failed");

    assert_eq!(resp.status().as_u16(), 200, "expected 200 for usage capture");
    let usage: Value = resp.json().await.expect("invalid JSON from usage capture");
    assert_eq!(usage["metric_name"].as_str().unwrap(), "api_calls");
    println!(
        "  captured usage: metric={}, quantity={}",
        usage["metric_name"], usage["quantity"]
    );

    // Idempotency: same key returns same record
    let resp = client
        .post(format!("{}/api/ar/usage", base))
        .bearer_auth(&jwt)
        .json(&json!({
            "idempotency_key": idempotency_key,
            "customer_id": customer_id.to_string(),
            "metric_name": "api_calls",
            "quantity": 42.0,
            "unit": "calls",
            "unit_price_minor": 10,
            "period_start": period_start,
            "period_end": period_end
        }))
        .send()
        .await
        .expect("idempotent usage request failed");
    assert_eq!(resp.status().as_u16(), 200);
    println!("  idempotent replay → 200 ✓");

    // 10b. Auth enforcement
    let resp = client
        .post(format!("{}/api/ar/usage", base))
        .json(&json!({
            "idempotency_key": Uuid::new_v4(),
            "customer_id": "1",
            "metric_name": "x",
            "quantity": 1.0,
            "unit": "x",
            "unit_price_minor": 1,
            "period_start": period_start,
            "period_end": period_end
        }))
        .send()
        .await
        .expect("unauth usage request failed");
    assert_eq!(resp.status().as_u16(), 401);
    println!("  no-JWT → 401 ✓");

    // =================================================================
    // Verify no SQL/stack traces leaked in error responses
    // =================================================================
    println!("\n--- Error response sanitization check ---");
    let resp = client
        .get(format!("{}/api/ar/customers/999999", base))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("error sanitization check failed");
    assert_eq!(resp.status().as_u16(), 404);
    let body = resp.text().await.unwrap();
    assert!(
        !body.contains("SELECT") && !body.contains("sqlx") && !body.contains("panicked"),
        "error response must not leak SQL or stack traces: {}",
        &body[..body.len().min(200)]
    );
    println!("  404 response is sanitized ✓");

    println!("\n=== All 10 AR smoke tests passed ===");
}
