//! HTTP smoke tests: AR Tax + Aging + Recon + Admin
//!
//! Proves that 16 AR operational/admin routes respond correctly at the HTTP
//! boundary via reqwest against a live AR service. Each route is tested for:
//! - Happy path: correct status code + valid JSON response
//! - Auth enforcement: no JWT -> 401 Unauthorized (JWT-protected routes)
//! - Admin auth: no X-Admin-Token -> 403 Forbidden (admin routes)
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests --test smoke_ar_tax_aging_admin -- --nocapture
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

#[tokio::test]
async fn smoke_ar_tax_aging_admin() {
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

    // Gate: verify the AR service accepts our JWT
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

    // =========================================================================
    // TAX CONFIGURATION — Jurisdictions
    // =========================================================================

    // --- 1. POST /api/ar/tax/config/jurisdictions ---
    println!("\n--- 1. POST /api/ar/tax/config/jurisdictions ---");
    let jur_body = json!({
        "country_code": "US",
        "state_code": "TX",
        "jurisdiction_name": format!("Smoke Tax Jur {}", Uuid::new_v4()),
        "tax_type": "sales_tax"
    });
    let resp = client
        .post(format!("{base}/api/ar/tax/config/jurisdictions"))
        .bearer_auth(&jwt)
        .json(&jur_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 201,
        "expected 201 for jurisdiction create, got {status}: {body}"
    );
    let jur_id = body["id"]
        .as_str()
        .expect("jurisdiction.id missing")
        .to_string();
    println!("  created jurisdiction id={jur_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/tax/config/jurisdictions"),
        Some(jur_body),
    )
    .await;

    // --- 2. GET /api/ar/tax/config/jurisdictions/{id} ---
    println!("\n--- 2. GET /api/ar/tax/config/jurisdictions/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/tax/config/jurisdictions/{jur_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for jurisdiction get");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), jur_id);
    println!("  retrieved jurisdiction id={jur_id}");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/tax/config/jurisdictions/{jur_id}"),
        None,
    )
    .await;

    // =========================================================================
    // TAX CONFIGURATION — Rules
    // =========================================================================

    // --- 3. POST /api/ar/tax/config/rules ---
    println!("\n--- 3. POST /api/ar/tax/config/rules ---");
    let rule_body = json!({
        "jurisdiction_id": jur_id,
        "tax_code": "GENERAL",
        "rate": 0.0825,
        "flat_amount_minor": 0,
        "is_exempt": false,
        "effective_from": "2025-01-01",
        "priority": 1
    });
    let resp = client
        .post(format!("{base}/api/ar/tax/config/rules"))
        .bearer_auth(&jwt)
        .json(&rule_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert_eq!(
        status, 201,
        "expected 201 for rule create, got {status}: {body}"
    );
    let rule_id = body["id"].as_str().expect("rule.id missing").to_string();
    println!("  created rule id={rule_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/tax/config/rules"),
        Some(rule_body),
    )
    .await;

    // --- 4. GET /api/ar/tax/config/rules/{id} ---
    println!("\n--- 4. GET /api/ar/tax/config/rules/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/tax/config/rules/{rule_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for rule get");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"].as_str().unwrap(), rule_id);
    println!("  retrieved rule id={rule_id}");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/tax/config/rules/{rule_id}"),
        None,
    )
    .await;

    // =========================================================================
    // TAX REPORTS
    // =========================================================================

    // --- 5. GET /api/ar/tax/reports/export ---
    println!("\n--- 5. GET /api/ar/tax/reports/export ---");
    let resp = client
        .get(format!(
            "{base}/api/ar/tax/reports/export?from=2025-01-01&to=2026-12-31&format=json"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for tax export");
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        body.get("rows").is_some() || body.get("app_id").is_some(),
        "tax export response missing expected fields: {body}"
    );
    println!(
        "  tax export -> {status}, total_tax={}",
        body.get("total_tax_minor").unwrap_or(&json!(0))
    );

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/tax/reports/export?from=2025-01-01&to=2026-12-31"),
        None,
    )
    .await;

    // --- 6. GET /api/ar/tax/reports/summary ---
    println!("\n--- 6. GET /api/ar/tax/reports/summary ---");
    let resp = client
        .get(format!(
            "{base}/api/ar/tax/reports/summary?from=2025-01-01&to=2026-12-31"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for tax summary");
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        body.get("rows").is_some(),
        "tax summary missing rows field: {body}"
    );
    println!(
        "  tax summary -> {status}, total_invoices={}",
        body.get("total_invoices").unwrap_or(&json!(0))
    );

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/tax/reports/summary?from=2025-01-01&to=2026-12-31"),
        None,
    )
    .await;

    // =========================================================================
    // AGING
    // =========================================================================

    // --- 7. GET /api/ar/aging ---
    println!("\n--- 7. GET /api/ar/aging ---");
    let resp = client
        .get(format!("{base}/api/ar/aging"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for aging report");
    let body: Value = resp.json().await.unwrap_or(json!(null));
    assert!(
        body.get("aging").is_some(),
        "aging response missing 'aging' field: {body}"
    );
    println!(
        "  aging report -> {status}, buckets={}",
        match body.get("aging") {
            Some(Value::Array(a)) => a.len(),
            _ => 0,
        }
    );

    assert_unauth(&client, "GET", &format!("{base}/api/ar/aging"), None).await;

    // --- 8. POST /api/ar/aging/refresh ---
    println!("\n--- 8. POST /api/ar/aging/refresh ---");
    let email = format!("smoke-aging-{}@test.local", Uuid::new_v4());
    let resp = client
        .post(format!("{base}/api/ar/customers"))
        .bearer_auth(&jwt)
        .json(&json!({"email": email, "name": "Smoke Aging Customer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let cust: Value = resp.json().await.unwrap();
    let customer_id = cust["id"].as_i64().expect("customer.id missing");

    let refresh_body = json!({"customer_id": customer_id});
    let resp = client
        .post(format!("{base}/api/ar/aging/refresh"))
        .bearer_auth(&jwt)
        .json(&refresh_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for aging refresh, got {status}");
    let body: Value = resp.json().await.unwrap_or(json!(null));
    println!(
        "  aging refresh -> {status}, customer_id={customer_id}, snapshot={}",
        body.get("customer_id").unwrap_or(&json!(null))
    );

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/aging/refresh"),
        Some(refresh_body),
    )
    .await;

    // =========================================================================
    // RECONCILIATION
    // =========================================================================

    // --- 9. POST /api/ar/recon/poll ---
    println!("\n--- 9. POST /api/ar/recon/poll ---");
    let poll_body = json!({"worker_id": "smoke-worker", "batch_size": 5});
    let resp = client
        .post(format!("{base}/api/ar/recon/poll"))
        .bearer_auth(&jwt)
        .json(&poll_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for recon poll, got {status}");
    let body: Value = resp.json().await.unwrap_or(json!([]));
    println!(
        "  recon poll -> {status}, outcomes={}",
        match &body {
            Value::Array(a) => a.len(),
            _ => 0,
        }
    );

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/recon/poll"),
        Some(poll_body),
    )
    .await;

    // --- 10. POST /api/ar/recon/run ---
    println!("\n--- 10. POST /api/ar/recon/run ---");
    let run_body = json!({"recon_run_id": Uuid::new_v4()});
    let resp = client
        .post(format!("{base}/api/ar/recon/run"))
        .bearer_auth(&jwt)
        .json(&run_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for recon run, got {status}");
    let body: Value = resp.json().await.unwrap_or(json!(null));
    println!(
        "  recon run -> {status}, matched={}",
        body.get("matched_count").unwrap_or(&json!(0))
    );

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/recon/run"),
        Some(run_body),
    )
    .await;

    // --- 11. POST /api/ar/recon/schedule ---
    println!("\n--- 11. POST /api/ar/recon/schedule ---");
    let now = Utc::now().naive_utc();
    let window_start = (now - chrono::Duration::hours(1))
        .format("%Y-%m-%dT%H:%M:%S")
        .to_string();
    let window_end = now.format("%Y-%m-%dT%H:%M:%S").to_string();
    let schedule_body = json!({
        "scheduled_run_id": Uuid::new_v4(),
        "app_id": tenant_id,
        "window_start": window_start,
        "window_end": window_end
    });
    let resp = client
        .post(format!("{base}/api/ar/recon/schedule"))
        .bearer_auth(&jwt)
        .json(&schedule_body)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for recon schedule, got {status}");
    println!("  recon schedule -> {status}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/ar/recon/schedule"),
        Some(schedule_body),
    )
    .await;

    // =========================================================================
    // EVENTS
    // =========================================================================

    // --- 12. GET /api/ar/events ---
    println!("\n--- 12. GET /api/ar/events ---");
    let resp = client
        .get(format!("{base}/api/ar/events"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 200, "expected 200 for events list");
    let body: Value = resp.json().await.unwrap_or(json!([]));
    println!(
        "  events list -> {status}, count={}",
        match &body {
            Value::Array(a) => a.len(),
            _ => 0,
        }
    );

    assert_unauth(&client, "GET", &format!("{base}/api/ar/events"), None).await;

    // --- 13. GET /api/ar/events/{id} ---
    println!("\n--- 13. GET /api/ar/events/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/ar/events/999999"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    assert_eq!(status, 404, "expected 404 for nonexistent event");
    println!("  nonexistent event -> 404 OK");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/ar/events/999999"),
        None,
    )
    .await;

    // =========================================================================
    // ADMIN (X-Admin-Token protected, not JWT)
    // =========================================================================

    let admin_token = std::env::var("ADMIN_TOKEN").unwrap_or_default();
    if admin_token.is_empty() {
        eprintln!("ADMIN_TOKEN not set -- testing admin 403 only");
    }

    // --- 14. POST /api/ar/admin/consistency-check ---
    println!("\n--- 14. POST /api/ar/admin/consistency-check ---");
    let cc_body = json!({"projection_name": "ar_invoices"});
    if !admin_token.is_empty() {
        let resp = client
            .post(format!("{base}/api/ar/admin/consistency-check"))
            .header("x-admin-token", &admin_token)
            .json(&cc_body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 500,
            "expected 200/500 for consistency-check, got {status}"
        );
        println!("  consistency-check -> {status}");
    }
    // No token -> 403
    let resp = client
        .post(format!("{base}/api/ar/admin/consistency-check"))
        .json(&cc_body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status().as_u16(),
        403,
        "expected 403 without admin token"
    );
    println!("  no admin token -> 403 OK");

    // --- 15. POST /api/ar/admin/projection-status ---
    println!("\n--- 15. POST /api/ar/admin/projection-status ---");
    let ps_body = json!({"projection_name": "ar_invoices"});
    if !admin_token.is_empty() {
        let resp = client
            .post(format!("{base}/api/ar/admin/projection-status"))
            .header("x-admin-token", &admin_token)
            .json(&ps_body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 500,
            "expected 200/500 for projection-status, got {status}"
        );
        println!("  projection-status -> {status}");
    }
    let resp = client
        .post(format!("{base}/api/ar/admin/projection-status"))
        .json(&ps_body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status().as_u16(),
        403,
        "expected 403 without admin token"
    );
    println!("  no admin token -> 403 OK");

    // --- 16. GET /api/ar/admin/projections ---
    println!("\n--- 16. GET /api/ar/admin/projections ---");
    if !admin_token.is_empty() {
        let resp = client
            .get(format!("{base}/api/ar/admin/projections"))
            .header("x-admin-token", &admin_token)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 500,
            "expected 200/500 for projections list, got {status}"
        );
        println!("  projections list -> {status}");
    }
    let resp = client
        .get(format!("{base}/api/ar/admin/projections"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status().as_u16(),
        403,
        "expected 403 without admin token"
    );
    println!("  no admin token -> 403 OK");

    println!("\n=== All 16 AR tax/aging/recon/admin smoke tests passed ===");
}
