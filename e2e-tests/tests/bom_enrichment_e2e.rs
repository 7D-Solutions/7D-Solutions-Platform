// E2E: BOM line enrichment — ?include=item_details
//
// Proves the three invariants from bd-7lb9x:
// 1. Without ?include, GET /api/bom/revisions/{id}/lines returns bare BomLine objects
//    (no `item` key in the JSON — backward compatible).
// 2. With ?include=item_details, each line includes an embedded `item` object
//    with sku, name, description, and unit_cost_minor.
// 3. An unresolvable component_item_id returns `item: null` — not 500.
//
// Requires: live BOM service (8107) and Inventory service (8092).
// The BOM service calls Inventory server-side when enrichment is requested.
// No mocks. Real Postgres.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const BOM_DEFAULT_URL: &str = "http://localhost:8107";
const INV_DEFAULT_URL: &str = "http://localhost:8092";

fn bom_url() -> String {
    std::env::var("BOM_URL").unwrap_or_else(|_| BOM_DEFAULT_URL.to_string())
}

fn inv_url() -> String {
    std::env::var("INVENTORY_URL").unwrap_or_else(|_| INV_DEFAULT_URL.to_string())
}

// ── JWT helpers ───────────────────────────────────────────────────────────────

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

// ── Service health checks ─────────────────────────────────────────────────────

async fn wait_for_bom(client: &Client) -> bool {
    let url = format!("{}/api/health", bom_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  BOM health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  BOM health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn wait_for_inventory(client: &Client) -> bool {
    let url = format!("{}/api/health", inv_url());
    for attempt in 1..=10 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Inventory health {}/10: {}", attempt, r.status()),
            Err(e) => eprintln!("  Inventory health {}/10: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

// ── Setup helpers ─────────────────────────────────────────────────────────────

/// Create an inventory item and return its UUID.
async fn create_inventory_item(
    client: &Client,
    jwt: &str,
    tenant_id: &str,
    sku: &str,
    name: &str,
) -> Option<Uuid> {
    let resp = client
        .post(format!("{}/api/inventory/items", inv_url()))
        .bearer_auth(jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "sku": sku,
            "name": name,
            "description": "BOM enrichment test item",
            "inventory_account_ref": "1200",
            "cogs_account_ref": "5000",
            "variance_account_ref": "5010",
            "uom": "EA",
            "tracking_mode": "none",
            "idempotency_key": Uuid::new_v4()
        }))
        .send()
        .await
        .expect("create inventory item request");
    if !resp.status().is_success() {
        let s = resp.status();
        let b: Value = resp.json().await.unwrap_or(json!({}));
        eprintln!("  create inventory item failed: {} - {}", s, b);
        return None;
    }
    let body: Value = resp.json().await.expect("inventory item body");
    let id = body["id"].as_str()?;
    Some(Uuid::parse_str(id).expect("valid UUID"))
}

/// Create a BOM header and return its UUID.
async fn create_bom(client: &Client, jwt: &str, part_id: Uuid, description: &str) -> Uuid {
    let resp = client
        .post(format!("{}/api/bom", bom_url()))
        .bearer_auth(jwt)
        .json(&json!({ "part_id": part_id, "description": description }))
        .send()
        .await
        .expect("create BOM");
    let s = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert_eq!(s, StatusCode::CREATED, "create BOM: {}", body);
    Uuid::parse_str(body["id"].as_str().expect("id in BOM")).unwrap()
}

/// Create a BOM revision and return its UUID.
async fn create_revision(client: &Client, jwt: &str, bom_id: Uuid) -> Uuid {
    let resp = client
        .post(format!("{}/api/bom/{}/revisions", bom_url(), bom_id))
        .bearer_auth(jwt)
        .json(&json!({ "revision_label": "A" }))
        .send()
        .await
        .expect("create revision");
    let s = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert_eq!(s, StatusCode::CREATED, "create revision: {}", body);
    Uuid::parse_str(body["id"].as_str().expect("id in revision")).unwrap()
}

/// Add a BOM line and return the response body.
async fn add_bom_line(
    client: &Client,
    jwt: &str,
    revision_id: Uuid,
    component_item_id: Uuid,
    quantity: f64,
) -> Value {
    let resp = client
        .post(format!(
            "{}/api/bom/revisions/{}/lines",
            bom_url(),
            revision_id
        ))
        .bearer_auth(jwt)
        .json(&json!({
            "component_item_id": component_item_id,
            "quantity": quantity,
            "uom": "EA"
        }))
        .send()
        .await
        .expect("add BOM line");
    let s = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert_eq!(s, StatusCode::CREATED, "add BOM line: {}", body);
    body
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Test 1: GET without ?include returns bare BomLine — no `item` key in response.
/// Backward compatibility invariant: existing consumers must not break.
#[tokio::test]
async fn backward_compat_without_include() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_bom(&client).await {
        eprintln!("BOM service not reachable at {} -- skipping", bom_url());
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["bom.mutate", "bom.read"]);

    // JWT gate
    let probe = client
        .get(format!("{}/api/bom/{}", bom_url(), Uuid::new_v4()))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("BOM returns 401 with JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // Seed: BOM + revision + one line (item UUID need not exist in inventory)
    let part_id = Uuid::new_v4();
    let component_id = Uuid::new_v4();
    let bom_id = create_bom(&client, &jwt, part_id, "backward-compat test BOM").await;
    let revision_id = create_revision(&client, &jwt, bom_id).await;
    add_bom_line(&client, &jwt, revision_id, component_id, 2.0).await;

    // GET lines WITHOUT ?include
    let resp = client
        .get(format!(
            "{}/api/bom/revisions/{}/lines",
            bom_url(),
            revision_id
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("GET lines");
    assert_eq!(resp.status(), StatusCode::OK, "GET lines -> 200");

    let body: Value = resp.json().await.expect("lines body");
    let items = body["data"]
        .as_array()
        .expect("data array in paginated response");
    assert!(!items.is_empty(), "should have at least one line");

    let line = &items[0];
    assert!(
        line.get("item").is_none(),
        "bare response must NOT have 'item' key — got: {}",
        line
    );
    assert!(
        line.get("component_item_id").is_some(),
        "component_item_id must be present in bare response"
    );
    println!("  backward_compat_without_include: PASS — no 'item' key on bare line");
}

/// Test 2: GET with ?include=item_details embeds the item object on each line.
/// Requires both BOM (8107) and Inventory (8092) services running.
#[tokio::test]
async fn enriched_item_details_returned() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_bom(&client).await {
        eprintln!("BOM service not reachable at {} -- skipping", bom_url());
        return;
    }
    if !wait_for_inventory(&client).await {
        eprintln!(
            "Inventory service not reachable at {} -- skipping enrichment test",
            inv_url()
        );
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let bom_jwt = make_jwt(
        &key,
        &tenant_id,
        &["bom.mutate", "bom.read", "inventory.read"],
    );
    let inv_jwt = make_jwt(&key, &tenant_id, &["inventory.mutate", "inventory.read"]);

    // JWT gate for BOM
    let probe = client
        .get(format!("{}/api/bom/{}", bom_url(), Uuid::new_v4()))
        .bearer_auth(&bom_jwt)
        .send()
        .await
        .expect("JWT probe BOM");
    if probe.status().as_u16() == 401 {
        eprintln!("BOM returns 401 with JWT -- skipping");
        return;
    }

    // JWT gate for inventory
    let probe_inv = client
        .get(format!("{}/api/inventory/uoms", inv_url()))
        .bearer_auth(&inv_jwt)
        .send()
        .await
        .expect("JWT probe Inventory");
    if probe_inv.status().as_u16() == 401 {
        eprintln!("Inventory returns 401 with JWT -- skipping enrichment test");
        return;
    }

    let sku = format!("BOM-ENRICH-{}", &Uuid::new_v4().to_string()[..8]);
    let Some(item_id) =
        create_inventory_item(&client, &inv_jwt, &tenant_id, &sku, "BOM enrichment item").await
    else {
        eprintln!("Could not create inventory item -- skipping enrichment test");
        return;
    };
    println!("  Created inventory item: {item_id}");

    // Seed BOM with line pointing to real inventory item
    let part_id = Uuid::new_v4();
    let bom_id = create_bom(&client, &bom_jwt, part_id, "enrichment test BOM").await;
    let revision_id = create_revision(&client, &bom_jwt, bom_id).await;
    add_bom_line(&client, &bom_jwt, revision_id, item_id, 3.0).await;

    // GET with ?include=item_details
    let resp = client
        .get(format!(
            "{}/api/bom/revisions/{}/lines?include=item_details",
            bom_url(),
            revision_id
        ))
        .bearer_auth(&bom_jwt)
        .send()
        .await
        .expect("GET enriched lines");
    assert_eq!(resp.status(), StatusCode::OK, "GET enriched lines -> 200");

    let body: Value = resp.json().await.expect("enriched lines body");
    let items_arr = body["data"].as_array().expect("data array");
    assert!(
        !items_arr.is_empty(),
        "should have at least one enriched line"
    );

    let line = &items_arr[0];
    let item = line
        .get("item")
        .expect("'item' key must be present in enriched response");
    assert!(
        !item.is_null(),
        "item must not be null for a valid inventory item: {}",
        line
    );

    // Verify item object has the required fields
    assert_eq!(
        item["sku"],
        json!(sku),
        "item.sku should match created item"
    );
    assert_eq!(
        item["name"],
        json!("BOM enrichment item"),
        "item.name should match created item"
    );
    assert!(
        item.get("description").is_some(),
        "item.description field must be present"
    );
    // unit_cost_minor may be null — inventory API v1 does not expose it
    assert!(
        item.get("unit_cost_minor").is_some(),
        "unit_cost_minor key must be present (may be null)"
    );

    println!("  enriched_item_details_returned: PASS — item: {}", item);
}

/// Test 3: Unresolvable component_item_id returns `item: null` — not 500.
/// The BOM service must gracefully handle items that do not exist in inventory.
#[tokio::test]
async fn null_item_for_unresolvable_part_id() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_bom(&client).await {
        eprintln!("BOM service not reachable at {} -- skipping", bom_url());
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["bom.mutate", "bom.read"]);

    // JWT gate
    let probe = client
        .get(format!("{}/api/bom/{}", bom_url(), Uuid::new_v4()))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("BOM returns 401 with JWT -- skipping");
        return;
    }

    // Line points to a random UUID that will never exist in inventory
    let nonexistent_item_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();
    let bom_id = create_bom(&client, &jwt, part_id, "null-item test BOM").await;
    let revision_id = create_revision(&client, &jwt, bom_id).await;
    add_bom_line(&client, &jwt, revision_id, nonexistent_item_id, 1.0).await;

    // GET with ?include=item_details — must return 200 with item: null
    let resp = client
        .get(format!(
            "{}/api/bom/revisions/{}/lines?include=item_details",
            bom_url(),
            revision_id
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("GET enriched lines (null item)");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "unresolvable part_id must return 200, not 500"
    );

    let body: Value = resp.json().await.expect("null-item lines body");
    let items_arr = body["data"].as_array().expect("data array");
    assert!(!items_arr.is_empty(), "should have one line");

    let line = &items_arr[0];
    let item = line
        .get("item")
        .expect("'item' key must be present even when unresolvable");
    assert!(
        item.is_null(),
        "item must be null for unresolvable part_id — got: {}",
        item
    );

    println!("  null_item_for_unresolvable_part_id: PASS — item: null as expected");
}
