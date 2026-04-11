// E2E: Production composite endpoints
//
// Covers four new features from bd-i6if4, bd-8v63o, bd-k5bla, bd-dhl7p:
//
// 1. derived_status: WO GET response includes computed derived_status field
//    (not_started / in_progress / complete) based on operations state.
//
// 2. routing_step_enrichment: GET /api/production/routings/{id}/steps with
//    ?include=workcenter_details embeds a `workcenter` object; without the
//    param the response is identical to before (backward compat).
//
// 3. batch_work_orders: GET /api/production/work-orders?ids=a,b,c returns all
//    requested WOs in a single call. Validates 400 on empty/oversized id list.
//
// 4. composite_create: POST /api/production/work-orders/create allocates a WO
//    number from the Numbering service and creates the WO in one call.
//    Skipped gracefully if the Numbering service is not reachable.
//
// Requires: live Production service (8108).
// No mocks. Real Postgres.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const PROD_DEFAULT_URL: &str = "http://localhost:8108";
const NUMBERING_DEFAULT_URL: &str = "http://localhost:8120";

fn prod_url() -> String {
    std::env::var("PRODUCTION_URL").unwrap_or_else(|_| PROD_DEFAULT_URL.to_string())
}

fn numbering_url() -> String {
    std::env::var("NUMBERING_URL").unwrap_or_else(|_| NUMBERING_DEFAULT_URL.to_string())
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
        perms: vec![
            "production.mutate".to_string(),
            "production.read".to_string(),
        ],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

// ── Service health ────────────────────────────────────────────────────────────

async fn wait_for_production(client: &Client) -> bool {
    let url = format!("{}/api/health", prod_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  Production health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  Production health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn numbering_reachable(client: &Client) -> bool {
    let url = format!("{}/api/health", numbering_url());
    match tokio::time::timeout(
        Duration::from_secs(3),
        client.get(&url).send(),
    )
    .await
    {
        Ok(Ok(r)) => r.status().is_success(),
        _ => false,
    }
}

// ── Setup helpers ─────────────────────────────────────────────────────────────

/// Create a workcenter and return its UUID.
async fn create_workcenter(client: &Client, auth: &str, tenant_id: &str, name: &str) -> Uuid {
    let code = format!("WC-{}", &Uuid::new_v4().to_string()[..6].to_uppercase());
    let resp = client
        .post(format!("{}/api/production/workcenters", prod_url()))
        .header("Authorization", auth)
        .json(&json!({
            "tenant_id": tenant_id,
            "code": code,
            "name": name,
            "capacity": 1
        }))
        .send()
        .await
        .expect("create workcenter");
    let s = resp.status();
    let body: Value = resp.json().await.expect("workcenter body");
    assert_eq!(s, StatusCode::CREATED, "create workcenter: {}", body);
    Uuid::parse_str(
        body["workcenter_id"]
            .as_str()
            .expect("workcenter_id in response"),
    )
    .unwrap()
}

/// Create a routing template and return its UUID.
async fn create_routing(client: &Client, auth: &str, tenant_id: &str) -> Uuid {
    let name = format!("Routing-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{}/api/production/routings", prod_url()))
        .header("Authorization", auth)
        .json(&json!({
            "tenant_id": tenant_id,
            "name": name,
            "description": "E2E composite test routing"
        }))
        .send()
        .await
        .expect("create routing");
    let s = resp.status();
    let body: Value = resp.json().await.expect("routing body");
    assert_eq!(s, StatusCode::CREATED, "create routing: {}", body);
    Uuid::parse_str(
        body["routing_template_id"]
            .as_str()
            .expect("routing_template_id"),
    )
    .unwrap()
}

/// Add a step to a routing template.
async fn add_step(
    client: &Client,
    auth: &str,
    tenant_id: &str,
    routing_id: Uuid,
    workcenter_id: Uuid,
    seq: i32,
    op_name: &str,
) {
    let resp = client
        .post(format!(
            "{}/api/production/routings/{}/steps",
            prod_url(),
            routing_id
        ))
        .header("Authorization", auth)
        .json(&json!({
            "tenant_id": tenant_id,
            "sequence_number": seq,
            "workcenter_id": workcenter_id,
            "operation_name": op_name,
            "run_time_minutes": 30
        }))
        .send()
        .await
        .expect("add routing step");
    let s = resp.status();
    let body: Value = resp.json().await.expect("step body");
    assert_eq!(s, StatusCode::CREATED, "add routing step: {}", body);
}

/// Release a routing template.
async fn release_routing(client: &Client, auth: &str, routing_id: Uuid) {
    let resp = client
        .post(format!(
            "{}/api/production/routings/{}/release",
            prod_url(),
            routing_id
        ))
        .header("Authorization", auth)
        .json(&json!({}))
        .send()
        .await
        .expect("release routing");
    let s = resp.status();
    let body: Value = resp.json().await.expect("release body");
    assert_eq!(s, StatusCode::OK, "release routing: {}", body);
}

/// Create a work order and return its UUID.
async fn create_work_order(
    client: &Client,
    auth: &str,
    tenant_id: &str,
    routing_id: Option<Uuid>,
) -> Uuid {
    let order_number = format!("WO-E2E-{}", &Uuid::new_v4().to_string()[..8]);
    let mut payload = json!({
        "tenant_id": tenant_id,
        "order_number": order_number,
        "item_id": Uuid::new_v4(),
        "bom_revision_id": Uuid::new_v4(),
        "planned_quantity": 10
    });
    if let Some(rid) = routing_id {
        payload["routing_template_id"] = json!(rid);
    }
    let resp = client
        .post(format!("{}/api/production/work-orders", prod_url()))
        .header("Authorization", auth)
        .json(&payload)
        .send()
        .await
        .expect("create work order");
    let s = resp.status();
    let body: Value = resp.json().await.expect("WO body");
    assert_eq!(s, StatusCode::CREATED, "create WO: {}", body);
    Uuid::parse_str(
        body.get("work_order_id")
            .or_else(|| body.get("id"))
            .and_then(|v| v.as_str())
            .expect("work_order_id in response"),
    )
    .unwrap()
}

/// Release a work order.
async fn release_work_order(client: &Client, auth: &str, wo_id: Uuid) {
    let resp = client
        .post(format!(
            "{}/api/production/work-orders/{}/release",
            prod_url(),
            wo_id
        ))
        .header("Authorization", auth)
        .json(&json!({}))
        .send()
        .await
        .expect("release WO");
    let s = resp.status();
    let body: Value = resp.json().await.expect("release WO body");
    assert_eq!(s, StatusCode::OK, "release WO: {}", body);
}

/// Get a single work order and return its JSON body.
async fn get_work_order(client: &Client, auth: &str, wo_id: Uuid) -> Value {
    let resp = client
        .get(format!(
            "{}/api/production/work-orders/{}",
            prod_url(),
            wo_id
        ))
        .header("Authorization", auth)
        .send()
        .await
        .expect("get WO");
    assert_eq!(resp.status(), StatusCode::OK, "GET WO -> 200");
    resp.json().await.expect("WO JSON")
}

// ── Test 1: derived_status ─────────────────────────────────────────────────────

/// GET /api/production/work-orders/{id} includes derived_status.
/// Progresses through: not_started → in_progress → complete.
#[tokio::test]
async fn derived_status_progression() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_production(&client).await {
        eprintln!("Production service not reachable -- skipping");
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let auth = format!("Bearer {}", make_jwt(&key, &tenant_id));

    // JWT gate
    let probe = client
        .get(format!("{}/api/production/work-orders", prod_url()))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("Production returns 401 with JWT -- skipping");
        return;
    }

    // Set up a 2-step routing
    let wc_id = create_workcenter(&client, &auth, &tenant_id, "Mill-E2E-A").await;
    let wc2_id = create_workcenter(&client, &auth, &tenant_id, "Mill-E2E-B").await;
    let routing_id = create_routing(&client, &auth, &tenant_id).await;
    add_step(&client, &auth, &tenant_id, routing_id, wc_id, 10, "Cut").await;
    add_step(&client, &auth, &tenant_id, routing_id, wc2_id, 20, "Weld").await;
    release_routing(&client, &auth, routing_id).await;

    // Create + release WO
    let wo_id = create_work_order(&client, &auth, &tenant_id, Some(routing_id)).await;
    release_work_order(&client, &auth, wo_id).await;

    // Check 1: derived_status = not_started (no operations yet)
    let wo = get_work_order(&client, &auth, wo_id).await;
    assert_eq!(
        wo["derived_status"], "not_started",
        "before init: derived_status must be not_started — got: {}",
        wo
    );
    println!("  [1] not_started: PASS");

    // Also verify it appears on the list endpoint
    let list_resp = client
        .get(format!("{}/api/production/work-orders", prod_url()))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("list WOs");
    assert_eq!(list_resp.status(), StatusCode::OK);
    let list_body: Value = list_resp.json().await.expect("list body");
    let list_items = list_body["data"].as_array().expect("data array");
    let found = list_items.iter().find(|w| {
        w.get("work_order_id").and_then(|v| v.as_str()) == Some(&wo_id.to_string())
    });
    if let Some(wo_in_list) = found {
        assert!(
            wo_in_list.get("derived_status").is_some(),
            "derived_status must be present on list response items"
        );
    }

    // Initialize operations from routing
    let init_resp = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/initialize",
            prod_url(),
            wo_id
        ))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("initialize operations");
    assert_eq!(init_resp.status(), StatusCode::CREATED, "initialize operations");

    // List operations and sort by sequence
    let ops_resp = client
        .get(format!(
            "{}/api/production/work-orders/{}/operations",
            prod_url(),
            wo_id
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("list operations");
    assert_eq!(ops_resp.status(), StatusCode::OK);
    let ops_body: Value = ops_resp.json().await.expect("ops body");
    let mut ops: Vec<Value> = ops_body["data"].as_array().expect("ops array").to_vec();
    ops.sort_by_key(|o| o["sequence_number"].as_u64().unwrap_or(u64::MAX));
    assert_eq!(ops.len(), 2, "should have 2 operations");

    let op1_id = ops[0]["operation_id"]
        .as_str()
        .or_else(|| ops[0]["id"].as_str())
        .expect("op1 id");
    let op2_id = ops[1]["operation_id"]
        .as_str()
        .or_else(|| ops[1]["id"].as_str())
        .expect("op2 id");

    // Start op1 → derived_status = in_progress
    let start1 = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/{}/start",
            prod_url(),
            wo_id,
            op1_id
        ))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("start op1");
    assert_eq!(start1.status(), StatusCode::OK, "start op1");

    let wo = get_work_order(&client, &auth, wo_id).await;
    assert_eq!(
        wo["derived_status"], "in_progress",
        "after starting op1: derived_status must be in_progress — got: {}",
        wo
    );
    println!("  [2] in_progress: PASS");

    // Complete op1
    let complete1 = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/{}/complete",
            prod_url(),
            wo_id,
            op1_id
        ))
        .header("Authorization", &auth)
        .json(&json!({ "actual_quantity": 10 }))
        .send()
        .await
        .expect("complete op1");
    assert_eq!(complete1.status(), StatusCode::OK, "complete op1");

    // Start + complete op2
    let start2 = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/{}/start",
            prod_url(),
            wo_id,
            op2_id
        ))
        .header("Authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .expect("start op2");
    assert_eq!(start2.status(), StatusCode::OK, "start op2");

    let complete2 = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/{}/complete",
            prod_url(),
            wo_id,
            op2_id
        ))
        .header("Authorization", &auth)
        .json(&json!({ "actual_quantity": 10 }))
        .send()
        .await
        .expect("complete op2");
    assert_eq!(complete2.status(), StatusCode::OK, "complete op2");

    // Both ops complete → derived_status = complete
    let wo = get_work_order(&client, &auth, wo_id).await;
    assert_eq!(
        wo["derived_status"], "complete",
        "after all ops complete: derived_status must be complete — got: {}",
        wo
    );
    println!("  [3] complete: PASS");
}

// ── Test 2: routing step enrichment ──────────────────────────────────────────

/// Without ?include, steps have no `workcenter` object (backward compat).
/// With ?include=workcenter_details, each step has a `workcenter` sub-object.
#[tokio::test]
async fn routing_step_workcenter_enrichment() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_production(&client).await {
        eprintln!("Production service not reachable -- skipping");
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let auth = format!("Bearer {}", make_jwt(&key, &tenant_id));

    let probe = client
        .get(format!("{}/api/production/workcenters", prod_url()))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("Production returns 401 with JWT -- skipping");
        return;
    }

    let wc_code = format!("WC-{}", &Uuid::new_v4().to_string()[..6].to_uppercase());
    let wc_name = format!("Lathe-{}", &Uuid::new_v4().to_string()[..6]);

    // Create workcenter with a known code and name
    let resp = client
        .post(format!("{}/api/production/workcenters", prod_url()))
        .header("Authorization", &auth)
        .json(&json!({
            "tenant_id": &tenant_id,
            "code": wc_code,
            "name": wc_name,
            "capacity": 2
        }))
        .send()
        .await
        .expect("create workcenter for enrichment test");
    let s = resp.status();
    let wc_body: Value = resp.json().await.expect("wc body");
    assert_eq!(s, StatusCode::CREATED, "create workcenter: {}", wc_body);
    let wc_id = Uuid::parse_str(
        wc_body["workcenter_id"].as_str().expect("workcenter_id"),
    )
    .unwrap();

    // Create routing and add one step
    let routing_id = create_routing(&client, &auth, &tenant_id).await;
    add_step(&client, &auth, &tenant_id, routing_id, wc_id, 10, "Turn").await;

    // ── Bare steps (no ?include) ──
    let bare_resp = client
        .get(format!(
            "{}/api/production/routings/{}/steps",
            prod_url(),
            routing_id
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("GET bare steps");
    assert_eq!(bare_resp.status(), StatusCode::OK, "GET bare steps");
    let bare_body: Value = bare_resp.json().await.expect("bare steps body");
    let bare_items = bare_body["data"].as_array().expect("bare data array");
    assert!(!bare_items.is_empty(), "should have steps");

    let bare_step = &bare_items[0];
    assert!(
        bare_step.get("workcenter").is_none(),
        "bare step must NOT have 'workcenter' sub-object — got: {}",
        bare_step
    );
    assert!(
        bare_step.get("workcenter_id").is_some(),
        "bare step must have workcenter_id"
    );
    println!("  backward_compat (no workcenter object): PASS");

    // ── Enriched steps (?include=workcenter_details) ──
    let enriched_resp = client
        .get(format!(
            "{}/api/production/routings/{}/steps?include=workcenter_details",
            prod_url(),
            routing_id
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("GET enriched steps");
    assert_eq!(enriched_resp.status(), StatusCode::OK, "GET enriched steps");
    let enriched_body: Value = enriched_resp.json().await.expect("enriched steps body");
    let enriched_items = enriched_body["data"].as_array().expect("enriched data array");
    assert!(!enriched_items.is_empty(), "should have enriched steps");

    let enriched_step = &enriched_items[0];
    let wc_obj = enriched_step
        .get("workcenter")
        .expect("enriched step must have 'workcenter' sub-object");
    assert!(
        !wc_obj.is_null(),
        "workcenter object must not be null for a valid workcenter"
    );
    assert_eq!(
        wc_obj["name"], json!(wc_name),
        "workcenter.name must match created workcenter"
    );
    assert_eq!(
        wc_obj["code"], json!(wc_code),
        "workcenter.code must match created workcenter"
    );

    println!(
        "  routing_step_workcenter_enrichment: PASS — workcenter: {}",
        wc_obj
    );
}

// ── Test 3: batch work orders ─────────────────────────────────────────────────

/// GET /api/production/work-orders?ids=... fetches multiple WOs in one call.
/// Empty id list → 400. Oversized list → 400.
#[tokio::test]
async fn batch_work_orders() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_production(&client).await {
        eprintln!("Production service not reachable -- skipping");
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let auth = format!("Bearer {}", make_jwt(&key, &tenant_id));

    let probe = client
        .get(format!("{}/api/production/work-orders", prod_url()))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("Production returns 401 with JWT -- skipping");
        return;
    }

    // Seed 3 work orders
    let wo1 = create_work_order(&client, &auth, &tenant_id, None).await;
    let wo2 = create_work_order(&client, &auth, &tenant_id, None).await;
    let wo3 = create_work_order(&client, &auth, &tenant_id, None).await;

    let ids_param = format!("{},{},{}", wo1, wo2, wo3);

    // Happy path: batch fetch 3 WOs
    let batch_resp = client
        .get(format!(
            "{}/api/production/work-orders?ids={}",
            prod_url(),
            ids_param
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("batch fetch");
    assert_eq!(batch_resp.status(), StatusCode::OK, "batch fetch -> 200");

    let batch_body: Value = batch_resp.json().await.expect("batch body");
    let items = batch_body.as_array().expect("batch returns a flat array");
    assert_eq!(items.len(), 3, "should return all 3 requested WOs");

    // Each item must have work_order_id and derived_status
    for item in items {
        assert!(
            item.get("work_order_id").is_some(),
            "batch item must have work_order_id: {}",
            item
        );
        assert!(
            item.get("derived_status").is_some(),
            "batch item must have derived_status: {}",
            item
        );
    }
    println!("  batch 3 WOs: PASS");

    // Error: empty ids string → 400
    let empty_resp = client
        .get(format!("{}/api/production/work-orders?ids=", prod_url()))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("empty ids request");
    assert_eq!(
        empty_resp.status(),
        StatusCode::BAD_REQUEST,
        "empty ids must return 400"
    );
    println!("  empty ids -> 400: PASS");

    // Error: more than 50 IDs → 400
    let oversized: Vec<String> = (0..51).map(|_| Uuid::new_v4().to_string()).collect();
    let oversized_param = oversized.join(",");
    let oversized_resp = client
        .get(format!(
            "{}/api/production/work-orders?ids={}",
            prod_url(),
            oversized_param
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("oversized ids request");
    assert_eq!(
        oversized_resp.status(),
        StatusCode::BAD_REQUEST,
        "51 ids must return 400"
    );
    println!("  oversized ids -> 400: PASS");

    // Batch with ?include=operations returns operations array (possibly empty)
    let with_ops_resp = client
        .get(format!(
            "{}/api/production/work-orders?ids={}&include=operations",
            prod_url(),
            ids_param
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("batch with operations");
    assert_eq!(
        with_ops_resp.status(),
        StatusCode::OK,
        "batch with include=operations -> 200"
    );
    let with_ops_body: Value = with_ops_resp.json().await.expect("batch+ops body");
    let ops_items = with_ops_body.as_array().expect("batch+ops returns array");
    assert_eq!(ops_items.len(), 3, "should return 3 WOs with operations include");
    for item in ops_items {
        // operations key must be present (may be empty array for WOs with no ops)
        assert!(
            item.get("operations").is_some(),
            "item must have 'operations' key when include=operations: {}",
            item
        );
    }
    println!("  batch with include=operations: PASS");
}

// ── Test 4: composite WO create ───────────────────────────────────────────────

/// POST /api/production/work-orders/create allocates a number from Numbering
/// and creates the WO in one call.
/// Skipped if the Numbering service is not reachable.
#[tokio::test]
async fn composite_create_work_order() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap();

    if !wait_for_production(&client).await {
        eprintln!("Production service not reachable -- skipping");
        return;
    }

    if !numbering_reachable(&client).await {
        eprintln!(
            "Numbering service not reachable at {} -- skipping composite create test",
            numbering_url()
        );
        return;
    }

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let auth = format!("Bearer {}", make_jwt(&key, &tenant_id));

    let probe = client
        .get(format!("{}/api/production/work-orders", prod_url()))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("JWT probe");
    if probe.status().as_u16() == 401 {
        eprintln!("Production returns 401 with JWT -- skipping");
        return;
    }

    let item_id = Uuid::new_v4();
    let idempotency_key = Uuid::new_v4().to_string();

    // Composite create (number only — no BOM or routing)
    let resp = client
        .post(format!("{}/api/production/work-orders/create", prod_url()))
        .header("Authorization", &auth)
        .json(&json!({
            "item_id": item_id,
            "planned_quantity": 5,
            "idempotency_key": idempotency_key
        }))
        .send()
        .await
        .expect("composite create WO");
    let s = resp.status();
    let body: Value = resp.json().await.expect("composite WO body");
    if s == StatusCode::SERVICE_UNAVAILABLE {
        eprintln!(
            "Production returned 503 for composite create (numbering service unreachable from inside Docker) -- skipping: {}",
            body
        );
        return;
    }
    assert_eq!(s, StatusCode::CREATED, "composite create -> 201: {}", body);

    let order_number = body["order_number"].as_str().expect("order_number in response");
    assert!(
        !order_number.is_empty(),
        "order_number must be allocated (non-empty)"
    );
    assert!(
        body.get("work_order_id").is_some(),
        "work_order_id must be present"
    );
    assert_eq!(body["item_id"], json!(item_id), "item_id must match");
    println!("  composite create (number only): PASS — order_number={}", order_number);

    // Idempotent re-send returns same WO
    let resp2 = client
        .post(format!("{}/api/production/work-orders/create", prod_url()))
        .header("Authorization", &auth)
        .json(&json!({
            "item_id": item_id,
            "planned_quantity": 5,
            "idempotency_key": idempotency_key
        }))
        .send()
        .await
        .expect("composite create WO idempotent");
    let s2 = resp2.status();
    let body2: Value = resp2.json().await.expect("idempotent WO body");
    // 201 or 200 depending on implementation; order_number must be same
    assert!(
        s2 == StatusCode::CREATED || s2 == StatusCode::OK,
        "idempotent re-send should succeed: {} - {}",
        s2,
        body2
    );
    assert_eq!(
        body2["order_number"].as_str().unwrap_or(""),
        order_number,
        "idempotent re-send must return same order_number"
    );
    println!("  composite create (idempotent): PASS");

    // Composite create WITH optional bom_revision_id and routing_template_id
    let bom_rev_id = Uuid::new_v4();
    let wc_id = create_workcenter(&client, &auth, &tenant_id, "Mill-Composite").await;
    let routing_id = create_routing(&client, &auth, &tenant_id).await;
    add_step(&client, &auth, &tenant_id, routing_id, wc_id, 10, "Assemble").await;
    release_routing(&client, &auth, routing_id).await;

    let full_key = Uuid::new_v4().to_string();
    let full_resp = client
        .post(format!("{}/api/production/work-orders/create", prod_url()))
        .header("Authorization", &auth)
        .json(&json!({
            "item_id": Uuid::new_v4(),
            "bom_revision_id": bom_rev_id,
            "routing_template_id": routing_id,
            "planned_quantity": 20,
            "idempotency_key": full_key
        }))
        .send()
        .await
        .expect("composite create full");
    let full_s = full_resp.status();
    let full_body: Value = full_resp.json().await.expect("full composite body");
    assert_eq!(full_s, StatusCode::CREATED, "composite create with BOM+routing -> 201: {}", full_body);
    assert_eq!(
        full_body["bom_revision_id"],
        json!(bom_rev_id),
        "bom_revision_id must be attached"
    );
    assert_eq!(
        full_body["routing_template_id"],
        json!(routing_id),
        "routing_template_id must be attached"
    );
    let full_order = full_body["order_number"].as_str().expect("order_number");
    assert!(!full_order.is_empty(), "order_number must be allocated");
    println!("  composite create (with BOM+routing): PASS — order_number={}", full_order);
}
