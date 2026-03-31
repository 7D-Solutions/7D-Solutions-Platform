// HTTP smoke tests: Shipping-Receiving
//
// Proves all 15 Shipping-Receiving routes respond correctly via reqwest against
// the live shipping-receiving service (port 8103).
//
// Inbound lifecycle:
//   create → add_line → confirmed → in_transit → arrived → receiving
//   → receive_line → accept_line → route_line (direct_to_stock)
//   → GET routings → GET shipment → close
//
// Outbound lifecycle:
//   create → confirmed → picking → add_line → ship_qty → packed
//   → POST /ship → POST /deliver → POST /close
//
// Query routes (using seeded inbound line data):
//   GET /po/{po_id}/shipments
//   GET /po-lines/{po_line_id}/lines
//   GET /source/{ref_type}/{ref_id}/shipments
//
// No mocks, no stubs — all calls against the live service.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const SR_DEFAULT_URL: &str = "http://localhost:8103";

fn sr_url() -> String {
    std::env::var("SHIPPING_RECEIVING_URL").unwrap_or_else(|_| SR_DEFAULT_URL.to_string())
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

/// All SR routes require auth — 401 without JWT.
async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PATCH" => client.patch(url),
        _ => panic!("unsupported method: {method}"),
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

fn extract_uuid(body: &Value, key: &str) -> Uuid {
    Uuid::parse_str(
        body.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("missing '{}' in: {}", key, body)),
    )
    .unwrap_or_else(|_| panic!("invalid UUID for '{}' in: {}", key, body))
}

#[tokio::test]
async fn smoke_shipping_receiving() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();

    if !wait_for_sr(&client).await {
        eprintln!("SR service not reachable at {} -- skipping", sr_url());
        return;
    }
    println!("SR service healthy at {}", sr_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id, &["shipping_receiving.mutate"]);
    let base = sr_url();

    // Gate: verify JWT is accepted (probe a GET route that requires auth)
    let probe = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{}",
            Uuid::new_v4()
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("SR returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }
    println!("  JWT probe ok ({})", probe.status());

    // ========================================================================
    // Auth gate: all 15 routes require JWT — 401 without it
    // ========================================================================
    println!("\n-- Auth gate (15 routes) --");
    let rid = Uuid::new_v4();
    let lid = Uuid::new_v4();

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments"),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/shipping-receiving/shipments/{rid}"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "PATCH",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/status"),
        Some(json!({"status": "confirmed"})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/lines"),
        Some(json!({"qty_expected": 1})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/lines/{lid}/receive"),
        Some(json!({"qty_received": 1, "qty_accepted": 1, "qty_rejected": 0})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/lines/{lid}/accept"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/lines/{lid}/ship-qty"),
        Some(json!({"qty_shipped": 1})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/lines/{lid}/route"),
        Some(json!({"route_decision": "direct_to_stock"})),
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/routings"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/ship"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/deliver"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/shipping-receiving/shipments/{rid}/close"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/shipping-receiving/po/{rid}/shipments"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/shipping-receiving/po-lines/{rid}/lines"),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/shipping-receiving/source/purchase_order/{rid}/shipments"),
        None,
    )
    .await;

    // Reference IDs for seeding lines (used later by query routes)
    let po_id = Uuid::new_v4();
    let po_line_id = Uuid::new_v4();
    let source_ref_id = Uuid::new_v4();
    let source_ref_type = "purchase_order";

    // ========================================================================
    // INBOUND LIFECYCLE
    // ========================================================================

    // Step 1: Create inbound shipment
    println!("\n-- Step 1: POST /api/shipping-receiving/shipments (inbound) --");
    let resp = client
        .post(format!("{base}/api/shipping-receiving/shipments"))
        .bearer_auth(&jwt)
        .json(&json!({
            "direction": "inbound",
            "tracking_number": "SMOKE-INBOUND-001",
            "currency": "USD"
        }))
        .send()
        .await
        .expect("create inbound shipment failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("create inbound body");
    assert_eq!(status, StatusCode::CREATED, "create inbound: {}", body);
    let inbound_id = extract_uuid(&body, "id");
    assert_eq!(body["status"], "draft");
    assert_eq!(body["direction"], "inbound");
    println!("  created inbound {} -> 201 ok (status=draft)", inbound_id);

    // Step 2: Add inbound line (seeds po_id, po_line_id, source_ref for query routes)
    println!("\n-- Step 2: POST /{inbound_id}/lines --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/lines"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "sku": "SMOKE-SKU-001",
            "uom": "EA",
            "qty_expected": 100,
            "po_id": po_id,
            "po_line_id": po_line_id,
            "source_ref_type": source_ref_type,
            "source_ref_id": source_ref_id
        }))
        .send()
        .await
        .expect("add inbound line failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("add inbound line body");
    assert_eq!(status, StatusCode::CREATED, "add inbound line: {}", body);
    let inbound_line_id = extract_uuid(&body, "id");
    assert_eq!(body["qty_expected"], 100);
    println!("  added inbound line {} -> 201 ok", inbound_line_id);

    // Step 3: Transition inbound → confirmed
    println!("\n-- Step 3: PATCH /{inbound_id}/status → confirmed --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "confirmed"}))
        .send()
        .await
        .expect("confirm inbound failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("confirm body");
    assert_eq!(status, StatusCode::OK, "confirm inbound: {}", body);
    assert_eq!(body["status"], "confirmed");
    println!("  -> confirmed ok");

    // Step 4: Transition inbound → in_transit
    println!("\n-- Step 4: PATCH /{inbound_id}/status → in_transit --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "in_transit"}))
        .send()
        .await
        .expect("in_transit failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("in_transit body");
    assert_eq!(status, StatusCode::OK, "in_transit: {}", body);
    assert_eq!(body["status"], "in_transit");
    println!("  -> in_transit ok");

    // Step 5: Transition inbound → arrived
    println!("\n-- Step 5: PATCH /{inbound_id}/status → arrived --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "arrived", "arrived_at": chrono::Utc::now().to_rfc3339()}))
        .send()
        .await
        .expect("arrived failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("arrived body");
    assert_eq!(status, StatusCode::OK, "arrived: {}", body);
    assert_eq!(body["status"], "arrived");
    println!("  -> arrived ok");

    // Step 6: Transition inbound → receiving
    println!("\n-- Step 6: PATCH /{inbound_id}/status → receiving --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "receiving"}))
        .send()
        .await
        .expect("receiving failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("receiving body");
    assert_eq!(status, StatusCode::OK, "receiving: {}", body);
    assert_eq!(body["status"], "receiving");
    println!("  -> receiving ok");

    // Step 7: Receive the line
    println!("\n-- Step 7: POST /{inbound_id}/lines/{inbound_line_id}/receive --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/lines/{inbound_line_id}/receive"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "qty_received": 100,
            "qty_accepted": 95,
            "qty_rejected": 5
        }))
        .send()
        .await
        .expect("receive line failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("receive line body");
    assert_eq!(status, StatusCode::OK, "receive line: {}", body);
    assert_eq!(body["qty_received"], 100);
    println!("  receive line -> 200 ok (qty_received=100)");

    // Step 8: Accept the line (qty_accepted = qty_received, qty_rejected = 0)
    println!("\n-- Step 8: POST /{inbound_id}/lines/{inbound_line_id}/accept --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/lines/{inbound_line_id}/accept"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("accept line failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("accept line body");
    assert_eq!(status, StatusCode::OK, "accept line: {}", body);
    assert_eq!(body["qty_accepted"], 100);
    assert_eq!(body["qty_rejected"], 0);
    println!("  accept line -> 200 ok (qty_accepted=100, qty_rejected=0)");

    // Step 9: Route the line (requires inbound + "receiving" status)
    println!("\n-- Step 9: POST /{inbound_id}/lines/{inbound_line_id}/route --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/lines/{inbound_line_id}/route"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "route_decision": "direct_to_stock",
            "reason": "Smoke test: all quantities accepted"
        }))
        .send()
        .await
        .expect("route line failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("route line body");
    assert_eq!(status, StatusCode::CREATED, "route line: {}", body);
    assert_eq!(body["route_decision"], "direct_to_stock");
    println!("  route line -> 201 ok (direct_to_stock)");

    // Step 10: List routings for the inbound shipment
    println!("\n-- Step 10: GET /{inbound_id}/routings --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/routings"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("list routings failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("list routings body");
    assert_eq!(status, StatusCode::OK, "list routings: {}", body);
    let routings = body.as_array().expect("routings should be array");
    assert!(!routings.is_empty(), "routings should contain seeded routing");
    println!("  list routings -> 200 ok ({} routing(s))", routings.len());

    // Step 11: GET inbound shipment
    println!("\n-- Step 11: GET /api/shipping-receiving/shipments/{inbound_id} --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get inbound shipment failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("get inbound body");
    assert_eq!(status, StatusCode::OK, "get inbound: {}", body);
    assert_eq!(body["direction"], "inbound");
    assert_eq!(body["status"], "receiving");
    println!("  get inbound shipment -> 200 ok (status=receiving)");

    // Step 12: Close inbound shipment
    println!("\n-- Step 12: POST /{inbound_id}/close --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{inbound_id}/close"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("close inbound failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("close inbound body");
    assert_eq!(status, StatusCode::OK, "close inbound: {}", body);
    assert_eq!(body["status"], "closed");
    println!("  close inbound -> 200 ok (status=closed)");

    // ========================================================================
    // OUTBOUND LIFECYCLE
    // ========================================================================

    // Step 13: Create outbound shipment
    println!("\n-- Step 13: POST /api/shipping-receiving/shipments (outbound) --");
    let resp = client
        .post(format!("{base}/api/shipping-receiving/shipments"))
        .bearer_auth(&jwt)
        .json(&json!({
            "direction": "outbound",
            "tracking_number": "SMOKE-OUTBOUND-001",
            "currency": "USD"
        }))
        .send()
        .await
        .expect("create outbound shipment failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("create outbound body");
    assert_eq!(status, StatusCode::CREATED, "create outbound: {}", body);
    let outbound_id = extract_uuid(&body, "id");
    assert_eq!(body["status"], "draft");
    assert_eq!(body["direction"], "outbound");
    println!("  created outbound {} -> 201 ok (status=draft)", outbound_id);

    // Step 14: Transition outbound → confirmed
    println!("\n-- Step 14: PATCH /{outbound_id}/status → confirmed --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "confirmed"}))
        .send()
        .await
        .expect("confirm outbound failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("confirm outbound body");
    assert_eq!(status, StatusCode::OK, "confirm outbound: {}", body);
    assert_eq!(body["status"], "confirmed");
    println!("  -> confirmed ok");

    // Step 15: Transition outbound → picking
    println!("\n-- Step 15: PATCH /{outbound_id}/status → picking --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "picking"}))
        .send()
        .await
        .expect("picking failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("picking body");
    assert_eq!(status, StatusCode::OK, "picking: {}", body);
    assert_eq!(body["status"], "picking");
    println!("  -> picking ok");

    // Step 16: Add outbound line
    println!("\n-- Step 16: POST /{outbound_id}/lines --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/lines"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "sku": "SMOKE-SKU-002",
            "uom": "EA",
            "qty_expected": 50
        }))
        .send()
        .await
        .expect("add outbound line failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("add outbound line body");
    assert_eq!(status, StatusCode::CREATED, "add outbound line: {}", body);
    let outbound_line_id = extract_uuid(&body, "id");
    println!("  added outbound line {} -> 201 ok", outbound_line_id);

    // Step 17: Set ship qty on the outbound line
    println!("\n-- Step 17: POST /{outbound_id}/lines/{outbound_line_id}/ship-qty --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/lines/{outbound_line_id}/ship-qty"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"qty_shipped": 50}))
        .send()
        .await
        .expect("ship-qty failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("ship-qty body");
    assert_eq!(status, StatusCode::OK, "ship-qty: {}", body);
    assert_eq!(body["qty_shipped"], 50);
    println!("  ship-qty -> 200 ok (qty_shipped=50)");

    // Step 18: Transition outbound → packed
    println!("\n-- Step 18: PATCH /{outbound_id}/status → packed --");
    let resp = client
        .patch(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/status"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"status": "packed"}))
        .send()
        .await
        .expect("packed failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("packed body");
    assert_eq!(status, StatusCode::OK, "packed: {}", body);
    assert_eq!(body["status"], "packed");
    println!("  -> packed ok");

    // Step 19: Ship (packed → shipped via convenience handler)
    println!("\n-- Step 19: POST /{outbound_id}/ship --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/ship"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("ship outbound failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("ship body");
    assert_eq!(status, StatusCode::OK, "ship: {}", body);
    assert_eq!(body["status"], "shipped");
    println!("  POST /ship -> 200 ok (status=shipped)");

    // Step 20: Deliver (shipped → delivered via convenience handler)
    println!("\n-- Step 20: POST /{outbound_id}/deliver --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/deliver"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("deliver outbound failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("deliver body");
    assert_eq!(status, StatusCode::OK, "deliver: {}", body);
    assert_eq!(body["status"], "delivered");
    println!("  POST /deliver -> 200 ok (status=delivered)");

    // Step 21: Close outbound (delivered → closed)
    println!("\n-- Step 21: POST /{outbound_id}/close --");
    let resp = client
        .post(format!(
            "{base}/api/shipping-receiving/shipments/{outbound_id}/close"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("close outbound failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("close outbound body");
    assert_eq!(status, StatusCode::OK, "close outbound: {}", body);
    assert_eq!(body["status"], "closed");
    println!("  POST /close -> 200 ok (status=closed)");

    // ========================================================================
    // QUERY ROUTES (using seeded data from inbound line)
    // ========================================================================

    // Step 22: GET /po/{po_id}/shipments — returns inbound shipment
    println!("\n-- Step 22: GET /api/shipping-receiving/po/{po_id}/shipments --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/po/{po_id}/shipments"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("shipments_by_po failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("shipments_by_po body");
    assert_eq!(status, StatusCode::OK, "shipments_by_po: {}", body);
    let rows = body.as_array().expect("shipments_by_po should be array");
    assert!(!rows.is_empty(), "shipments_by_po should return seeded shipment");
    println!("  shipments_by_po -> 200 ok ({} row(s))", rows.len());

    // Step 23: GET /po-lines/{po_line_id}/lines — returns inbound line
    println!("\n-- Step 23: GET /api/shipping-receiving/po-lines/{po_line_id}/lines --");
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/po-lines/{po_line_id}/lines"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("lines_by_po_line failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("lines_by_po_line body");
    assert_eq!(status, StatusCode::OK, "lines_by_po_line: {}", body);
    let rows = body.as_array().expect("lines_by_po_line should be array");
    assert!(!rows.is_empty(), "lines_by_po_line should return seeded line");
    println!("  lines_by_po_line -> 200 ok ({} row(s))", rows.len());

    // Step 24: GET /source/{ref_type}/{ref_id}/shipments — returns inbound shipment
    println!(
        "\n-- Step 24: GET /api/shipping-receiving/source/{source_ref_type}/{source_ref_id}/shipments --"
    );
    let resp = client
        .get(format!(
            "{base}/api/shipping-receiving/source/{source_ref_type}/{source_ref_id}/shipments"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("shipments_by_source_ref failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("shipments_by_source_ref body");
    assert_eq!(
        status,
        StatusCode::OK,
        "shipments_by_source_ref: {}",
        body
    );
    let rows = body
        .as_array()
        .expect("shipments_by_source_ref should be array");
    assert!(
        !rows.is_empty(),
        "shipments_by_source_ref should return seeded shipment"
    );
    println!(
        "  shipments_by_source_ref -> 200 ok ({} row(s))",
        rows.len()
    );

    println!("\n=== smoke_shipping_receiving PASSED (15 routes) ===");
}
