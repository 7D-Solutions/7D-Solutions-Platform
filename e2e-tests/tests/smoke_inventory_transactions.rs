// HTTP smoke tests: Inventory Transactions
//
// Proves that 10 core inventory transaction routes respond correctly at the
// HTTP boundary via reqwest against the live Inventory service.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const INV_DEFAULT_URL: &str = "http://localhost:8092";

fn inv_url() -> String {
    std::env::var("INVENTORY_URL").unwrap_or_else(|_| INV_DEFAULT_URL.to_string())
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

async fn wait_for_inventory(client: &Client) -> bool {
    let url = format!("{}/api/health", inv_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Inventory health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Inventory health {}/15: {}", attempt, e),
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
async fn smoke_inventory_transactions() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_inventory(&client).await {
        eprintln!(
            "Inventory service not reachable at {} -- skipping",
            inv_url()
        );
        return;
    }
    println!("Inventory service healthy at {}", inv_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["inventory.mutate", "inventory.read"],
    );
    let base = inv_url();

    // Gate: verify the Inventory service accepts our JWT
    let probe = client
        .get(format!("{base}/api/inventory/uoms"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Inventory returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    println!("\n--- Setup: create UOM ---");
    let resp = client
        .post(format!("{base}/api/inventory/uoms"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "name": "Each",
            "code": "EA",
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Create UOM failed: {}",
        resp.status()
    );
    println!("  created UOM EA");

    println!("\n--- Setup: create item ---");
    let sku = format!("TXSMOKE-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/inventory/items"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "sku": sku,
            "name": "Transaction Smoke Item",
            "description": "Created by transaction smoke test",
            "inventory_account_ref": "1200",
            "cogs_account_ref": "5000",
            "variance_account_ref": "5100",
            "uom": "EA",
            "tracking_mode": "none",
            "make_buy": "buy",
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let created: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Create item failed: {status} - {created}");
    let item_id = created["id"].as_str().expect("No id in create response");
    println!("  created item id={item_id}");

    let warehouse_a = Uuid::new_v4();
    let warehouse_b = Uuid::new_v4();

    // 1. POST /api/inventory/receipts
    println!("\n--- 1. POST /api/inventory/receipts ---");
    let resp = client
        .post(format!("{base}/api/inventory/receipts"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "warehouse_id": warehouse_a,
            "quantity": 100,
            "unit_cost_minor": 1500,
            "currency": "USD",
            "source_type": "purchase",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create receipt failed: {status} - {body}"
    );
    println!("  receipt created: {status}");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/receipts"), Some(json!({}))).await;
    // 2. POST /api/inventory/issues
    println!("\n--- 2. POST /api/inventory/issues ---");
    let resp = client
        .post(format!("{base}/api/inventory/issues"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "warehouse_id": warehouse_a,
            "quantity": 10,
            "currency": "USD",
            "source_module": "smoke-test",
            "source_type": "manual",
            "source_id": Uuid::new_v4().to_string(),
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create issue failed: {status} - {body}"
    );
    println!("  issue created: {status}");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/issues"), Some(json!({}))).await;
    // 3. POST /api/inventory/adjustments
    println!("\n--- 3. POST /api/inventory/adjustments ---");
    let resp = client
        .post(format!("{base}/api/inventory/adjustments"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "warehouse_id": warehouse_a,
            "quantity_delta": 5,
            "reason": "cycle_count_correction",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create adjustment failed: {status} - {body}"
    );
    println!("  adjustment created: {status}");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/adjustments"), Some(json!({}))).await;
    // 4. POST /api/inventory/transfers
    println!("\n--- 4. POST /api/inventory/transfers ---");
    let resp = client
        .post(format!("{base}/api/inventory/transfers"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "from_warehouse_id": warehouse_a,
            "to_warehouse_id": warehouse_b,
            "quantity": 20,
            "currency": "USD",
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create transfer failed: {status} - {body}"
    );
    println!("  transfer created: {status}");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/transfers"), Some(json!({}))).await;
    // 5. POST /api/inventory/status-transfers
    println!("\n--- 5. POST /api/inventory/status-transfers ---");
    let resp = client
        .post(format!("{base}/api/inventory/status-transfers"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "warehouse_id": warehouse_a,
            "from_status": "available",
            "to_status": "quarantine",
            "quantity": 5,
            "idempotency_key": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Status transfer failed: {status} - {body}"
    );
    println!("  status transfer created: {status}");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/status-transfers"), Some(json!({}))).await;
    // 6. GET /api/inventory/uoms
    println!("\n--- 6. GET /api/inventory/uoms ---");
    let resp = client
        .get(format!("{base}/api/inventory/uoms"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List UOMs failed: {status}");
    assert!(body.is_array(), "UOMs response should be an array");
    println!("  listed {} UOMs", body.as_array().map(|a| a.len()).unwrap_or(0));
    assert_unauth(&client, "GET", &format!("{base}/api/inventory/uoms"), None).await;
    // 7. POST /api/inventory/locations
    println!("\n--- 7. POST /api/inventory/locations ---");
    let resp = client
        .post(format!("{base}/api/inventory/locations"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "warehouse_id": warehouse_a,
            "code": format!("BIN-{}", &Uuid::new_v4().to_string()[..6]),
            "name": "Smoke Test Bin A1"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let loc: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create location failed: {status} - {loc}"
    );
    let location_id = loc["id"]
        .as_str()
        .or(loc["location_id"].as_str())
        .expect("No location id in response");
    println!("  created location id={location_id}");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/locations"), Some(json!({"warehouse_id": warehouse_a, "code": "X", "name": "X"}))).await;
    // 8. GET /api/inventory/locations/{id}
    println!("\n--- 8. GET /api/inventory/locations/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/inventory/locations/{location_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Get location failed: {status}");
    let fetched: Value = resp.json().await.unwrap_or(json!({}));
    println!("  retrieved location name={}", fetched["name"]);
    assert_unauth(&client, "GET", &format!("{base}/api/inventory/locations/{location_id}"), None).await;
    // 9. POST /api/inventory/locations/{id}/deactivate
    println!("\n--- 9. POST /api/inventory/locations/{{id}}/deactivate ---");
    let resp = client
        .post(format!("{base}/api/inventory/locations/{location_id}/deactivate"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Deactivate location failed: {status}");
    println!("  deactivated location");
    assert_unauth(&client, "POST", &format!("{base}/api/inventory/locations/{location_id}/deactivate"), None).await;
    // 10. GET /api/inventory/warehouses/{warehouse_id}/locations
    println!("\n--- 10. GET /api/inventory/warehouses/{{warehouse_id}}/locations ---");
    let resp = client
        .get(format!("{base}/api/inventory/warehouses/{warehouse_a}/locations"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "List warehouse locations failed: {status}");
    assert!(body.is_array(), "Warehouse locations should be an array");
    println!("  listed {} locations for warehouse", body.as_array().map(|a| a.len()).unwrap_or(0));
    assert_unauth(&client, "GET", &format!("{base}/api/inventory/warehouses/{warehouse_a}/locations"), None).await;

    println!("\n=== All 10 inventory transaction routes passed ===");
}
