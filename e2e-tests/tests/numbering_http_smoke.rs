// HTTP smoke tests: Numbering (4 routes)
//
// Proves that the 4 Numbering API routes respond correctly at the HTTP
// boundary via reqwest against the live numbering service.
//
// Flow: PUT policy → GET policy → POST allocate (gap-free) → POST confirm

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, Method, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const NUMBERING_DEFAULT_URL: &str = "http://localhost:8120";

fn numbering_url() -> String {
    std::env::var("NUMBERING_URL").unwrap_or_else(|_| NUMBERING_DEFAULT_URL.to_string())
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
    let url = format!("{}/api/health", numbering_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  numbering health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  numbering health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: Method, url: &str, body: Option<Value>) {
    let req = match method {
        Method::GET => client.get(url),
        Method::POST => client.post(url),
        Method::PUT => client.put(url),
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
    println!("  no-JWT -> 401 ok");
}

#[tokio::test]
async fn smoke_numbering() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Numbering service not reachable at {} -- skipping",
            numbering_url()
        );
        return;
    }
    println!("Numbering service healthy at {}", numbering_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["numbering.allocate"]);
    let base = numbering_url();

    // Probe JWT acceptance
    let probe = client
        .get(format!("{base}/api/health"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Numbering returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    // Use a unique entity name per test run to avoid cross-run interference
    let entity = format!("smoke-invoice-{}", &Uuid::new_v4().to_string()[..8]);
    let idempotency_key = Uuid::new_v4().to_string();

    // --- 1. PUT /policies/{entity} ---
    println!("\n--- 1. PUT /policies/{entity} ---");
    let resp = client
        .put(format!("{base}/policies/{entity}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "pattern": "INV-{number}",
            "prefix": "INV",
            "padding": 6
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let policy_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "PUT policy failed: {status} - {policy_body}"
    );
    assert_eq!(policy_body["entity"], entity, "policy entity mismatch");
    assert_eq!(
        policy_body["pattern"], "INV-{number}",
        "policy pattern mismatch"
    );
    println!(
        "  policy upserted: entity={} version={}",
        policy_body["entity"], policy_body["version"]
    );

    assert_unauth(
        &client,
        Method::PUT,
        &format!("{base}/policies/{entity}"),
        Some(json!({"pattern": "X-{number}"})),
    )
    .await;

    // --- 2. GET /policies/{entity} ---
    println!("\n--- 2. GET /policies/{entity} ---");
    let resp = client
        .get(format!("{base}/policies/{entity}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "GET policy failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["entity"], entity, "fetched entity mismatch");
    assert_eq!(
        fetched["pattern"], "INV-{number}",
        "fetched pattern mismatch"
    );
    assert_eq!(fetched["padding"], 6, "fetched padding mismatch");
    println!(
        "  policy retrieved: pattern={} padding={}",
        fetched["pattern"], fetched["padding"]
    );

    assert_unauth(
        &client,
        Method::GET,
        &format!("{base}/policies/{entity}"),
        None,
    )
    .await;

    // --- 3. POST /allocate (gap-free to enable confirm test) ---
    println!("\n--- 3. POST /allocate ---");
    let resp = client
        .post(format!("{base}/allocate"))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity": entity,
            "idempotency_key": idempotency_key,
            "gap_free": true
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let alloc_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "POST allocate failed: {status} - {alloc_body}"
    );
    let number_value = alloc_body["number_value"]
        .as_i64()
        .expect("no number_value");
    assert!(number_value > 0, "number_value must be positive");
    // gap-free allocation starts as "reserved"
    assert_eq!(
        alloc_body["status"], "reserved",
        "gap-free allocation must be reserved"
    );
    assert_eq!(alloc_body["entity"], entity);
    // formatted_number should use our policy
    let formatted = alloc_body["formatted_number"].as_str().unwrap_or("");
    assert!(
        formatted.starts_with("INV-"),
        "formatted_number should start with INV-: got {formatted}"
    );
    println!("  allocated number={number_value} formatted={formatted} status=reserved");

    assert_unauth(
        &client,
        Method::POST,
        &format!("{base}/allocate"),
        Some(json!({"entity": entity, "idempotency_key": Uuid::new_v4().to_string()})),
    )
    .await;

    // Idempotency: re-allocate with same key → replay
    let resp = client
        .post(format!("{base}/allocate"))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity": entity,
            "idempotency_key": idempotency_key,
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "idempotent re-allocate failed: {}",
        resp.status()
    );
    let replay_body: Value = resp.json().await.unwrap();
    assert_eq!(
        replay_body["replay"], true,
        "duplicate alloc must be replay=true"
    );
    assert_eq!(
        replay_body["number_value"], number_value,
        "replay must return same number"
    );
    println!("  idempotency replay: number={number_value} replay=true");

    // --- 4. POST /confirm ---
    println!("\n--- 4. POST /confirm ---");
    let resp = client
        .post(format!("{base}/confirm"))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity": entity,
            "idempotency_key": idempotency_key
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let confirm_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::OK,
        "POST confirm failed: {status} - {confirm_body}"
    );
    assert_eq!(
        confirm_body["status"], "confirmed",
        "confirm must yield confirmed status"
    );
    assert_eq!(
        confirm_body["number_value"], number_value,
        "confirmed number must match"
    );
    assert_eq!(
        confirm_body["replay"], false,
        "first confirm must not be replay"
    );
    println!("  confirmed number={number_value} status=confirmed");

    // Idempotent re-confirm
    let resp = client
        .post(format!("{base}/confirm"))
        .bearer_auth(&jwt)
        .json(&json!({"entity": entity, "idempotency_key": idempotency_key}))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "idempotent re-confirm failed: {}",
        resp.status()
    );
    let replay_confirm: Value = resp.json().await.unwrap();
    assert_eq!(
        replay_confirm["replay"], true,
        "re-confirm must be replay=true"
    );
    println!("  idempotent re-confirm: replay=true");

    assert_unauth(
        &client,
        Method::POST,
        &format!("{base}/confirm"),
        Some(json!({"entity": entity, "idempotency_key": Uuid::new_v4().to_string()})),
    )
    .await;

    println!("\n=== All 4 Numbering routes passed ===");
}
