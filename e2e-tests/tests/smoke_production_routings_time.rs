// HTTP smoke tests: Production Routings, Time Entries, and Downtime
//
// Proves that 12 core production routes respond correctly at the
// HTTP boundary via reqwest against the live Production service.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const PROD_DEFAULT_URL: &str = "http://localhost:8108";

fn prod_url() -> String {
    std::env::var("PRODUCTION_URL").unwrap_or_else(|_| PROD_DEFAULT_URL.to_string())
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
        perms: vec!["*".to_string()],
        actor_type: "user".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, key).unwrap()
}

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

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        _ => panic!("Unsupported method: {}", method),
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

fn extract_id(val: &Value, keys: &[&str]) -> String {
    for key in keys {
        if let Some(id) = val[key].as_str() {
            return id.to_string();
        }
    }
    panic!("No ID found in response: {}", val);
}

#[tokio::test]
async fn smoke_production_routings_time() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_production(&client).await {
        eprintln!(
            "Production service not reachable at {} -- skipping",
            prod_url()
        );
        return;
    }
    println!("Production service healthy at {}", prod_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(&key, &tenant_id);
    let base = prod_url();

    // Gate: verify JWT is accepted
    let probe = client
        .get(format!("{base}/api/production/routings"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Production returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    // Setup: create workcenter (needed for routing steps and downtime)
    println!("\n--- Setup: create workcenter ---");
    let wc_code = format!("WC-{}", &Uuid::new_v4().to_string()[..6]);
    let resp = client
        .post(format!("{base}/api/production/workcenters"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "code": wc_code,
            "name": "Routings Smoke Workcenter",
            "capacity": 8,
            "cost_rate_minor": 3500
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let wc: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Create workcenter failed: {status} - {wc}"
    );
    let wc_id = extract_id(&wc, &["workcenter_id", "id"]);
    println!("  created workcenter id={wc_id}");

    let item_id = Uuid::new_v4();

    // 1. POST /api/production/routings
    println!("\n--- 1. POST /api/production/routings ---");
    let resp = client
        .post(format!("{base}/api/production/routings"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "name": "Smoke Routing Template",
            "item_id": item_id,
            "revision": "A"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let rt: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create routing failed: {status} - {rt}"
    );
    let routing_id = extract_id(&rt, &["routing_template_id", "id"]);
    println!("  created routing id={routing_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/routings"),
        Some(json!({"tenant_id": "x", "name": "X", "revision": "1"})),
    )
    .await;

    // Setup: add routing step before releasing (makes GET steps meaningful)
    println!("\n--- Setup: add routing step ---");
    let resp = client
        .post(format!("{base}/api/production/routings/{routing_id}/steps"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "sequence_number": 10,
            "workcenter_id": wc_id,
            "operation_name": "Assemble",
            "setup_time_minutes": 15,
            "run_time_minutes": 45
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let step_val: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Add routing step failed: {status} - {step_val}"
    );
    println!("  added routing step");

    // 2. GET /api/production/routings/{id}
    println!("\n--- 2. GET /api/production/routings/{{id}} ---");
    let resp = client
        .get(format!("{base}/api/production/routings/{routing_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "GET routing failed: {status}");
    let fetched: Value = resp.json().await.unwrap_or(json!({}));
    println!("  routing name={}", fetched["name"]);
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/production/routings/{routing_id}"),
        None,
    )
    .await;

    // 3. POST /api/production/routings/{id}/release
    println!("\n--- 3. POST /api/production/routings/{{id}}/release ---");
    let resp = client
        .post(format!(
            "{base}/api/production/routings/{routing_id}/release"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let released: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Release routing failed: {status} - {released}"
    );
    println!("  routing status={}", released["status"]);
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/routings/{routing_id}/release"),
        None,
    )
    .await;

    // 4. GET /api/production/routings/{id}/steps
    println!("\n--- 4. GET /api/production/routings/{{id}}/steps ---");
    let resp = client
        .get(format!(
            "{base}/api/production/routings/{routing_id}/steps"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let steps: Value = resp.json().await.unwrap_or(json!([]));
    assert!(status.is_success(), "GET routing steps failed: {status}");
    assert!(steps.is_array(), "Routing steps should be an array");
    println!(
        "  {} routing steps",
        steps.as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/production/routings/{routing_id}/steps"),
        None,
    )
    .await;

    // 5. GET /api/production/routings/by-item
    println!("\n--- 5. GET /api/production/routings/by-item ---");
    let today = Utc::now().format("%Y-%m-%d").to_string();
    let resp = client
        .get(format!(
            "{base}/api/production/routings/by-item?item_id={item_id}&effective_date={today}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let by_item: Value = resp.json().await.unwrap_or(json!([]));
    assert!(
        status.is_success(),
        "GET routings by-item failed: {status}"
    );
    assert!(by_item.is_array(), "by-item should return an array");
    println!(
        "  {} routings for item",
        by_item.as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(
        &client,
        "GET",
        &format!(
            "{base}/api/production/routings/by-item?item_id={item_id}&effective_date={today}"
        ),
        None,
    )
    .await;

    // Setup: create work order for time entry tests
    println!("\n--- Setup: create work order ---");
    let order_number = format!("WO-SMOKE-{}", &Uuid::new_v4().to_string()[..6]);
    let bom_rev_id = Uuid::new_v4();
    let resp = client
        .post(format!("{base}/api/production/work-orders"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "order_number": order_number,
            "item_id": item_id,
            "bom_revision_id": bom_rev_id,
            "routing_template_id": routing_id,
            "planned_quantity": 50
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let wo: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Create work order failed: {status} - {wo}"
    );
    let wo_id = extract_id(&wo, &["work_order_id", "id"]);
    println!("  created work order id={wo_id}");

    println!("\n--- Setup: release work order ---");
    let resp = client
        .post(format!("{base}/api/production/work-orders/{wo_id}/release"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    assert!(status.is_success(), "Release work order failed: {status}");
    println!("  work order released");

    // 6. POST /api/production/time-entries/start
    println!("\n--- 6. POST /api/production/time-entries/start ---");
    let resp = client
        .post(format!("{base}/api/production/time-entries/start"))
        .bearer_auth(&jwt)
        .json(&json!({
            "work_order_id": wo_id,
            "actor_id": "operator-01"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let te: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Start timer failed: {status} - {te}"
    );
    let te_id = extract_id(&te, &["time_entry_id", "id"]);
    println!("  timer started id={te_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/time-entries/start"),
        Some(json!({"work_order_id": wo_id, "actor_id": "x"})),
    )
    .await;

    // 7. POST /api/production/time-entries/manual
    println!("\n--- 7. POST /api/production/time-entries/manual ---");
    let start_ts = Utc::now() - chrono::Duration::hours(3);
    let end_ts = Utc::now() - chrono::Duration::hours(2);
    let resp = client
        .post(format!("{base}/api/production/time-entries/manual"))
        .bearer_auth(&jwt)
        .json(&json!({
            "work_order_id": wo_id,
            "actor_id": "operator-02",
            "start_ts": start_ts.to_rfc3339(),
            "end_ts": end_ts.to_rfc3339(),
            "minutes": 60
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let manual: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Manual entry failed: {status} - {manual}"
    );
    println!("  manual entry created: minutes={}", manual["minutes"]);
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/time-entries/manual"),
        Some(json!({})),
    )
    .await;

    // 8. POST /api/production/time-entries/{id}/stop
    println!("\n--- 8. POST /api/production/time-entries/{{id}}/stop ---");
    let resp = client
        .post(format!(
            "{base}/api/production/time-entries/{te_id}/stop"
        ))
        .bearer_auth(&jwt)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let stopped: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "Stop timer failed: {status} - {stopped}"
    );
    println!("  timer stopped: minutes={}", stopped["minutes"]);
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/time-entries/{te_id}/stop"),
        Some(json!({})),
    )
    .await;

    // 9. POST /api/production/workcenters/{id}/downtime/start
    println!("\n--- 9. POST /api/production/workcenters/{{id}}/downtime/start ---");
    let resp = client
        .post(format!(
            "{base}/api/production/workcenters/{wc_id}/downtime/start"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "workcenter_id": wc_id,
            "reason": "Planned maintenance",
            "reason_code": "PM",
            "started_by": "supervisor-01"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let dt: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Start downtime failed: {status} - {dt}"
    );
    let dt_id = extract_id(&dt, &["downtime_id", "id"]);
    println!("  downtime started id={dt_id}");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/workcenters/{wc_id}/downtime/start"),
        Some(json!({"tenant_id": "x", "workcenter_id": wc_id, "reason": "X"})),
    )
    .await;

    // 10. GET /api/production/workcenters/{id}/downtime
    println!("\n--- 10. GET /api/production/workcenters/{{id}}/downtime ---");
    let resp = client
        .get(format!(
            "{base}/api/production/workcenters/{wc_id}/downtime"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let dt_list: Value = resp.json().await.unwrap_or(json!([]));
    assert!(
        status.is_success(),
        "GET workcenter downtime failed: {status}"
    );
    assert!(dt_list.is_array(), "Downtime list should be an array");
    println!(
        "  {} downtime records for workcenter",
        dt_list.as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/production/workcenters/{wc_id}/downtime"),
        None,
    )
    .await;

    // 11. POST /api/production/downtime/{id}/end
    println!("\n--- 11. POST /api/production/downtime/{{id}}/end ---");
    let resp = client
        .post(format!("{base}/api/production/downtime/{dt_id}/end"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": tenant_id,
            "ended_by": "supervisor-01"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let ended: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status.is_success(),
        "End downtime failed: {status} - {ended}"
    );
    println!("  downtime ended");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/production/downtime/{dt_id}/end"),
        Some(json!({"tenant_id": "x"})),
    )
    .await;

    // 12. GET /api/production/downtime/active
    println!("\n--- 12. GET /api/production/downtime/active ---");
    let resp = client
        .get(format!("{base}/api/production/downtime/active"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let active: Value = resp.json().await.unwrap_or(json!([]));
    assert!(
        status.is_success(),
        "GET active downtime failed: {status}"
    );
    assert!(active.is_array(), "Active downtime should be an array");
    println!(
        "  {} active downtime records",
        active.as_array().map(|a| a.len()).unwrap_or(0)
    );
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/production/downtime/active"),
        None,
    )
    .await;

    println!("\n=== All 12 production routings/time/downtime routes passed ===");
}
