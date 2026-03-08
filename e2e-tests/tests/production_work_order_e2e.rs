//! E2E: Production Work Order lifecycle
//!
//! Tests the state machine invariants via real HTTP calls:
//!   Draft -> Released -> (Operations) -> Closed
//!   Invalid transitions return 422.
//!
//! No mocks. Requires live Docker stack (port 8108).

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const BASE_URL: &str = "http://localhost:8108";

// ── JWT helpers ──────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct TestClaims {
    sub: String,
    tenant_id: String,
    exp: usize,
    iat: usize,
}

fn make_jwt(tenant_id: &str) -> Option<String> {
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM").ok()?;
    let now = Utc::now().timestamp() as usize;
    let claims = TestClaims {
        sub: "test-user".to_string(),
        tenant_id: tenant_id.to_string(),
        exp: now + 3600,
        iat: now,
    };
    let key = EncodingKey::from_rsa_pem(pem.as_bytes()).ok()?;
    encode(&Header::new(Algorithm::RS256), &claims, &key).ok()
}

// ── Utility helpers ──────────────────────────────────────────────────────────

/// Wait for the service health endpoint to respond OK.
async fn wait_for_health(client: &Client) -> bool {
    for _ in 0..15 {
        if let Ok(r) = client
            .get(format!("{BASE_URL}/api/health"))
            .send()
            .await
        {
            if r.status().is_success() {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    false
}

/// Extract `id` (UUID string) from a JSON response body.
fn extract_id(body: &Value) -> Uuid {
    let raw = body
        .get("id")
        .or_else(|| body.get("work_order_id"))
        .and_then(|v| v.as_str())
        .expect("response body should contain 'id'");
    Uuid::parse_str(raw).expect("id should be a valid UUID")
}

/// Assert that a request without Authorization gets 401.
async fn assert_unauth(client: &Client, method: &str, path: &str) {
    let url = format!("{BASE_URL}{path}");
    let resp = match method {
        "GET" => client.get(&url).send().await,
        "POST" => client.post(&url).json(&json!({})).send().await,
        _ => panic!("unsupported method {method}"),
    }
    .expect("request should not fail at transport level");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "{method} {path} should require auth"
    );
}

// ── Routing/Workcenter setup helpers ─────────────────────────────────────────

/// Create a workcenter and return its UUID.
async fn create_workcenter(client: &Client, auth: &str, tenant_id: &str, name: &str) -> Uuid {
    let body = json!({
        "tenant_id": tenant_id,
        "name": name,
        "workcenter_type": "machine",
        "capacity": 1,
        "cost_per_hour": "100.00"
    });
    let resp = client
        .post(format!("{BASE_URL}/api/production/workcenters"))
        .header("Authorization", auth)
        .json(&body)
        .send()
        .await
        .expect("create workcenter request");
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "create workcenter should return 201"
    );
    let val: Value = resp.json().await.expect("workcenter JSON");
    extract_id(&val)
}

/// Create a routing template (draft) and return its UUID.
async fn create_routing(client: &Client, auth: &str, tenant_id: &str) -> Uuid {
    let body = json!({
        "tenant_id": tenant_id,
        "name": format!("Routing-{}", Uuid::new_v4()),
        "description": "Two-step E2E routing"
    });
    let resp = client
        .post(format!("{BASE_URL}/api/production/routings"))
        .header("Authorization", auth)
        .json(&body)
        .send()
        .await
        .expect("create routing request");
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "create routing should return 201"
    );
    let val: Value = resp.json().await.expect("routing JSON");
    extract_id(&val)
}

/// Add a routing step and return its UUID.
async fn add_routing_step(
    client: &Client,
    auth: &str,
    tenant_id: &str,
    routing_id: Uuid,
    workcenter_id: Uuid,
    sequence: u32,
) -> Uuid {
    let body = json!({
        "tenant_id": tenant_id,
        "workcenter_id": workcenter_id,
        "sequence_number": sequence,
        "operation_name": format!("Step-{sequence}"),
        "standard_time_minutes": 30,
        "is_required": true
    });
    let resp = client
        .post(format!("{BASE_URL}/api/production/routings/{routing_id}/steps"))
        .header("Authorization", auth)
        .json(&body)
        .send()
        .await
        .expect("add routing step request");
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "add routing step should return 201"
    );
    let val: Value = resp.json().await.expect("step JSON");
    extract_id(&val)
}

/// Release a routing template (makes it usable on work orders).
async fn release_routing(client: &Client, auth: &str, routing_id: Uuid) {
    let resp = client
        .post(format!("{BASE_URL}/api/production/routings/{routing_id}/release"))
        .header("Authorization", auth)
        .json(&json!({}))
        .send()
        .await
        .expect("release routing request");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "release routing should return 200"
    );
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Full lifecycle: Draft -> Release -> Operations -> Close.
/// Also verifies invalid transitions are rejected with 422.
#[tokio::test]
async fn work_order_full_lifecycle_state_machine() {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP client");

    if !wait_for_health(&client).await {
        eprintln!("Production service not reachable at {BASE_URL} -- skipping");
        return;
    }

    let tenant_id = Uuid::new_v4().to_string();
    let token = match make_jwt(&tenant_id) {
        Some(t) => t,
        None => {
            eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
            return;
        }
    };
    let auth = format!("Bearer {token}");

    // ── Auth guard ──
    assert_unauth(&client, "POST", "/api/production/work-orders").await;
    assert_unauth(&client, "GET", "/api/production/work-orders").await;

    // ── Build a 2-step routing ──
    let wc1 = create_workcenter(&client, &auth, &tenant_id, "Mill-A").await;
    let wc2 = create_workcenter(&client, &auth, &tenant_id, "Mill-B").await;
    let routing_id = create_routing(&client, &auth, &tenant_id).await;
    add_routing_step(&client, &auth, &tenant_id, routing_id, wc1, 10).await;
    add_routing_step(&client, &auth, &tenant_id, routing_id, wc2, 20).await;
    release_routing(&client, &auth, routing_id).await;

    // ── Step 1: Create work order ──
    let item_id = Uuid::new_v4();
    let bom_rev_id = Uuid::new_v4();
    let order_number = format!("WO-E2E-{}", &Uuid::new_v4().to_string()[..8]);

    let create_body = json!({
        "tenant_id": tenant_id,
        "order_number": order_number,
        "item_id": item_id,
        "bom_revision_id": bom_rev_id,
        "routing_id": routing_id,
        "planned_quantity": 100,
        "planned_start_date": "2026-04-01",
        "planned_end_date": "2026-04-15"
    });

    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders"))
        .header("Authorization", &auth)
        .json(&create_body)
        .send()
        .await
        .expect("create work order");
    assert_eq!(resp.status(), StatusCode::CREATED, "create WO -> 201");
    let wo: Value = resp.json().await.expect("WO JSON");
    let wo_id = extract_id(&wo);
    assert_eq!(wo["status"].as_str().unwrap_or(""), "draft", "new WO status = draft");

    // ── Step 2: GET verifies draft ──
    let resp = client
        .get(format!("{BASE_URL}/api/production/work-orders/{wo_id}"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("GET work order");
    assert_eq!(resp.status(), StatusCode::OK, "GET WO -> 200");
    let fetched: Value = resp.json().await.expect("fetched WO JSON");
    assert_eq!(fetched["status"].as_str().unwrap_or(""), "draft");

    // ── Step 3: Close draft -> 422 (invalid transition) ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/close"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("close draft WO");
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "close draft WO -> 422"
    );

    // ── Step 4: Release ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/release"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("release WO");
    assert_eq!(resp.status(), StatusCode::OK, "release WO -> 200");
    let released: Value = resp.json().await.expect("released WO JSON");
    assert_eq!(released["status"].as_str().unwrap_or(""), "released");

    // ── Step 5: Re-release -> 422 ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/release"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("re-release WO");
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "re-release WO -> 422"
    );

    // ── Step 6: Initialize operations ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/operations/initialize"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("initialize operations");
    assert_eq!(resp.status(), StatusCode::OK, "initialize operations -> 200");

    // ── Step 7: List operations -> 2 pending ──
    let resp = client
        .get(format!("{BASE_URL}/api/production/work-orders/{wo_id}/operations"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("list operations");
    assert_eq!(resp.status(), StatusCode::OK, "list ops -> 200");
    let ops: Value = resp.json().await.expect("ops JSON");
    let ops_arr = ops.as_array().expect("ops should be an array");
    assert_eq!(ops_arr.len(), 2, "should have 2 operations after init");
    for op in ops_arr {
        assert_eq!(
            op["status"].as_str().unwrap_or(""),
            "pending",
            "all ops should start pending"
        );
    }

    // Sort ops by sequence to get op1 (seq=10) and op2 (seq=20)
    let mut sorted_ops = ops_arr.clone();
    sorted_ops.sort_by_key(|o| o["sequence_number"].as_u64().unwrap_or(u64::MAX));
    let op1_id = extract_id(&sorted_ops[0]);
    let op2_id = extract_id(&sorted_ops[1]);

    // ── Step 8: Start op2 before op1 complete -> 422 (sequencing guard) ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/operations/{op2_id}/start"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("start op2 before op1");
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "starting op2 before op1 complete -> 422"
    );

    // ── Step 9: Start op1 ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/operations/{op1_id}/start"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("start op1");
    assert_eq!(resp.status(), StatusCode::OK, "start op1 -> 200");
    let op1_started: Value = resp.json().await.expect("op1 started JSON");
    assert_eq!(op1_started["status"].as_str().unwrap_or(""), "in_progress");

    // ── Step 10: Complete op1 ──
    let resp = client
        .post(format!(
            "{BASE_URL}/api/production/work-orders/{wo_id}/operations/{op1_id}/complete"
        ))
        .header("Authorization", &auth)
        .json(&json!({ "actual_quantity": 100 }))
        .send()
        .await
        .expect("complete op1");
    assert_eq!(resp.status(), StatusCode::OK, "complete op1 -> 200");
    let op1_done: Value = resp.json().await.expect("op1 completed JSON");
    assert_eq!(op1_done["status"].as_str().unwrap_or(""), "completed");

    // ── Step 11: Start op2 ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/operations/{op2_id}/start"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("start op2");
    assert_eq!(resp.status(), StatusCode::OK, "start op2 -> 200");
    let op2_started: Value = resp.json().await.expect("op2 started JSON");
    assert_eq!(op2_started["status"].as_str().unwrap_or(""), "in_progress");

    // ── Step 12: Complete op2 ──
    let resp = client
        .post(format!(
            "{BASE_URL}/api/production/work-orders/{wo_id}/operations/{op2_id}/complete"
        ))
        .header("Authorization", &auth)
        .json(&json!({ "actual_quantity": 100 }))
        .send()
        .await
        .expect("complete op2");
    assert_eq!(resp.status(), StatusCode::OK, "complete op2 -> 200");
    let op2_done: Value = resp.json().await.expect("op2 completed JSON");
    assert_eq!(op2_done["status"].as_str().unwrap_or(""), "completed");

    // ── Step 13: GET ops -> all completed ──
    let resp = client
        .get(format!("{BASE_URL}/api/production/work-orders/{wo_id}/operations"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("list ops after completion");
    assert_eq!(resp.status(), StatusCode::OK);
    let final_ops: Value = resp.json().await.expect("final ops JSON");
    for op in final_ops.as_array().expect("ops array") {
        assert_eq!(
            op["status"].as_str().unwrap_or(""),
            "completed",
            "all ops should be completed"
        );
    }

    // ── Step 14: Finished-goods receipt ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/fg-receipt"))
        .header("Authorization", &auth)
        .json(&json!({ "quantity_received": 100, "location_id": Uuid::new_v4() }))
        .send()
        .await
        .expect("finished-goods receipt");
    assert!(
        resp.status() == StatusCode::OK
            || resp.status() == StatusCode::ACCEPTED
            || resp.status() == StatusCode::CREATED,
        "FG receipt should succeed, got {}",
        resp.status()
    );

    // ── Step 15: Close WO ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/close"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("close WO");
    assert_eq!(resp.status(), StatusCode::OK, "close WO -> 200");
    let closed: Value = resp.json().await.expect("closed WO JSON");
    assert_eq!(closed["status"].as_str().unwrap_or(""), "closed");

    // ── Step 16: GET -> closed ──
    let resp = client
        .get(format!("{BASE_URL}/api/production/work-orders/{wo_id}"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("GET closed WO");
    assert_eq!(resp.status(), StatusCode::OK);
    let final_wo: Value = resp.json().await.expect("final WO JSON");
    assert_eq!(final_wo["status"].as_str().unwrap_or(""), "closed");

    // ── Step 17: Release closed -> 422 ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders/{wo_id}/release"))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("release closed WO");
    assert_eq!(
        resp.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "release closed WO -> 422"
    );
}

/// CRUD field round-trip: verifies all fields survive create/read,
/// 404 for unknown IDs, and 409 for duplicate order numbers.
#[tokio::test]
async fn work_order_crud_fields_round_trip() {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP client");

    if !wait_for_health(&client).await {
        eprintln!("Production service not reachable at {BASE_URL} -- skipping");
        return;
    }

    let tenant_id = Uuid::new_v4().to_string();
    let token = match make_jwt(&tenant_id) {
        Some(t) => t,
        None => {
            eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
            return;
        }
    };
    let auth = format!("Bearer {token}");

    let item_id = Uuid::new_v4();
    let bom_rev_id = Uuid::new_v4();
    let routing_id = Uuid::new_v4();
    let order_number = format!("WO-CRUD-{}", &Uuid::new_v4().to_string()[..8]);

    let create_body = json!({
        "tenant_id": tenant_id,
        "order_number": order_number,
        "item_id": item_id,
        "bom_revision_id": bom_rev_id,
        "routing_id": routing_id,
        "planned_quantity": 50,
        "planned_start_date": "2026-05-01",
        "planned_end_date": "2026-05-10"
    });

    // ── Create ──
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders"))
        .header("Authorization", &auth)
        .json(&create_body)
        .send()
        .await
        .expect("create WO for CRUD test");
    assert_eq!(resp.status(), StatusCode::CREATED, "create WO -> 201");
    let created: Value = resp.json().await.expect("created WO JSON");
    let wo_id = extract_id(&created);

    // Verify returned fields
    assert_eq!(
        created["order_number"].as_str().unwrap_or(""),
        order_number
    );
    assert_eq!(
        created["item_id"].as_str().unwrap_or(""),
        item_id.to_string()
    );
    assert_eq!(
        created["bom_revision_id"].as_str().unwrap_or(""),
        bom_rev_id.to_string()
    );
    assert_eq!(created["planned_quantity"].as_u64().unwrap_or(0), 50);
    assert_eq!(created["status"].as_str().unwrap_or(""), "draft");

    // ── GET round-trip ──
    let resp = client
        .get(format!("{BASE_URL}/api/production/work-orders/{wo_id}"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("GET WO");
    assert_eq!(resp.status(), StatusCode::OK, "GET WO -> 200");
    let fetched: Value = resp.json().await.expect("fetched WO JSON");
    assert_eq!(
        fetched["order_number"].as_str().unwrap_or(""),
        order_number
    );
    assert_eq!(
        fetched["item_id"].as_str().unwrap_or(""),
        item_id.to_string()
    );
    assert_eq!(
        fetched["bom_revision_id"].as_str().unwrap_or(""),
        bom_rev_id.to_string()
    );
    assert_eq!(fetched["planned_quantity"].as_u64().unwrap_or(0), 50);
    assert_eq!(fetched["status"].as_str().unwrap_or(""), "draft");

    // ── GET unknown ID -> 404 ──
    let unknown = Uuid::new_v4();
    let resp = client
        .get(format!("{BASE_URL}/api/production/work-orders/{unknown}"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("GET unknown WO");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "GET unknown WO -> 404");

    // ── Duplicate order_number -> 409 ──
    let dup_body = json!({
        "tenant_id": tenant_id,
        "order_number": order_number,
        "item_id": item_id,
        "bom_revision_id": bom_rev_id,
        "routing_id": routing_id,
        "planned_quantity": 5,
        "planned_start_date": "2026-06-01",
        "planned_end_date": "2026-06-05"
    });
    let resp = client
        .post(format!("{BASE_URL}/api/production/work-orders"))
        .header("Authorization", &auth)
        .json(&dup_body)
        .send()
        .await
        .expect("duplicate WO");
    assert_eq!(
        resp.status(),
        StatusCode::CONFLICT,
        "duplicate order_number -> 409"
    );
}
