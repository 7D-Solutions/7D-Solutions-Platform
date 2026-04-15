// HTTP smoke tests: Maintenance Assets + Plans
//
// Proves that 13 core maintenance routes respond correctly at the HTTP
// boundary via reqwest against the live maintenance service.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use uuid::Uuid;

const MAINTENANCE_DEFAULT_URL: &str = "http://localhost:8101";

fn maintenance_url() -> String {
    std::env::var("MAINTENANCE_URL").unwrap_or_else(|_| MAINTENANCE_DEFAULT_URL.to_string())
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

async fn wait_for_service(client: &Client) -> bool {
    let url = format!("{}/api/health", maintenance_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  maintenance health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  maintenance health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PATCH" => client.patch(url),
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
async fn smoke_maintenance_assets() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    if !wait_for_service(&client).await {
        eprintln!(
            "Maintenance service not reachable at {} -- skipping",
            maintenance_url()
        );
        return;
    }
    println!("Maintenance service healthy at {}", maintenance_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["maintenance.mutate", "maintenance.read"],
    );
    let base = maintenance_url();

    let probe = client
        .get(format!("{base}/api/health"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!(
            "Maintenance returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping."
        );
        return;
    }

    let now = Utc::now();

    println!("\n--- 1. POST /api/maintenance/assets ---");
    let asset_tag = format!("SMOKE-{}", &Uuid::new_v4().to_string()[..8]);
    let resp = client
        .post(format!("{base}/api/maintenance/assets"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "asset_tag": asset_tag,
            "name": "Smoke Test CNC Mill",
            "description": "Smoke test asset",
            "asset_type": "machinery",
            "location": "Shop Floor A",
            "department": "Manufacturing"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create asset failed: {status} - {body}"
    );
    let asset_id = body["id"].as_str().expect("No id in asset response");
    println!("  created asset id={asset_id} tag={asset_tag}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/maintenance/assets"),
        Some(json!({"asset_tag": "X", "name": "X", "asset_type": "machinery"})),
    )
    .await;

    println!("\n--- 2. GET /api/maintenance/assets/{{asset_id}} ---");
    let resp = client
        .get(format!("{base}/api/maintenance/assets/{asset_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get asset failed: {}",
        resp.status()
    );
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["name"], "Smoke Test CNC Mill");
    println!("  retrieved asset name={}", fetched["name"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/maintenance/assets/{asset_id}"),
        None,
    )
    .await;

    println!("\n--- 10. POST /api/maintenance/meter-types (prerequisite) ---");
    let resp = client
        .post(format!("{base}/api/maintenance/meter-types"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "name": format!("Hours-{}", &Uuid::new_v4().to_string()[..8]),
            "unit_label": "hours"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let mt_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create meter type failed: {status} - {mt_body}"
    );
    let meter_type_id = mt_body["id"].as_str().expect("No meter type id");
    println!("  created meter type id={meter_type_id}");

    println!("\n--- 3. POST /api/maintenance/assets/{{asset_id}}/readings ---");
    let resp = client
        .post(format!("{base}/api/maintenance/assets/{asset_id}/readings"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "meter_type_id": meter_type_id,
            "reading_value": 1000,
            "recorded_at": now.to_rfc3339(),
            "recorded_by": "smoke-test"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let reading_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Record reading failed: {status} - {reading_body}"
    );
    println!("  recorded reading value=1000");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/maintenance/assets/{asset_id}/readings"),
        Some(json!({"meter_type_id": meter_type_id, "reading_value": 100})),
    )
    .await;

    println!("\n--- 4. POST /api/maintenance/assets/{{asset_id}}/calibration-events ---");
    let due_at = now + chrono::Duration::days(365);
    let resp = client
        .post(format!(
            "{base}/api/maintenance/assets/{asset_id}/calibration-events"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "performed_at": now.to_rfc3339(),
            "due_at": due_at.to_rfc3339(), "result": "pass"
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let cal_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Record calibration failed: {status} - {cal_body}"
    );
    println!("  recorded calibration result=pass");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/maintenance/assets/{asset_id}/calibration-events"),
        Some(json!({"performed_at": now.to_rfc3339(), "due_at": due_at.to_rfc3339(), "result": "pass"})),
    )
    .await;

    println!("\n--- 5. GET /api/maintenance/assets/{{asset_id}}/calibration-status ---");
    let resp = client
        .get(format!(
            "{base}/api/maintenance/assets/{asset_id}/calibration-status"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Calibration status failed: {}",
        resp.status()
    );
    let cal_status: Value = resp.json().await.unwrap();
    println!("  calibration status={cal_status}");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/maintenance/assets/{asset_id}/calibration-status"),
        None,
    )
    .await;

    println!("\n--- 6. GET /api/maintenance/assets/{{asset_id}}/downtime ---");
    let resp = client
        .get(format!("{base}/api/maintenance/assets/{asset_id}/downtime"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Asset downtime failed: {}",
        resp.status()
    );
    let dt: Value = resp.json().await.unwrap();
    println!(
        "  asset downtime returned {} event(s)",
        dt.as_array().map_or(0, |a| a.len())
    );

    println!("\n--- 7. POST /api/maintenance/plans ---");
    let resp = client
        .post(format!("{base}/api/maintenance/plans"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "name": "Smoke Test Preventive Plan",
            "description": "30-day calendar-based inspection",
            "asset_type_filter": "machinery",
            "schedule_type": "calendar",
            "calendar_interval_days": 30,
            "priority": "medium",
            "estimated_duration_minutes": 60,
            "estimated_cost_minor": 5000
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let plan_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Create plan failed: {status} - {plan_body}"
    );
    let plan_id = plan_body["id"].as_str().expect("No id in plan response");
    println!("  created plan id={plan_id}");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/maintenance/plans"),
        Some(json!({"name": "X", "schedule_type": "calendar", "calendar_interval_days": 7})),
    )
    .await;

    println!("\n--- 8. GET /api/maintenance/plans/{{plan_id}} ---");
    let resp = client
        .get(format!("{base}/api/maintenance/plans/{plan_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get plan failed: {}",
        resp.status()
    );
    let fetched_plan: Value = resp.json().await.unwrap();
    assert_eq!(fetched_plan["name"], "Smoke Test Preventive Plan");
    println!("  retrieved plan name={}", fetched_plan["name"]);

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/maintenance/plans/{plan_id}"),
        None,
    )
    .await;

    println!("\n--- 9. POST /api/maintenance/plans/{{plan_id}}/assign ---");
    let resp = client
        .post(format!("{base}/api/maintenance/plans/{plan_id}/assign"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "asset_id": asset_id
        }))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let assign_body: Value = resp.json().await.unwrap_or(json!({}));
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "Assign plan failed: {status} - {assign_body}"
    );
    println!("  assigned plan to asset");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/maintenance/plans/{plan_id}/assign"),
        Some(json!({"asset_id": Uuid::new_v4()})),
    )
    .await;

    println!("\n--- 10. POST /api/maintenance/meter-types (verified above) ---");
    println!("  meter type id={meter_type_id} -- verified");

    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/maintenance/meter-types"),
        Some(json!({"name": "X", "unit_label": "X"})),
    )
    .await;

    println!("\n--- 11. GET /api/maintenance/assignments ---");
    let resp = client
        .get(format!("{base}/api/maintenance/assignments"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List assignments failed: {}",
        resp.status()
    );
    let assignments: Value = resp.json().await.unwrap();
    let assignment_count = assignments.as_array().map_or(0, |a| a.len());
    assert!(
        assignment_count > 0,
        "Expected at least 1 assignment after assigning plan"
    );
    println!("  listed {assignment_count} assignment(s)");

    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/maintenance/assignments"),
        None,
    )
    .await;

    println!("\n--- 12. GET /api/maintenance/downtime-events ---");
    let resp = client
        .get(format!("{base}/api/maintenance/downtime-events"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "List downtime events failed: {}",
        resp.status()
    );
    let dt_events: Value = resp.json().await.unwrap();
    println!(
        "  listed {} downtime event(s)",
        dt_events.as_array().map_or(0, |a| a.len())
    );

    println!("\n--- 13. GET /api/maintenance/downtime-events/{{id}} ---");
    let dt_resp = client
        .post(format!("{base}/api/maintenance/downtime-events"))
        .bearer_auth(&jwt)
        .json(&json!({
            "tenant_id": "", "asset_id": asset_id,
            "start_time": now.to_rfc3339(),
            "reason": "Smoke test downtime",
            "impact_classification": "minor"
        }))
        .send()
        .await
        .unwrap();
    let dt_status = dt_resp.status();
    let dt_body: Value = dt_resp.json().await.unwrap_or(json!({}));
    assert!(
        dt_status == StatusCode::CREATED || dt_status == StatusCode::OK,
        "Create downtime event failed: {dt_status} - {dt_body}"
    );
    let dt_id = dt_body["id"].as_str().expect("No downtime event id");

    let resp = client
        .get(format!("{base}/api/maintenance/downtime-events/{dt_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "Get downtime event failed: {}",
        resp.status()
    );
    let dt_detail: Value = resp.json().await.unwrap();
    assert_eq!(dt_detail["reason"], "Smoke test downtime");
    println!(
        "  retrieved downtime event id={dt_id} reason={}",
        dt_detail["reason"]
    );
    println!("\n=== All 13 Maintenance Assets + Plans routes passed ===");
}
