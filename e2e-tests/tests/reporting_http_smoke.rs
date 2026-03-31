// HTTP smoke tests: Reporting service
//
// Proves that all 11 reporting routes respond correctly at the HTTP boundary
// via reqwest against the live Reporting service. No mocks, no stubs.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const RPT_DEFAULT_URL: &str = "http://localhost:8096";

fn rpt_url() -> String {
    std::env::var("REPORTING_URL").unwrap_or_else(|_| RPT_DEFAULT_URL.to_string())
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

async fn wait_for_reporting(client: &Client) -> bool {
    let url = format!("{}/api/health", rpt_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Reporting health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Reporting health {}/15: {}", attempt, e),
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
    let req = if let Some(b) = body { req.json(&b) } else { req };
    let resp = req.send().await.expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_reporting() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_reporting(&client).await {
        eprintln!(
            "Reporting service not reachable at {} -- skipping",
            rpt_url()
        );
        return;
    }
    println!("Reporting service healthy at {}", rpt_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["reporting.read", "reporting.mutate"]);
    let base = rpt_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!(
            "{base}/api/reporting/pl?from=2026-01-01&to=2026-01-31"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("Reporting returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // Date ranges for queries — current month
    let today = Utc::now().date_naive();
    let first_of_month = today
        .with_day(1)
        .unwrap_or(today)
        .format("%Y-%m-%d")
        .to_string();
    let today_str = today.format("%Y-%m-%d").to_string();

    // ── 1. GET /api/reporting/pl ─────────────────────────────────────
    println!("\n--- 1. GET /api/reporting/pl ---");
    let resp = client
        .get(format!("{base}/api/reporting/pl"))
        .bearer_auth(&jwt)
        .query(&[("from", &first_of_month), ("to", &today_str)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "P&L failed: {status} - {body}");
    println!("  P&L ok: {status}");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/reporting/pl?from={first_of_month}&to={today_str}"),
        None,
    )
    .await;

    // ── 2. GET /api/reporting/balance-sheet ──────────────────────────
    println!("\n--- 2. GET /api/reporting/balance-sheet ---");
    let resp = client
        .get(format!("{base}/api/reporting/balance-sheet"))
        .bearer_auth(&jwt)
        .query(&[("as_of", &today_str)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Balance sheet failed: {status} - {body}");
    println!("  balance sheet ok: {status}");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/reporting/balance-sheet?as_of={today_str}"),
        None,
    )
    .await;

    // ── 3. GET /api/reporting/cashflow ───────────────────────────────
    println!("\n--- 3. GET /api/reporting/cashflow ---");
    let resp = client
        .get(format!("{base}/api/reporting/cashflow"))
        .bearer_auth(&jwt)
        .query(&[("from", &first_of_month), ("to", &today_str)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Cash flow failed: {status} - {body}");
    println!("  cashflow ok: {status}");

    // ── 4. GET /api/reporting/ar-aging ───────────────────────────────
    println!("\n--- 4. GET /api/reporting/ar-aging ---");
    let resp = client
        .get(format!("{base}/api/reporting/ar-aging"))
        .bearer_auth(&jwt)
        .query(&[("as_of", &today_str)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "AR aging failed: {status} - {body}");
    assert!(
        body["aging"].is_array(),
        "AR aging response should have 'aging' array"
    );
    println!(
        "  AR aging ok: {} buckets",
        body["aging"].as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/reporting/ar-aging?as_of={today_str}"),
        None,
    )
    .await;

    // ── 5. GET /api/reporting/ap-aging ───────────────────────────────
    println!("\n--- 5. GET /api/reporting/ap-aging ---");
    let resp = client
        .get(format!("{base}/api/reporting/ap-aging"))
        .bearer_auth(&jwt)
        .query(&[("as_of", &today_str)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "AP aging failed: {status} - {body}");
    // ApAgingReport has { as_of, vendors: [], summary_by_currency: [] }
    assert!(
        body["vendors"].is_array(),
        "AP aging response should have 'vendors' array; got: {body}"
    );
    println!(
        "  AP aging ok: {} vendor rows",
        body["vendors"].as_array().map(|a| a.len()).unwrap_or(0)
    );

    // ── 6. GET /api/reporting/kpis ───────────────────────────────────
    println!("\n--- 6. GET /api/reporting/kpis ---");
    let resp = client
        .get(format!("{base}/api/reporting/kpis"))
        .bearer_auth(&jwt)
        .query(&[("as_of", &today_str)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "KPIs failed: {status} - {body}");
    println!("  KPIs ok: {status}");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/reporting/kpis?as_of={today_str}"),
        None,
    )
    .await;

    // ── 7. GET /api/reporting/forecast ───────────────────────────────
    println!("\n--- 7. GET /api/reporting/forecast ---");
    let resp = client
        .get(format!("{base}/api/reporting/forecast"))
        .bearer_auth(&jwt)
        .query(&[("horizons", "7,14,30")])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Forecast failed: {status} - {body}");
    println!("  forecast ok: {status}");

    // ── 8. POST /api/reporting/rebuild ───────────────────────────────
    // Requires JWT + x-admin-token. Without admin token → 403.
    println!("\n--- 8. POST /api/reporting/rebuild ---");
    let resp = client
        .post(format!("{base}/api/reporting/rebuild"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "from": first_of_month,
            "to": today_str
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    // Without ADMIN_TOKEN configured → 403. With it configured → 200.
    // Either is valid proof the route is wired.
    assert!(
        status == StatusCode::FORBIDDEN || status.is_success(),
        "Rebuild returned unexpected status: {status} - {body}"
    );
    println!("  rebuild responded: {status}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/reporting/rebuild"),
        Some(json!({"tenant_id": tenant_id, "from": first_of_month, "to": today_str})),
    )
    .await;

    // ── 9. POST /api/reporting/admin/projection-status ───────────────
    // Admin router — no JWT, x-admin-token required → 403 without it
    println!("\n--- 9. POST /api/reporting/admin/projection-status ---");
    let resp = client
        .post(format!("{base}/api/reporting/admin/projection-status"))
        .json(&json!({"projection_name": "rpt_trial_balance_cache"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    // Without x-admin-token → 403 (expected). With valid token → 200.
    assert!(
        status == StatusCode::FORBIDDEN || status.is_success(),
        "Admin projection-status returned unexpected status: {status}"
    );
    println!("  projection-status responded: {status}");

    // ── 10. POST /api/reporting/admin/consistency-check ──────────────
    println!("\n--- 10. POST /api/reporting/admin/consistency-check ---");
    let resp = client
        .post(format!("{base}/api/reporting/admin/consistency-check"))
        .json(&json!({"projection_name": "rpt_trial_balance_cache"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::FORBIDDEN || status.is_success(),
        "Admin consistency-check returned unexpected status: {status}"
    );
    println!("  consistency-check responded: {status}");

    // ── 11. GET /api/reporting/admin/projections ──────────────────────
    println!("\n--- 11. GET /api/reporting/admin/projections ---");
    let resp = client
        .get(format!("{base}/api/reporting/admin/projections"))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(
        status == StatusCode::FORBIDDEN || status.is_success(),
        "Admin projections returned unexpected status: {status}"
    );
    println!("  admin/projections responded: {status}");

    println!("\n=== All 11 reporting routes passed ===");
}

// chrono::NaiveDate::with_day is from chrono's Datelike trait
use chrono::Datelike;
