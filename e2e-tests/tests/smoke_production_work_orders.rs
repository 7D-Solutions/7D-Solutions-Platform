//! HTTP smoke tests for Production Work Orders + Operations
//! Covers all 14 production routes via reqwest against the live service.
//! Full WO lifecycle: create → release → init ops → start → complete → fg receipt → close

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;
use uuid::Uuid;

const BASE_URL: &'static str = "http://localhost:8108";

#[derive(Debug, Serialize)]
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
        roles: vec!["admin".to_string()],
        perms: vec![
            "production.mutate".to_string(),
            "production.read".to_string(),
        ],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

async fn wait_for_health(client: &Client) -> bool {
    let url = format!("{}/api/health", BASE_URL);
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        _ => panic!("Unsupported method: {}", method),
    };
    let res = req
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(
        res.status().as_u16(),
        401,
        "expected 401 without JWT at {} {}",
        method,
        url
    );
    println!("  no-JWT -> 401 ok");
}

fn extract_id(body: &Value, keys: &[&str]) -> Uuid {
    for k in keys {
        if let Some(v) = body.get(k) {
            if let Some(s) = v.as_str() {
                if let Ok(u) = Uuid::parse_str(s) {
                    return u;
                }
            }
        }
    }
    panic!("Could not find ID in {} using keys {:?}", body, keys);
}

#[tokio::test]
async fn smoke_production_work_orders() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_health(&client).await {
        eprintln!(
            "Production service not reachable at {} -- skipping",
            BASE_URL
        );
        return;
    }
    println!("Production service healthy at {}", BASE_URL);

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id);
    let auth = format!("Bearer {}", jwt);

    // --- Step 0: JWT gate ---
    println!("Step 0: JWT gate - verify unauth rejection");
    let unauth_routes = vec![
        ("GET", format!("{}/api/production/workcenters", BASE_URL)),
        (
            "GET",
            format!("{}/api/production/work-orders/{}", BASE_URL, Uuid::new_v4()),
        ),
    ];
    for (m, url) in &unauth_routes {
        assert_unauth(&client, m, url).await;
    }
    println!(
        "  All {} routes correctly reject unauth",
        unauth_routes.len()
    );

    // Probe: verify our JWT is accepted
    let probe = client
        .get(format!(
            "{}/api/production/work-orders/{}",
            BASE_URL,
            Uuid::new_v4()
        ))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_ne!(probe.status().as_u16(), 401, "JWT should be accepted");
    println!("  JWT probe: {} (not 401)", probe.status());

    // --- Step 1: Create workcenter ---
    println!("Step 1: Create workcenter");
    let wc_code = format!("WC-SMK-{}", &Uuid::new_v4().to_string()[..8]);
    let res = client
        .post(format!("{}/api/production/workcenters", BASE_URL))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "code": wc_code,
            "name": "Smoke Test Workcenter",
            "capacity": 10,
            "cost_rate_minor": 5000
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::CREATED, "Create workcenter: {}", body);
    let wc_val: Value = serde_json::from_str(&body).unwrap();
    let wc_id = extract_id(&wc_val, &["workcenter_id", "id"]);
    println!("  Created workcenter: {}", wc_id);

    // --- Step 2: List workcenters ---
    println!("Step 2: List workcenters");
    let res = client
        .get(format!(
            "{}/api/production/workcenters?tenant_id={}",
            BASE_URL, tenant_id
        ))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "List workcenters");
    println!("  List workcenters OK");

    // --- Step 3: Get workcenter by ID ---
    println!("Step 3: Get workcenter");
    let res = client
        .get(format!("{}/api/production/workcenters/{}", BASE_URL, wc_id))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "Get workcenter");
    println!("  Get workcenter OK");

    // --- Step 4: Update workcenter ---
    println!("Step 4: Update workcenter");
    let res = client
        .put(format!("{}/api/production/workcenters/{}", BASE_URL, wc_id))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "name": "Updated Smoke Workcenter",
            "capacity": 20
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::OK, "Update workcenter: {}", body);
    println!("  Update workcenter OK");

    // --- Step 5: Deactivate workcenter (separate one) ---
    println!("Step 5: Deactivate workcenter");
    let wc2_code = format!("WC-DAC-{}", &Uuid::new_v4().to_string()[..8]);
    let res = client
        .post(format!("{}/api/production/workcenters", BASE_URL))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "code": wc2_code,
            "name": "Deactivation Test WC"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let wc2_val: Value = res.json().await.unwrap();
    let wc2_id = extract_id(&wc2_val, &["workcenter_id", "id"]);
    let res = client
        .post(format!(
            "{}/api/production/workcenters/{}/deactivate",
            BASE_URL, wc2_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    assert!(
        st == StatusCode::OK || st == StatusCode::NO_CONTENT,
        "Deactivate workcenter: got {}",
        st
    );
    println!("  Deactivate workcenter OK");

    // --- Step 6: Create routing template with a step ---
    println!("Step 6: Create routing template");
    let res = client
        .post(format!("{}/api/production/routings", BASE_URL))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "name": "Smoke Test Routing",
            "description": "Routing for smoke test"
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::CREATED, "Create routing: {}", body);
    let routing_val: Value = serde_json::from_str(&body).unwrap();
    let routing_id = extract_id(&routing_val, &["routing_template_id", "id"]);
    println!("  Created routing: {}", routing_id);

    // Add a step to the routing
    println!("  Adding step to routing");
    let res = client
        .post(format!(
            "{}/api/production/routings/{}/steps",
            BASE_URL, routing_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "sequence_number": 10,
            "workcenter_id": wc_id.to_string(),
            "operation_name": "Assembly",
            "setup_time_minutes": 15,
            "run_time_minutes": 45,
            "is_required": true
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::CREATED, "Add routing step: {}", body);
    println!("  Added routing step OK");

    // Release the routing
    println!("  Releasing routing");
    let res = client
        .post(format!(
            "{}/api/production/routings/{}/release",
            BASE_URL, routing_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert!(st.is_success(), "Release routing: got {} {}", st, body);
    println!("  Released routing OK");

    // --- Step 7: Create work order ---
    println!("Step 7: Create work order");
    let order_number = format!("WO-SMK-{}", &Uuid::new_v4().to_string()[..8]);
    let item_id = Uuid::new_v4();
    let bom_rev_id = Uuid::new_v4();
    let res = client
        .post(format!("{}/api/production/work-orders", BASE_URL))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "order_number": order_number,
            "item_id": item_id.to_string(),
            "bom_revision_id": bom_rev_id.to_string(),
            "routing_template_id": routing_id.to_string(),
            "planned_quantity": 100
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::CREATED, "Create work order: {}", body);
    let wo_val: Value = serde_json::from_str(&body).unwrap();
    let wo_id = extract_id(&wo_val, &["work_order_id", "id"]);
    println!("  Created work order: {}", wo_id);

    // --- Step 8: Get work order ---
    println!("Step 8: Get work order");
    let res = client
        .get(format!("{}/api/production/work-orders/{}", BASE_URL, wo_id))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "Get work order");
    println!("  Get work order OK");

    // --- Step 9: Release work order ---
    println!("Step 9: Release work order");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/release",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert!(st.is_success(), "Release WO: got {} {}", st, body);
    println!("  Released work order OK");

    // --- Step 10: Initialize operations ---
    println!("Step 10: Initialize operations");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/initialize",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::CREATED, "Initialize operations: {}", body);
    let ops_arr: Value = serde_json::from_str(&body).unwrap();
    let ops = ops_arr.as_array().expect("ops should be array");
    assert!(!ops.is_empty(), "Should have at least one operation");
    let op_id = extract_id(&ops[0], &["operation_id", "id"]);
    println!("  Initialized {} operation(s), first: {}", ops.len(), op_id);

    // --- Step 11: List operations ---
    println!("Step 11: List operations");
    let res = client
        .get(format!(
            "{}/api/production/work-orders/{}/operations",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "List operations");
    println!("  List operations OK");

    // --- Step 12: Start operation ---
    println!("Step 12: Start operation");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/{}/start",
            BASE_URL, wo_id, op_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::OK, "Start operation: {}", body);
    println!("  Start operation OK");

    // --- Step 13: Complete operation ---
    println!("Step 13: Complete operation");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/operations/{}/complete",
            BASE_URL, wo_id, op_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::OK, "Complete operation: {}", body);
    println!("  Complete operation OK");

    // --- Step 14: Component issue ---
    println!("Step 14: Component issue");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/component-issues",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "items": [{
                "item_id": Uuid::new_v4().to_string(),
                "warehouse_id": Uuid::new_v4().to_string(),
                "quantity": 10,
                "currency": "USD"
            }],
            "correlation_id": Uuid::new_v4().to_string(),
            "causation_id": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::ACCEPTED, "Component issue: {}", body);
    println!("  Component issue OK (202)");

    // --- Step 15: FG receipt ---
    println!("Step 15: FG receipt");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/fg-receipt",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "item_id": item_id.to_string(),
            "warehouse_id": Uuid::new_v4().to_string(),
            "quantity": 100,
            "currency": "USD",
            "correlation_id": Uuid::new_v4().to_string(),
            "causation_id": Uuid::new_v4().to_string()
        }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert_eq!(st, StatusCode::ACCEPTED, "FG receipt: {}", body);
    println!("  FG receipt OK (202)");

    // --- Step 16: Get time entries ---
    println!("Step 16: Get time entries");
    let res = client
        .get(format!(
            "{}/api/production/work-orders/{}/time-entries",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "Get time entries");
    println!("  Get time entries OK");

    // --- Step 17: Close work order ---
    println!("Step 17: Close work order");
    let res = client
        .post(format!(
            "{}/api/production/work-orders/{}/close",
            BASE_URL, wo_id
        ))
        .header("authorization", &auth)
        .json(&serde_json::json!({ "tenant_id": tenant_id }))
        .send()
        .await
        .unwrap();
    let st = res.status();
    let body = res.text().await.unwrap();
    assert!(st.is_success(), "Close WO: got {} {}", st, body);
    println!("  Close work order OK");

    println!("\nAll 17 steps passed (14 production routes + 3 prereq routing routes)!");
}
