// E2E: Shipping-Receiving — receipts, shipments, inventory integration
//
// Tests the full inbound and outbound lifecycles via real HTTP API calls
// against the live shipping-receiving service (port 8103). After closing
// an inbound shipment, verifies inventory_ref_id IS NOT NULL on all lines
// in the SR database (proving the inventory integration fired).
//
// No mocks, no stubs — real Docker containers required.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;

const SR_DEFAULT_URL: &str = "http://localhost:8103";

fn sr_url() -> String {
    std::env::var("SHIPPING_RECEIVING_URL").unwrap_or_else(|_| SR_DEFAULT_URL.to_string())
}

fn sr_db_url() -> String {
    std::env::var("SHIPPING_RECEIVING_DB_URL").unwrap_or_else(|_| {
        "postgresql://shipping_receiving_user:shipping_receiving_pass@localhost:5454/shipping_receiving_db".to_string()
    })
}

fn inventory_db_url() -> String {
    std::env::var("INVENTORY_DB_URL").unwrap_or_else(|_| {
        "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
    })
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

fn make_jwt(key: &EncodingKey, tenant_id: &str) -> String {
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
        perms: vec!["shipping_receiving.mutate".to_string()],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

async fn wait_for_sr(client: &Client) -> bool {
    let url = format!("{}/healthz", sr_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  SR health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  SR health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

fn extract_uuid(body: &Value, key: &str) -> Uuid {
    Uuid::parse_str(
        body.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("missing '{}' in: {}", key, body)),
    )
    .unwrap_or_else(|_| panic!("invalid UUID for '{}' in: {}", key, body))
}

async fn get_sr_pool() -> Option<sqlx::PgPool> {
    let url = sr_db_url();
    match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => {
            // Run migrations so schema is current
            let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(
                "../modules/shipping-receiving/db/migrations",
            ))
            .await
            .ok()?;
            migrator.run(&pool).await.ok()?;
            Some(pool)
        }
        Err(e) => {
            eprintln!("SR DB not reachable ({}): {}", url, e);
            None
        }
    }
}

async fn try_get_inventory_pool() -> Option<sqlx::PgPool> {
    let url = inventory_db_url();
    match PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&url)
        .await
    {
        Ok(pool) => Some(pool),
        Err(e) => {
            eprintln!("Inventory DB not reachable (skipping inv check): {}", e);
            None
        }
    }
}

async fn transition(
    client: &Client,
    base: &str,
    jwt: &str,
    shipment_id: Uuid,
    to_status: &str,
) -> Value {
    let mut payload = json!({"status": to_status});
    if to_status == "arrived" {
        payload["arrived_at"] = json!(Utc::now().to_rfc3339());
    }
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/status"
        ))
        .bearer_auth(jwt)
        .json(&payload)
        .send()
        .await
        .unwrap_or_else(|e| panic!("transition to {to_status} failed: {e}"));
    let status = resp.status();
    let body: Value = resp.json().await.expect("transition body");
    assert_eq!(
        status,
        StatusCode::OK,
        "transition to {to_status}: {}",
        body
    );
    assert_eq!(
        body["status"], to_status,
        "status mismatch after transition"
    );
    println!("  -> {to_status} ok");
    body
}

async fn cleanup_sr(pool: &sqlx::PgPool, tenant_id: &str) {
    let tid = uuid::Uuid::parse_str(tenant_id).unwrap();
    sqlx::query("DELETE FROM sr_events_outbox WHERE tenant_id = $1::text")
        .bind(tid)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM shipment_line_routings WHERE tenant_id = $1")
        .bind(tid)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM shipment_lines WHERE tenant_id = $1")
        .bind(tid)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM shipments WHERE tenant_id = $1")
        .bind(tid)
        .execute(pool)
        .await
        .ok();
}

// ── Test 1: Full inbound lifecycle + inventory integration ───────────────────

#[tokio::test]
async fn inbound_receipt_and_inventory_integration() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_sr(&client).await {
        eprintln!("SR service not reachable at {} -- skipping", sr_url());
        return;
    }
    println!("SR service healthy");

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id);
    let base = sr_url();

    // JWT gate
    let probe = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{}",
            Uuid::new_v4()
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("SR returns 401 with JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // SR DB pool (needed for inventory_ref_id verification)
    let sr_pool = match get_sr_pool().await {
        Some(p) => p,
        None => {
            eprintln!("SR DB not reachable -- skipping DB verification");
            return;
        }
    };

    let po_id = Uuid::new_v4();
    let po_line_id = Uuid::new_v4();

    // Step 1: Create inbound shipment
    println!("\n-- Step 1: Create inbound shipment --");
    let resp = client
        .post(format!("{base}/api/shipping-receiving/shipments"))
        .bearer_auth(&jwt)
        .json(&json!({
            "direction": "inbound",
            "tracking_number": format!("E2E-INBOUND-{}", &tenant_id[..8]),
            "currency": "USD"
        }))
        .send()
        .await
        .expect("create inbound");
    let s = resp.status();
    let body: Value = resp.json().await.expect("create inbound body");
    assert_eq!(s, StatusCode::CREATED, "create inbound: {}", body);
    let shipment_id = extract_uuid(&body, "id");
    assert_eq!(body["status"], "draft");
    println!("  created {} (draft)", shipment_id);

    // Step 2: Add PO-linked line
    println!("\n-- Step 2: Add PO-linked line --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "sku": "E2E-SKU-001",
            "uom": "EA",
            "qty_expected": 50,
            "po_id": po_id,
            "po_line_id": po_line_id,
            "source_ref_type": "purchase_order",
            "source_ref_id": po_id
        }))
        .send()
        .await
        .expect("add line 1");
    let s = resp.status();
    let body: Value = resp.json().await.expect("add line 1 body");
    assert_eq!(s, StatusCode::CREATED, "add PO line: {}", body);
    let line1_id = extract_uuid(&body, "id");
    println!("  line 1 (PO-linked): {}", line1_id);

    // Step 3: Add blind line
    println!("\n-- Step 3: Add blind line --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "sku": "E2E-SKU-002",
            "uom": "EA",
            "qty_expected": 25
        }))
        .send()
        .await
        .expect("add blind line");
    let s = resp.status();
    let body: Value = resp.json().await.expect("blind line body");
    assert_eq!(s, StatusCode::CREATED, "add blind line: {}", body);
    let line2_id = extract_uuid(&body, "id");
    println!("  line 2 (blind): {}", line2_id);

    // Step 4-7: confirmed → in_transit → arrived → receiving
    println!("\n-- Steps 4-7: confirmed → in_transit → arrived → receiving --");
    transition(&client, &base, &jwt, shipment_id, "confirmed").await;
    transition(&client, &base, &jwt, shipment_id, "in_transit").await;
    transition(&client, &base, &jwt, shipment_id, "arrived").await;
    transition(&client, &base, &jwt, shipment_id, "receiving").await;

    // Step 8: Receive line 1 (full quantity)
    println!("\n-- Step 8: Receive line 1 --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines/{line1_id}/receive"
        ))
        .bearer_auth(&jwt)
        .json(&json!({ "qty_received": 50, "qty_accepted": 50, "qty_rejected": 0 }))
        .send()
        .await
        .expect("receive line 1");
    let s = resp.status();
    let body: Value = resp.json().await.expect("receive line 1 body");
    assert_eq!(s, StatusCode::OK, "receive line 1: {}", body);
    assert_eq!(body["qty_received"], 50);
    println!("  received line 1 (qty=50)");

    // Step 9: Receive line 2 (full quantity)
    println!("\n-- Step 9: Receive line 2 --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines/{line2_id}/receive"
        ))
        .bearer_auth(&jwt)
        .json(&json!({ "qty_received": 25, "qty_accepted": 25, "qty_rejected": 0 }))
        .send()
        .await
        .expect("receive line 2");
    let s = resp.status();
    let body: Value = resp.json().await.expect("receive line 2 body");
    assert_eq!(s, StatusCode::OK, "receive line 2: {}", body);
    println!("  received line 2 (qty=25)");

    // Step 10: Accept line 1
    println!("\n-- Step 10: Accept line 1 --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines/{line1_id}/accept"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("accept line 1");
    let s = resp.status();
    let body: Value = resp.json().await.expect("accept line 1 body");
    assert_eq!(s, StatusCode::OK, "accept line 1: {}", body);
    println!("  accepted line 1");

    // Step 11: Accept line 2
    println!("\n-- Step 11: Accept line 2 --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines/{line2_id}/accept"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("accept line 2");
    let s = resp.status();
    let body: Value = resp.json().await.expect("accept line 2 body");
    assert_eq!(s, StatusCode::OK, "accept line 2: {}", body);
    println!("  accepted line 2");

    // Step 12: Route both lines
    println!("\n-- Step 12: Route lines --");
    for lid in [line1_id, line2_id] {
        let resp = client
            .post(format!(
                "{base}/api/shipping-receiving/shipments/{shipment_id}/lines/{lid}/route"
            ))
            .bearer_auth(&jwt)
            .json(&json!({ "route_decision": "direct_to_stock" }))
            .send()
            .await
            .expect("route line");
        let s = resp.status();
        let body: Value = resp.json().await.expect("route body");
        assert_eq!(s, StatusCode::CREATED, "route line {lid}: {}", body);
        println!("  routed line {lid} -> direct_to_stock");
    }

    // Step 13: Close inbound — triggers inventory integration
    println!("\n-- Step 13: Close inbound (triggers inventory integration) --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/close"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("close inbound");
    let s = resp.status();
    let body: Value = resp.json().await.expect("close body");
    assert_eq!(s, StatusCode::OK, "close inbound: {}", body);
    assert_eq!(body["status"], "closed");
    println!("  closed -> status=closed");

    // Step 14: GET shipment — verify closed
    println!("\n-- Step 14: GET shipment -- verify closed --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get shipment");
    let body: Value = resp.json().await.expect("get body");
    assert_eq!(body["status"], "closed");
    println!("  GET /shipments/{shipment_id} -> closed ok");

    // Step 15: GET /po/{po_id}/shipments — verify PO linkage
    println!("\n-- Step 15: GET /po/{po_id}/shipments --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/po/{po_id}/shipments"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get po shipments");
    let s = resp.status();
    let body: Value = resp.json().await.expect("po shipments body");
    assert_eq!(s, StatusCode::OK, "po shipments: {}", body);
    let arr = body.as_array().expect("po shipments should be array");
    assert!(
        !arr.is_empty(),
        "expected at least one shipment for po_id {po_id}"
    );
    println!("  /po/{po_id}/shipments -> {} shipment(s)", arr.len());

    // Step 16: Verify inventory_ref_id IS NOT NULL on all lines (SR DB)
    println!("\n-- Step 16: Verify inventory_ref_id IS NOT NULL (SR DB) --");
    let rows: Vec<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, inventory_ref_id FROM shipment_lines WHERE shipment_id = $1 ORDER BY id",
    )
    .bind(shipment_id)
    .fetch_all(&sr_pool)
    .await
    .expect("query lines");

    assert_eq!(rows.len(), 2, "expected 2 lines, got {}", rows.len());
    for (line_id, inv_ref) in &rows {
        assert!(
            inv_ref.is_some(),
            "line {line_id}: inventory_ref_id IS NULL — inventory integration did not fire"
        );
        println!("  line {line_id} -> inventory_ref_id={}", inv_ref.unwrap());
    }
    println!("  inventory_ref_id set on all lines -- integration confirmed");

    // Step 17: Verify outbox event published
    println!("\n-- Step 17: Verify outbox event --");
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sr_events_outbox WHERE aggregate_id = $1::text AND event_type LIKE '%closed%'",
    )
    .bind(shipment_id)
    .fetch_one(&sr_pool)
    .await
    .expect("outbox query");
    assert!(
        event_count > 0,
        "no 'closed' outbox event for shipment {shipment_id}"
    );
    println!("  outbox: {event_count} closed event(s) found");

    // Optional: check inventory DB if reachable
    if let Some(inv_pool) = try_get_inventory_pool().await {
        println!("\n-- Optional: Inventory DB check --");
        let inv_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM inventory_ledger WHERE tenant_id = $1")
                .bind(uuid::Uuid::parse_str(&tenant_id).unwrap())
                .fetch_one(&inv_pool)
                .await
                .unwrap_or(0);
        println!("  inventory_ledger entries for tenant: {inv_count}");
    }

    // Cleanup
    cleanup_sr(&sr_pool, &tenant_id).await;
    println!("\n✓ inbound_receipt_and_inventory_integration passed");
}

// ── Test 2: Full outbound lifecycle + inventory issue ────────────────────────

#[tokio::test]
async fn outbound_shipment_lifecycle() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_sr(&client).await {
        eprintln!("SR service not reachable at {} -- skipping", sr_url());
        return;
    }
    println!("SR service healthy");

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id);
    let base = sr_url();

    // JWT gate
    let probe = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{}",
            Uuid::new_v4()
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("SR returns 401 with JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }

    // SR DB pool
    let sr_pool = match get_sr_pool().await {
        Some(p) => p,
        None => {
            eprintln!("SR DB not reachable -- skipping DB verification");
            return;
        }
    };

    // Step 1: Create outbound shipment
    println!("\n-- Step 1: Create outbound shipment --");
    let resp = client
        .post(format!("{base}/api/shipping-receiving/shipments"))
        .bearer_auth(&jwt)
        .json(&json!({
            "direction": "outbound",
            "tracking_number": format!("E2E-OUTBOUND-{}", &tenant_id[..8]),
            "currency": "USD"
        }))
        .send()
        .await
        .expect("create outbound");
    let s = resp.status();
    let body: Value = resp.json().await.expect("create outbound body");
    assert_eq!(s, StatusCode::CREATED, "create outbound: {}", body);
    let shipment_id = extract_uuid(&body, "id");
    assert_eq!(body["direction"], "outbound");
    println!("  created {} (draft)", shipment_id);

    // Step 2: confirmed → picking
    println!("\n-- Steps 2-3: confirmed → picking --");
    transition(&client, &base, &jwt, shipment_id, "confirmed").await;
    transition(&client, &base, &jwt, shipment_id, "picking").await;

    // Step 4: Add outbound line
    println!("\n-- Step 4: Add outbound line --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "sku": "E2E-OUT-SKU-001",
            "uom": "EA",
            "qty_expected": 30
        }))
        .send()
        .await
        .expect("add outbound line");
    let s = resp.status();
    let body: Value = resp.json().await.expect("outbound line body");
    assert_eq!(s, StatusCode::CREATED, "add outbound line: {}", body);
    let line_id = extract_uuid(&body, "id");
    println!("  line {line_id}");

    // Step 5: Set ship qty
    println!("\n-- Step 5: Set ship qty --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/lines/{line_id}/ship-qty"
        ))
        .bearer_auth(&jwt)
        .json(&json!({ "qty_shipped": 30 }))
        .send()
        .await
        .expect("ship-qty");
    let s = resp.status();
    let body: Value = resp.json().await.expect("ship-qty body");
    assert_eq!(s, StatusCode::OK, "ship-qty: {}", body);
    assert_eq!(body["qty_shipped"], 30);
    println!("  qty_shipped=30");

    // Step 6: packed
    println!("\n-- Step 6: packed --");
    transition(&client, &base, &jwt, shipment_id, "packed").await;

    // Step 7: Ship — triggers inventory issue integration
    println!("\n-- Step 7: POST /ship (triggers inventory issue) --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/ship"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("ship");
    let s = resp.status();
    let body: Value = resp.json().await.expect("ship body");
    assert_eq!(s, StatusCode::OK, "ship: {}", body);
    assert_eq!(body["status"], "shipped");
    println!("  shipped ok");

    // Step 8: Verify inventory_ref_id IS NOT NULL (SR DB)
    println!("\n-- Step 8: Verify inventory_ref_id IS NOT NULL (SR DB) --");
    let rows: Vec<(Uuid, Option<Uuid>)> =
        sqlx::query_as("SELECT id, inventory_ref_id FROM shipment_lines WHERE shipment_id = $1")
            .bind(shipment_id)
            .fetch_all(&sr_pool)
            .await
            .expect("query outbound lines");

    assert_eq!(
        rows.len(),
        1,
        "expected 1 outbound line, got {}",
        rows.len()
    );
    let (lid, inv_ref) = &rows[0];
    assert!(
        inv_ref.is_some(),
        "line {lid}: inventory_ref_id IS NULL — inventory issue integration did not fire"
    );
    println!(
        "  line {lid} -> inventory_ref_id={} -- issue confirmed",
        inv_ref.unwrap()
    );

    // Step 9: Verify outbox event
    println!("\n-- Step 9: Verify outbox event --");
    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sr_events_outbox WHERE aggregate_id = $1::text AND event_type LIKE '%shipped%'",
    )
    .bind(shipment_id)
    .fetch_one(&sr_pool)
    .await
    .expect("outbox query");
    assert!(
        event_count > 0,
        "no 'shipped' outbox event for shipment {shipment_id}"
    );
    println!("  outbox: {event_count} shipped event(s) found");

    // Step 10: deliver → close
    println!("\n-- Step 10: deliver → close --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/deliver"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("deliver");
    let s = resp.status();
    let body: Value = resp.json().await.expect("deliver body");
    assert_eq!(s, StatusCode::OK, "deliver: {}", body);
    assert_eq!(body["status"], "delivered");
    println!("  delivered ok");

    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}/close"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("close outbound");
    let s = resp.status();
    let body: Value = resp.json().await.expect("close outbound body");
    assert_eq!(s, StatusCode::OK, "close outbound: {}", body);
    assert_eq!(body["status"], "closed");
    println!("  closed ok");

    // Step 11: Final GET
    println!("\n-- Step 11: Final GET --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{shipment_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("final get");
    let body: Value = resp.json().await.expect("final get body");
    assert_eq!(body["status"], "closed");
    assert_eq!(body["direction"], "outbound");
    println!("  final GET -> closed, outbound");

    // Cleanup
    cleanup_sr(&sr_pool, &tenant_id).await;
    println!("\n✓ outbound_shipment_lifecycle passed");
}
