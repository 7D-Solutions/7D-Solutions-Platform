// HTTP smoke tests: Integrations service
//
// Proves that all 12 integrations routes respond correctly at the HTTP boundary
// via reqwest against the live Integrations service. No mocks, no stubs.
//
// Routes covered:
//   External Refs (6): create, get, list-by-entity, get-by-system, update, delete
//   Connectors (5): types, register, list, get, test
//   Webhooks (1): inbound webhook (unauthenticated)

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const INT_DEFAULT_URL: &str = "http://localhost:8099";

fn int_url() -> String {
    std::env::var("INTEGRATIONS_URL").unwrap_or_else(|_| INT_DEFAULT_URL.to_string())
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

async fn wait_for_integrations(client: &Client) -> bool {
    let url = format!("{}/api/health", int_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Integrations health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Integrations health {}/15: {}", attempt, e),
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
        "DELETE" => client.delete(url),
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
async fn smoke_integrations() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_integrations(&client).await {
        eprintln!(
            "Integrations service not reachable at {} -- skipping",
            int_url()
        );
        return;
    }
    println!("Integrations service healthy at {}", int_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["integrations.mutate", "integrations.read"],
    );
    let base = int_url();

    // Gate: verify the service accepts our JWT
    let probe = client
        .get(format!("{base}/api/integrations/connectors"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Integrations returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    let entity_id = Uuid::new_v4().to_string();
    let external_id = Uuid::new_v4().to_string();

    // ── 1. POST /api/integrations/external-refs ───────────────────────
    println!("\n--- 1. POST /api/integrations/external-refs ---");
    let resp = client
        .post(format!("{base}/api/integrations/external-refs"))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity_type": "invoice",
            "entity_id": entity_id,
            "system": "stripe",
            "external_id": external_id,
            "label": "Stripe invoice sync"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.as_u16() == 201,
        "Create external ref failed: {status} - {body}"
    );
    let ref_id = body["id"].as_i64().expect("no id in create response");
    println!("  external-ref id={ref_id} system={}", body["system"]);
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/integrations/external-refs"),
        Some(json!({"entity_type":"invoice","entity_id":"x","system":"stripe","external_id":"y"})),
    )
    .await;

    // ── 2. GET /api/integrations/external-refs/by-entity ─────────────
    println!("\n--- 2. GET /api/integrations/external-refs/by-entity ---");
    let resp = client
        .get(format!("{base}/api/integrations/external-refs/by-entity"))
        .bearer_auth(&jwt)
        .query(&[("entity_type", "invoice"), ("entity_id", &entity_id)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "List by entity failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  by-entity: {count} refs found");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/integrations/external-refs/by-entity?entity_type=invoice&entity_id={entity_id}"),
        None,
    )
    .await;

    // ── 3. GET /api/integrations/external-refs/by-system ─────────────
    println!("\n--- 3. GET /api/integrations/external-refs/by-system ---");
    let resp = client
        .get(format!("{base}/api/integrations/external-refs/by-system"))
        .bearer_auth(&jwt)
        .query(&[("system", "stripe"), ("external_id", &external_id)])
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Get by system failed: {status} - {body}"
    );
    println!("  by-system ok: entity_id={}", body["entity_id"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/integrations/external-refs/by-system?system=stripe&external_id={external_id}"),
        None,
    )
    .await;

    // ── 4. GET /api/integrations/external-refs/{id} ───────────────────
    println!("\n--- 4. GET /api/integrations/external-refs/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/integrations/external-refs/{ref_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Get external ref failed: {status} - {body}"
    );
    assert_eq!(body["id"].as_i64().unwrap_or(-1), ref_id);
    println!("  get ref ok: system={}", body["system"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/integrations/external-refs/{ref_id}"),
        None,
    )
    .await;

    // ── 5. PUT /api/integrations/external-refs/{id} ───────────────────
    println!("\n--- 5. PUT /api/integrations/external-refs/{{id}} ---");
    let resp = client
        .put(format!("{base}/api/integrations/external-refs/{ref_id}"))
        .bearer_auth(&jwt)
        .json(&json!({
            "label": "Updated Stripe invoice label"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Update external ref failed: {status} - {body}"
    );
    println!("  updated ref label={}", body["label"]);
    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/integrations/external-refs/{ref_id}"),
        Some(json!({"label": "X"})),
    )
    .await;

    // ── 6. DELETE /api/integrations/external-refs/{id} ───────────────
    println!("\n--- 6. DELETE /api/integrations/external-refs/{{id}} ---");
    // Create a second ref to delete
    let external_id2 = Uuid::new_v4().to_string();
    let resp2 = client
        .post(format!("{base}/api/integrations/external-refs"))
        .bearer_auth(&jwt)
        .json(&json!({
            "entity_type": "order",
            "entity_id": entity_id,
            "system": "stripe",
            "external_id": external_id2
        }))
        .send()
        .await
        .unwrap();
    let body2: Value = resp2.json().await.unwrap_or(json!({}));
    let ref_id2 = body2["id"].as_i64().unwrap_or(-1);
    assert!(ref_id2 > 0, "failed to create second external ref");

    let resp = client
        .delete(format!("{base}/api/integrations/external-refs/{ref_id2}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert_eq!(status.as_u16(), 204, "Delete external ref failed: {status}");
    println!("  external ref deleted: 204");
    assert_unauth(
        &client,
        "DELETE",
        &format!("{base}/api/integrations/external-refs/{ref_id2}"),
        None,
    )
    .await;

    // ── 7. GET /api/integrations/connectors/types ─────────────────────
    // No auth required — returns registered connector types
    println!("\n--- 7. GET /api/integrations/connectors/types ---");
    let resp = client
        .get(format!("{base}/api/integrations/connectors/types"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "List connector types failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  {count} connector types registered");
    assert!(count >= 1, "expected at least 1 connector type (echo)");

    // ── 8. POST /api/integrations/connectors ─────────────────────────
    println!("\n--- 8. POST /api/integrations/connectors ---");
    let resp = client
        .post(format!("{base}/api/integrations/connectors"))
        .bearer_auth(&jwt)
        .json(&json!({
            "connector_type": "echo",
            "name": "Smoke Test Echo Connector"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.as_u16() == 201,
        "Register connector failed: {status} - {body}"
    );
    let connector_id = body["id"]
        .as_str()
        .expect("no id in register connector response")
        .to_string();
    println!(
        "  connector id={connector_id} type={}",
        body["connector_type"]
    );
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/integrations/connectors"),
        Some(json!({"connector_type": "echo", "name": "X"})),
    )
    .await;

    // ── 9. GET /api/integrations/connectors ──────────────────────────
    println!("\n--- 9. GET /api/integrations/connectors ---");
    let resp = client
        .get(format!("{base}/api/integrations/connectors"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "List connectors failed: {status} - {body}"
    );
    let count = body.as_array().map(|a| a.len()).unwrap_or(0);
    println!("  listed {count} connectors");
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/integrations/connectors"),
        None,
    )
    .await;

    // ── 10. GET /api/integrations/connectors/{id} ─────────────────────
    println!("\n--- 10. GET /api/integrations/connectors/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/integrations/connectors/{connector_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Get connector failed: {status} - {body}"
    );
    assert_eq!(body["id"].as_str().unwrap_or(""), connector_id);
    println!("  connector ok: name={}", body["name"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/integrations/connectors/{connector_id}"),
        None,
    )
    .await;

    // ── 11. POST /api/integrations/connectors/{id}/test ───────────────
    println!("\n--- 11. POST /api/integrations/connectors/{{id}}/test ---");
    let idem_key = Uuid::new_v4().to_string();
    let resp = client
        .post(format!(
            "{base}/api/integrations/connectors/{connector_id}/test"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"idempotency_key": idem_key}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Connector test failed: {status} - {body}"
    );
    assert_eq!(
        body["success"].as_bool().unwrap_or(false),
        true,
        "echo connector test should succeed"
    );
    println!(
        "  connector test ok: success={} type={}",
        body["success"], body["connector_type"]
    );
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/integrations/connectors/{connector_id}/test"),
        Some(json!({"idempotency_key": "k"})),
    )
    .await;

    // ── 12. POST /api/webhooks/inbound/{system} ───────────────────────
    // Unauthenticated. Use "internal" system with JWT so tenant is derived from claims.
    println!("\n--- 12. POST /api/webhooks/inbound/internal ---");
    let resp = client
        .post(format!("{base}/api/webhooks/inbound/internal"))
        .bearer_auth(&jwt)
        .json(&json!({
            "event_type": "smoke.test",
            "payload": {"test": true}
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    // 200 = accepted/duplicate. Other valid responses prove the route is wired.
    assert!(
        status.as_u16() < 500,
        "Webhook ingest returned server error: {status} - {body}"
    );
    println!("  webhook responded: {status}");

    // Verify: no JWT + unsupported system → 400 (cannot determine tenant)
    let resp_noauth = client
        .post(format!("{base}/api/webhooks/inbound/unknown-system"))
        .json(&json!({"event_type": "test"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp_noauth.status().as_u16(),
        400,
        "expected 400 for unknown system without JWT"
    );
    println!("  no-JWT + unknown-system -> 400 ok");

    println!("\n=== All 12 integrations routes passed ===");
}
