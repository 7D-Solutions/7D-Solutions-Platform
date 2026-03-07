// HTTP smoke tests: GL Reporting (8 routes)
//
// Proves that 8 GL financial-statement GET routes respond correctly at the
// HTTP boundary via reqwest against the live GL service.
//
// Requires: GL service running, GL_DATABASE_URL or default postgres connection,
//           JWT_PRIVATE_KEY_PEM set in .env.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

const GL_DEFAULT_URL: &str = "http://localhost:8090";
const GL_DB_DEFAULT: &str = "postgresql://gl_user:gl_pass@localhost:5438/gl_db";

fn gl_url() -> String {
    std::env::var("GL_URL").unwrap_or_else(|_| GL_DEFAULT_URL.to_string())
}

fn gl_db_url() -> String {
    std::env::var("GL_DATABASE_URL").unwrap_or_else(|_| GL_DB_DEFAULT.to_string())
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

async fn wait_for_service(client: &Client) -> bool {
    let url = format!("{}/api/health", gl_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  gl health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  gl health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, url: &str) {
    let resp = client
        .get(url)
        .send()
        .await
        .expect("unauth request failed");
    assert_eq!(
        resp.status().as_u16(),
        401,
        "expected 401 without JWT at {url}"
    );
    println!("  no-JWT -> 401 ok");
}

async fn get_gl_pool() -> Option<sqlx::PgPool> {
    let url = gl_db_url();
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
        .ok()
}

async fn seed_period(pool: &sqlx::PgPool, tenant_id: &str) -> Uuid {
    let period_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO accounting_periods (id, tenant_id, period_start, period_end) \
         VALUES ($1, $2, '2026-01-01', '2026-01-31')",
    )
    .bind(period_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("failed to seed accounting period");
    period_id
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    sqlx::query("DELETE FROM accounting_periods WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await
        .ok();
}

#[tokio::test]
async fn smoke_gl_reporting() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "GL service not reachable at {} -- skipping",
            gl_url()
        );
        return;
    }
    println!("GL service healthy at {}", gl_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    // Connect to GL DB to seed a real accounting period
    let Some(pool) = get_gl_pool().await else {
        eprintln!("GL DB not reachable at {} -- skipping", gl_db_url());
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["gl.read"]);
    let base = gl_url();

    // Probe JWT acceptance
    let probe = client
        .get(format!("{base}/api/gl/periods"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("GL returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        cleanup(&pool, &tenant_id).await;
        return;
    }

    // Seed a real period — reporting routes require the period to exist in DB
    let period_id = seed_period(&pool, &tenant_id).await;
    println!("seeded period={period_id} for tenant={tenant_id}");

    // --- 1. GET /api/gl/trial-balance ---
    println!("\n--- 1. GET /api/gl/trial-balance ---");
    let resp = client
        .get(format!("{base}/api/gl/trial-balance"))
        .bearer_auth(&jwt)
        .query(&[("period_id", period_id.to_string())])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "trial-balance failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("  trial-balance -> 200 ok");

    assert_unauth(
        &client,
        &format!("{base}/api/gl/trial-balance?period_id={period_id}"),
    )
    .await;

    // --- 2. GET /api/gl/balance-sheet ---
    println!("\n--- 2. GET /api/gl/balance-sheet ---");
    let resp = client
        .get(format!("{base}/api/gl/balance-sheet"))
        .bearer_auth(&jwt)
        .query(&[("period_id", period_id.to_string()), ("currency", "USD".to_string())])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "balance-sheet failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("  balance-sheet -> 200 ok");

    assert_unauth(
        &client,
        &format!("{base}/api/gl/balance-sheet?period_id={period_id}&currency=USD"),
    )
    .await;

    // --- 3. GET /api/gl/income-statement ---
    println!("\n--- 3. GET /api/gl/income-statement ---");
    let resp = client
        .get(format!("{base}/api/gl/income-statement"))
        .bearer_auth(&jwt)
        .query(&[("period_id", period_id.to_string()), ("currency", "USD".to_string())])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "income-statement failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("  income-statement -> 200 ok");

    assert_unauth(
        &client,
        &format!("{base}/api/gl/income-statement?period_id={period_id}&currency=USD"),
    )
    .await;

    // --- 4. GET /api/gl/cash-flow ---
    println!("\n--- 4. GET /api/gl/cash-flow ---");
    let resp = client
        .get(format!("{base}/api/gl/cash-flow"))
        .bearer_auth(&jwt)
        .query(&[("period_id", period_id.to_string()), ("currency", "USD".to_string())])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "cash-flow failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("  cash-flow -> 200 ok");

    assert_unauth(
        &client,
        &format!("{base}/api/gl/cash-flow?period_id={period_id}&currency=USD"),
    )
    .await;

    // --- 5. GET /api/gl/detail ---
    println!("\n--- 5. GET /api/gl/detail ---");
    let resp = client
        .get(format!("{base}/api/gl/detail"))
        .bearer_auth(&jwt)
        .query(&[("period_id", period_id.to_string())])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "gl/detail failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    println!("  gl/detail -> 200 ok");

    assert_unauth(
        &client,
        &format!("{base}/api/gl/detail?period_id={period_id}"),
    )
    .await;

    // --- 6. GET /api/gl/reporting/trial-balance ---
    println!("\n--- 6. GET /api/gl/reporting/trial-balance ---");
    let resp = client
        .get(format!("{base}/api/gl/reporting/trial-balance"))
        .bearer_auth(&jwt)
        .query(&[
            ("period_id", period_id.to_string()),
            ("reporting_currency", "USD".to_string()),
        ])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "reporting/trial-balance failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["reporting_currency"], "USD",
        "reporting_currency field missing or wrong: {body}"
    );
    assert!(
        body["is_reporting_currency"].is_boolean(),
        "is_reporting_currency must be a boolean: {body}"
    );
    println!(
        "  reporting/trial-balance -> 200, reporting_currency={}, is_reporting_currency={}",
        body["reporting_currency"], body["is_reporting_currency"]
    );

    assert_unauth(
        &client,
        &format!(
            "{base}/api/gl/reporting/trial-balance?period_id={period_id}&reporting_currency=USD"
        ),
    )
    .await;

    // --- 7. GET /api/gl/reporting/income-statement ---
    println!("\n--- 7. GET /api/gl/reporting/income-statement ---");
    let resp = client
        .get(format!("{base}/api/gl/reporting/income-statement"))
        .bearer_auth(&jwt)
        .query(&[
            ("period_id", period_id.to_string()),
            ("reporting_currency", "USD".to_string()),
        ])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "reporting/income-statement failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["reporting_currency"], "USD",
        "reporting_currency field missing or wrong: {body}"
    );
    assert!(
        body["is_reporting_currency"].is_boolean(),
        "is_reporting_currency must be a boolean: {body}"
    );
    println!(
        "  reporting/income-statement -> 200, reporting_currency={}",
        body["reporting_currency"]
    );

    assert_unauth(
        &client,
        &format!(
            "{base}/api/gl/reporting/income-statement?period_id={period_id}&reporting_currency=USD"
        ),
    )
    .await;

    // --- 8. GET /api/gl/reporting/balance-sheet ---
    println!("\n--- 8. GET /api/gl/reporting/balance-sheet ---");
    let resp = client
        .get(format!("{base}/api/gl/reporting/balance-sheet"))
        .bearer_auth(&jwt)
        .query(&[
            ("period_id", period_id.to_string()),
            ("reporting_currency", "USD".to_string()),
        ])
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == StatusCode::OK,
        "reporting/balance-sheet failed: {} - {:?}",
        resp.status(),
        resp.text().await.unwrap_or_default()
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["reporting_currency"], "USD",
        "reporting_currency field missing or wrong: {body}"
    );
    assert!(
        body["is_reporting_currency"].is_boolean(),
        "is_reporting_currency must be a boolean: {body}"
    );
    println!(
        "  reporting/balance-sheet -> 200, reporting_currency={}",
        body["reporting_currency"]
    );

    assert_unauth(
        &client,
        &format!(
            "{base}/api/gl/reporting/balance-sheet?period_id={period_id}&reporting_currency=USD"
        ),
    )
    .await;

    cleanup(&pool, &tenant_id).await;
    println!("\n=== All 8 GL Reporting routes passed ===");
}
