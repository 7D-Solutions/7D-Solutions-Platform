// HTTP smoke tests: Inventory Items CRUD
//
// Proves that 14 core inventory routes respond correctly at the HTTP
// boundary via reqwest against the live Inventory service. Each route
// tested for happy path; auth enforcement verified separately.

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
        "PUT" => client.put(url),
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
async fn smoke_inventory_items_crud() {
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

    // --- 1. POST /api/inventory/uoms ---
    println!("\n--- 1. POST /api/inventory/uoms ---");
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
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Create UOM failed: {status} - {body}");
    let ea_uom_id = body["id"].as_str().expect("No id in UOM response").to_string();
    println!("  created UOM: {body}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/inventory/uoms"),
        Some(json!({"name": "X", "code": "X"})),
    )
    .await;

    // --- 2. POST /api/inventory/items ---
    println!("\n--- 2. POST /api/inventory/items ---");
    let sku = format!("SMOKE-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/inventory/items"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "sku": sku,
            "name": "Smoke Test Item",
            "description": "Created by smoke test",
            "inventory_account_ref": "1200",
            "cogs_account_ref": "5000",
            "variance_account_ref": "5100",
            "uom": "EA",
            "tracking_mode": "lot",
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

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/inventory/items"),
        Some(json!({})),
    )
    .await;

    // --- 3. GET /api/inventory/items/{id} ---
    println!("\n--- 3. GET /api/inventory/items/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/inventory/items/{item_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "Get item failed: {}", resp.status());
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["name"], "Smoke Test Item");
    println!("  retrieved item name={}", fetched["name"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/inventory/items/{item_id}"),
        None,
    )
    .await;

    // --- 4. PUT /api/inventory/items/{id}/make-buy ---
    println!("\n--- 4. PUT /api/inventory/items/{{id}}/make-buy ---");
    let resp = client
        .put(format!("{base}/api/inventory/items/{item_id}/make-buy"))
        .bearer_auth(&jwt)
        .json(&json!({
            "make_buy": "make",
            "tenant_id": tenant_id,
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(status.is_success(), "Set make/buy failed: {status} - {body_text}");
    println!("  set make/buy to 'make'");

    assert_unauth(
        &client,
        "PUT",
        &format!("{base}/api/inventory/items/{item_id}/make-buy"),
        Some(json!({"make_buy": "buy"})),
    )
    .await;

    // --- 5. POST /api/inventory/items/{item_id}/revisions ---
    println!("\n--- 5. POST /api/inventory/items/{{item_id}}/revisions ---");
    let resp = client
        .post(format!("{base}/api/inventory/items/{item_id}/revisions"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "name": "Rev A",
            "description": "Initial revision",
            "uom": "EA",
            "inventory_account_ref": "1200",
            "cogs_account_ref": "5000",
            "variance_account_ref": "5100",
            "change_reason": "Initial revision creation",
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let rev: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Create revision failed: {status} - {rev}");
    let revision_id = rev["id"]
        .as_str()
        .or(rev["revision_id"].as_str())
        .expect("No revision id");
    println!("  created revision id={revision_id}");

    // --- 6. GET /api/inventory/items/{item_id}/revisions/at ---
    println!("\n--- 6. GET /api/inventory/items/{{item_id}}/revisions/at ---");
    let at = Utc::now().to_rfc3339();
    let resp = client
        .get(format!(
            "{base}/api/inventory/items/{item_id}/revisions/at?at={at}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let s = resp.status();
    assert!(
        s == StatusCode::OK || s == StatusCode::NOT_FOUND,
        "Revision-at unexpected status: {s}"
    );
    println!("  revision-at returned {s}");

    // --- 7. PUT /api/inventory/items/{item_id}/revisions/{revision_id}/policy-flags ---
    println!("\n--- 7. PUT .../revisions/{{revision_id}}/policy-flags ---");
    let resp = client
        .put(format!(
            "{base}/api/inventory/items/{item_id}/revisions/{revision_id}/policy-flags"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "traceability_level": "lot",
            "inspection_required": false,
            "shelf_life_enforced": false,
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Update policy-flags failed: {status} - {body_text}"
    );
    println!("  updated policy-flags");

    // --- 8. POST /api/inventory/items/{item_id}/revisions/{revision_id}/activate ---
    println!("\n--- 8. POST .../revisions/{{revision_id}}/activate ---");
    let resp = client
        .post(format!(
            "{base}/api/inventory/items/{item_id}/revisions/{revision_id}/activate"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "effective_from": Utc::now().to_rfc3339(),
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Activate revision failed: {status} - {body_text}"
    );
    println!("  activated revision");

    // --- 9. POST /api/inventory/items/{item_id}/labels ---
    println!("\n--- 9. POST /api/inventory/items/{{item_id}}/labels ---");
    let resp = client
        .post(format!("{base}/api/inventory/items/{item_id}/labels"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "revision_id": revision_id,
            "label_type": "item_label",
            "barcode_format": "code128",
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let label: Value = resp.json().await.unwrap_or(json!({}));
    assert!(status.is_success(), "Generate label failed: {status} - {label}");
    let label_id = label["id"]
        .as_str()
        .or(label["label_id"].as_str())
        .expect("No label id");
    println!("  generated label id={label_id}");

    // --- 10. GET /api/inventory/labels/{label_id} ---
    println!("\n--- 10. GET /api/inventory/labels/{{label_id}} ---");
    let resp = client
        .get(format!("{base}/api/inventory/labels/{label_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "Get label failed: {}", resp.status());
    println!("  retrieved label");

    // --- 11. GET /api/inventory/items/{item_id}/history ---
    println!("\n--- 11. GET /api/inventory/items/{{item_id}}/history ---");
    let resp = client
        .get(format!("{base}/api/inventory/items/{item_id}/history"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get history failed: {}",
        resp.status()
    );
    println!("  retrieved movement history");

    // --- 12. POST /api/inventory/items/{id}/uom-conversions ---
    println!("\n--- 12. POST .../uom-conversions ---");
    // Create a second UOM
    let resp = client
        .post(format!("{base}/api/inventory/uoms"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "name": "Dozen",
            "code": "DZ",
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let dz_status = resp.status();
    let dz_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(dz_status.is_success(), "Create UOM2 failed: {dz_status} - {dz_body}");
    let dz_uom_id = dz_body["id"].as_str().expect("No id in UOM2 response").to_string();

    let resp = client
        .post(format!(
            "{base}/api/inventory/items/{item_id}/uom-conversions"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "from_uom_id": ea_uom_id,
            "to_uom_id": dz_uom_id,
            "factor": 12.0,
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Create UOM conversion failed: {status} - {body_text}"
    );
    println!("  created UOM conversion EA->DZ");

    // --- 13. POST /api/inventory/reorder-policies ---
    println!("\n--- 13. POST /api/inventory/reorder-policies ---");
    let resp = client
        .post(format!("{base}/api/inventory/reorder-policies"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "item_id": item_id,
            "reorder_point": 10,
            "safety_stock": 5,
            "max_qty": 100,
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Create reorder policy failed: {status} - {body_text}"
    );
    println!("  created reorder policy");

    // --- 14. POST /api/inventory/items/{id}/deactivate ---
    println!("\n--- 14. POST /api/inventory/items/{{id}}/deactivate ---");
    let resp = client
        .post(format!("{base}/api/inventory/items/{item_id}/deactivate"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    assert!(
        status.is_success(),
        "Deactivate item failed: {status} - {body_text}"
    );
    println!("  deactivated item");

    println!("\n=== All 14 inventory item routes passed ===");
}
