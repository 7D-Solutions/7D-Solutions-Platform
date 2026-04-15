// HTTP smoke tests: Quality Inspection
//
// Proves that all 15 Quality Inspection routes respond correctly at the HTTP
// boundary via reqwest against the live Quality Inspection service (port 8106).
//
// Lifecycle covered:
//   plan → activate → receiving inspection → hold → accept
//                                          → hold → reject
//                                          → hold → release
//   in-process inspection (create only)
//   final inspection (create only)
//   query routes: by-part-rev, by-receipt, by-wo, by-lot
//
// Inspector authorization is seeded directly into the workforce-competence DB
// before HTTP calls are made. No mocks, no stubs.

use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;
use uuid::Uuid;
use workforce_competence_rs::domain::{
    models::{ArtifactType, AssignCompetenceRequest, RegisterArtifactRequest},
    service as wc_service,
};

const QI_DEFAULT_URL: &str = "http://localhost:8106";

fn qi_url() -> String {
    std::env::var("QUALITY_INSPECTION_URL").unwrap_or_else(|_| QI_DEFAULT_URL.to_string())
}

fn wc_db_url() -> String {
    std::env::var("WORKFORCE_COMPETENCE_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://wc_user:wc_pass@localhost:5458/workforce_competence_db?sslmode=require"
            .to_string()
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

async fn wait_for_qi(client: &Client) -> bool {
    let url = format!("{}/api/health", qi_url());
    for attempt in 1..=15 {
        match client.get(&url).send().await {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => eprintln!("  QI health {}/15: {}", attempt, r.status()),
            Err(e) => eprintln!("  QI health {}/15: {}", attempt, e),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

/// All QI routes require auth (RequirePermissionsLayer). Expect 401 without JWT.
async fn assert_unauth(client: &Client, method: &str, url: &str, body: Option<Value>) {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
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

fn extract_uuid(body: &Value, key: &str) -> Uuid {
    Uuid::parse_str(
        body.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("missing '{}' in: {}", key, body)),
    )
    .unwrap_or_else(|_| panic!("invalid UUID for '{}' in: {}", key, body))
}

/// Seed inspector authorization in the workforce-competence DB.
async fn authorize_inspector(tenant_id: &str, inspector_id: Uuid) {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&wc_db_url())
        .await
        .expect("Failed to connect to workforce-competence DB");

    sqlx::migrate!("../modules/workforce-competence/db/migrations")
        .run(&pool)
        .await
        .expect("WC migrations failed");

    let artifact_req = RegisterArtifactRequest {
        tenant_id: tenant_id.to_string(),
        artifact_type: ArtifactType::Qualification,
        name: "Quality Inspection Disposition Authority".to_string(),
        code: "quality_inspection".to_string(),
        description: Some("Smoke test inspector auth".to_string()),
        valid_duration_days: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("smoke-test".to_string()),
        causation_id: None,
    };
    let (artifact, _) = wc_service::register_artifact(&pool, &artifact_req)
        .await
        .expect("register quality_inspection artifact");

    let assign_req = AssignCompetenceRequest {
        tenant_id: tenant_id.to_string(),
        operator_id: inspector_id,
        artifact_id: artifact.id,
        awarded_at: Utc::now() - chrono::Duration::hours(1),
        expires_at: None,
        evidence_ref: Some("smoke-fixture".to_string()),
        awarded_by: Some("smoke-harness".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("smoke-test".to_string()),
        causation_id: None,
    };
    wc_service::assign_competence(&pool, &assign_req)
        .await
        .expect("assign quality_inspection competence");

    println!(
        "  Inspector {} authorized in WC DB for tenant {}",
        inspector_id, tenant_id
    );
}

#[tokio::test]
async fn smoke_quality_inspection() {
    dotenvy::dotenv().ok();

    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();

    if !wait_for_qi(&client).await {
        eprintln!("QI service not reachable at {} -- skipping", qi_url());
        return;
    }
    println!("QI service healthy at {}", qi_url());

    let Some(key) = dev_private_key() else {
        eprintln!("JWT_PRIVATE_KEY_PEM not set -- skipping");
        return;
    };

    let tenant_id = Uuid::new_v4().to_string();
    let inspector_id = Uuid::new_v4();
    let jwt = make_jwt(
        &key,
        &tenant_id,
        &["quality_inspection.mutate", "quality_inspection.read"],
    );
    let base = qi_url();

    // Gate: verify JWT is accepted (probe a GET route that requires auth)
    let probe = client
        .get(format!(
            "{base}/api/quality-inspection/plans/{}",
            Uuid::new_v4()
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("JWT probe failed");
    if probe.status().as_u16() == 401 {
        eprintln!("QI returns 401 with valid JWT -- JWT_PUBLIC_KEY not configured. Skipping.");
        return;
    }
    println!("  JWT probe ok ({})", probe.status());

    // Seed inspector authorization in WC DB before disposition tests
    authorize_inspector(&tenant_id, inspector_id).await;

    // Test data IDs (UUIDs stored by QI, not validated against external tables)
    let part_id = Uuid::new_v4();
    let wo_id = Uuid::new_v4();
    let op_instance_id = Uuid::new_v4();
    let receipt_id = Uuid::new_v4();
    let lot_id = Uuid::new_v4();

    // ========================================================================
    // Auth gate: all 15 routes require JWT (RequirePermissionsLayer -> 401)
    // ========================================================================
    println!("\n-- Auth gate (15 routes) --");
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/quality-inspection/plans"),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!("{base}/api/quality-inspection/plans/{}", Uuid::new_v4()),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!(
            "{base}/api/quality-inspection/plans/{}/activate",
            Uuid::new_v4()
        ),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/quality-inspection/inspections"),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/quality-inspection/inspections/in-process"),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!("{base}/api/quality-inspection/inspections/final"),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!(
            "{base}/api/quality-inspection/inspections/{}",
            Uuid::new_v4()
        ),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!(
            "{base}/api/quality-inspection/inspections/{}/hold",
            Uuid::new_v4()
        ),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!(
            "{base}/api/quality-inspection/inspections/{}/release",
            Uuid::new_v4()
        ),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!(
            "{base}/api/quality-inspection/inspections/{}/accept",
            Uuid::new_v4()
        ),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "POST",
        &format!(
            "{base}/api/quality-inspection/inspections/{}/reject",
            Uuid::new_v4()
        ),
        Some(json!({})),
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!(
            "{base}/api/quality-inspection/inspections/by-part-rev?part_id={}",
            part_id
        ),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!(
            "{base}/api/quality-inspection/inspections/by-receipt?receipt_id={}",
            receipt_id
        ),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!(
            "{base}/api/quality-inspection/inspections/by-wo?wo_id={}",
            wo_id
        ),
        None,
    )
    .await;
    assert_unauth(
        &client,
        "GET",
        &format!(
            "{base}/api/quality-inspection/inspections/by-lot?lot_id={}",
            lot_id
        ),
        None,
    )
    .await;

    // ========================================================================
    // Step 1: Create inspection plan
    // ========================================================================
    println!("\n-- Step 1: POST /api/quality-inspection/plans --");
    let resp = client
        .post(format!("{base}/api/quality-inspection/plans"))
        .bearer_auth(&jwt)
        .json(&json!({
            "part_id": part_id,
            "plan_name": "Smoke Test Inspection Plan",
            "revision": "A",
            "characteristics": [{
                "name": "Diameter",
                "characteristic_type": "dimensional",
                "nominal": 10.0,
                "tolerance_low": 9.9,
                "tolerance_high": 10.1,
                "uom": "mm"
            }],
            "sampling_method": "full"
        }))
        .send()
        .await
        .expect("create plan failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("create plan body");
    assert_eq!(status, StatusCode::CREATED, "create plan: {}", body);
    let plan_id = extract_uuid(&body, "id");
    println!("  created plan {} -> 201 ok", plan_id);

    // ========================================================================
    // Step 2: Get inspection plan
    // ========================================================================
    println!("\n-- Step 2: GET /api/quality-inspection/plans/{plan_id} --");
    let resp = client
        .get(format!("{base}/api/quality-inspection/plans/{plan_id}"))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get plan failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("get plan body");
    assert_eq!(status, StatusCode::OK, "get plan: {}", body);
    assert_eq!(body["status"], "draft", "plan should start as draft");
    println!("  get plan {} -> 200 ok (status=draft)", plan_id);

    // ========================================================================
    // Step 3: Activate inspection plan
    // ========================================================================
    println!("\n-- Step 3: POST /api/quality-inspection/plans/{plan_id}/activate --");
    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/plans/{plan_id}/activate"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("activate plan failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("activate plan body");
    assert_eq!(status, StatusCode::OK, "activate plan: {}", body);
    assert_eq!(body["status"], "active");
    println!("  activated plan {} -> 200 ok (status=active)", plan_id);

    // ========================================================================
    // Step 4: Create receiving inspection A (path: hold -> accept)
    // ========================================================================
    println!("\n-- Step 4: POST /api/quality-inspection/inspections (receiving A) --");
    let resp = client
        .post(format!("{base}/api/quality-inspection/inspections"))
        .bearer_auth(&jwt)
        .json(&json!({
            "plan_id": plan_id,
            "receipt_id": receipt_id,
            "lot_id": lot_id,
            "part_id": part_id,
            "part_revision": "A",
            "inspector_id": inspector_id,
            "result": "pending",
            "notes": "Smoke test receiving inspection A"
        }))
        .send()
        .await
        .expect("create receiving inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("create receiving inspection body");
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create receiving inspection: {}",
        body
    );
    let inspection_a_id = extract_uuid(&body, "id");
    assert_eq!(body["inspection_type"], "receiving");
    assert_eq!(body["disposition"], "pending");
    println!(
        "  created receiving inspection A {} -> 201 ok",
        inspection_a_id
    );

    // ========================================================================
    // Step 5: Get inspection
    // ========================================================================
    println!("\n-- Step 5: GET /api/quality-inspection/inspections/{inspection_a_id} --");
    let resp = client
        .get(format!(
            "{base}/api/quality-inspection/inspections/{inspection_a_id}"
        ))
        .bearer_auth(&jwt)
        .send()
        .await
        .expect("get inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("get inspection body");
    assert_eq!(status, StatusCode::OK, "get inspection: {}", body);
    assert_eq!(body["inspection_type"], "receiving");
    println!("  get inspection {} -> 200 ok", inspection_a_id);

    // ========================================================================
    // Step 6: Hold inspection A (pending -> held)
    // ========================================================================
    println!("\n-- Step 6: POST /api/quality-inspection/inspections/{inspection_a_id}/hold --");
    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/{inspection_a_id}/hold"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "inspector_id": inspector_id,
            "reason": "Smoke test hold — pending disposition review"
        }))
        .send()
        .await
        .expect("hold inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("hold inspection body");
    assert_eq!(status, StatusCode::OK, "hold inspection: {}", body);
    assert_eq!(body["disposition"], "held");
    println!("  hold inspection A -> 200 ok (disposition=held)");

    // ========================================================================
    // Step 7: Accept inspection A (held -> accepted)
    // ========================================================================
    println!("\n-- Step 7: POST /api/quality-inspection/inspections/{inspection_a_id}/accept --");
    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/{inspection_a_id}/accept"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "inspector_id": inspector_id,
            "reason": "All characteristics within spec"
        }))
        .send()
        .await
        .expect("accept inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("accept inspection body");
    assert_eq!(status, StatusCode::OK, "accept inspection: {}", body);
    assert_eq!(body["disposition"], "accepted");
    println!("  accept inspection A -> 200 ok (disposition=accepted)");

    // ========================================================================
    // Step 8: Create receiving inspection B (path: hold -> reject)
    // ========================================================================
    println!("\n-- Step 8: POST /api/quality-inspection/inspections (receiving B) --");
    let resp = client
        .post(format!("{base}/api/quality-inspection/inspections"))
        .bearer_auth(&jwt)
        .json(&json!({
            "plan_id": plan_id,
            "receipt_id": receipt_id,
            "lot_id": lot_id,
            "part_id": part_id,
            "part_revision": "A",
            "result": "fail",
            "notes": "Smoke test receiving inspection B"
        }))
        .send()
        .await
        .expect("create receiving inspection B failed");
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .expect("create receiving inspection B body");
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create receiving inspection B: {}",
        body
    );
    let inspection_b_id = extract_uuid(&body, "id");
    println!(
        "  created receiving inspection B {} -> 201 ok",
        inspection_b_id
    );

    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/{inspection_b_id}/hold"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"inspector_id": inspector_id, "reason": "NCR — out of spec"}))
        .send()
        .await
        .expect("hold B failed");
    assert_eq!(resp.status(), StatusCode::OK, "hold B failed");
    println!("  hold inspection B -> 200 ok");

    // ========================================================================
    // Step 9: Reject inspection B (held -> rejected)
    // ========================================================================
    println!("\n-- Step 9: POST /api/quality-inspection/inspections/{inspection_b_id}/reject --");
    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/{inspection_b_id}/reject"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "inspector_id": inspector_id,
            "reason": "Part failed dimensional check"
        }))
        .send()
        .await
        .expect("reject inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("reject inspection body");
    assert_eq!(status, StatusCode::OK, "reject inspection: {}", body);
    assert_eq!(body["disposition"], "rejected");
    println!("  reject inspection B -> 200 ok (disposition=rejected)");

    // ========================================================================
    // Step 10: Create receiving inspection C (path: hold -> release)
    // ========================================================================
    println!("\n-- Step 10: POST /api/quality-inspection/inspections (receiving C) --");
    let resp = client
        .post(format!("{base}/api/quality-inspection/inspections"))
        .bearer_auth(&jwt)
        .json(&json!({
            "part_id": part_id,
            "part_revision": "A",
            "result": "pending",
            "notes": "Smoke test receiving inspection C"
        }))
        .send()
        .await
        .expect("create receiving inspection C failed");
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .expect("create receiving inspection C body");
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create receiving inspection C: {}",
        body
    );
    let inspection_c_id = extract_uuid(&body, "id");
    println!(
        "  created receiving inspection C {} -> 201 ok",
        inspection_c_id
    );

    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/{inspection_c_id}/hold"
        ))
        .bearer_auth(&jwt)
        .json(&json!({"inspector_id": inspector_id, "reason": "Awaiting MRB review"}))
        .send()
        .await
        .expect("hold C failed");
    assert_eq!(resp.status(), StatusCode::OK, "hold C failed");
    println!("  hold inspection C -> 200 ok");

    // ========================================================================
    // Step 11: Release inspection C (held -> released)
    // ========================================================================
    println!("\n-- Step 11: POST /api/quality-inspection/inspections/{inspection_c_id}/release --");
    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/{inspection_c_id}/release"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "inspector_id": inspector_id,
            "reason": "MRB approved use-as-is"
        }))
        .send()
        .await
        .expect("release inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("release inspection body");
    assert_eq!(status, StatusCode::OK, "release inspection: {}", body);
    assert_eq!(body["disposition"], "released");
    println!("  release inspection C -> 200 ok (disposition=released)");

    // ========================================================================
    // Step 12: Create in-process inspection
    // ========================================================================
    println!("\n-- Step 12: POST /api/quality-inspection/inspections/in-process --");
    let resp = client
        .post(format!(
            "{base}/api/quality-inspection/inspections/in-process"
        ))
        .bearer_auth(&jwt)
        .json(&json!({
            "wo_id": wo_id,
            "op_instance_id": op_instance_id,
            "plan_id": plan_id,
            "part_id": part_id,
            "part_revision": "A",
            "inspector_id": inspector_id,
            "result": "pass",
            "notes": "Smoke test in-process inspection"
        }))
        .send()
        .await
        .expect("create in-process inspection failed");
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .expect("create in-process inspection body");
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create in-process inspection: {}",
        body
    );
    assert_eq!(body["inspection_type"], "in_process");
    println!(
        "  created in-process inspection {} -> 201 ok",
        extract_uuid(&body, "id")
    );

    // ========================================================================
    // Step 13: Create final inspection
    // ========================================================================
    println!("\n-- Step 13: POST /api/quality-inspection/inspections/final --");
    let resp = client
        .post(format!("{base}/api/quality-inspection/inspections/final"))
        .bearer_auth(&jwt)
        .json(&json!({
            "wo_id": wo_id,
            "plan_id": plan_id,
            "lot_id": lot_id,
            "part_id": part_id,
            "part_revision": "A",
            "inspector_id": inspector_id,
            "result": "pass",
            "notes": "Smoke test final inspection"
        }))
        .send()
        .await
        .expect("create final inspection failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("create final inspection body");
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create final inspection: {}",
        body
    );
    assert_eq!(body["inspection_type"], "final");
    println!(
        "  created final inspection {} -> 201 ok",
        extract_uuid(&body, "id")
    );

    // ========================================================================
    // Step 14: Query by-part-rev (returns seeded inspections)
    // ========================================================================
    println!("\n-- Step 14: GET /api/quality-inspection/inspections/by-part-rev --");
    let resp = client
        .get(format!(
            "{base}/api/quality-inspection/inspections/by-part-rev"
        ))
        .bearer_auth(&jwt)
        .query(&[
            ("part_id", part_id.to_string()),
            ("part_revision", "A".to_string()),
        ])
        .send()
        .await
        .expect("by-part-rev failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("by-part-rev body");
    assert_eq!(status, StatusCode::OK, "by-part-rev: {}", body);
    let rows = body.as_array().expect("by-part-rev should return array");
    assert!(
        !rows.is_empty(),
        "by-part-rev should return seeded inspections"
    );
    println!("  by-part-rev -> 200 ok ({} rows)", rows.len());

    // ========================================================================
    // Step 15: Query by-receipt (returns receiving inspections A+B)
    // ========================================================================
    println!("\n-- Step 15: GET /api/quality-inspection/inspections/by-receipt --");
    let resp = client
        .get(format!(
            "{base}/api/quality-inspection/inspections/by-receipt"
        ))
        .bearer_auth(&jwt)
        .query(&[("receipt_id", receipt_id.to_string())])
        .send()
        .await
        .expect("by-receipt failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("by-receipt body");
    assert_eq!(status, StatusCode::OK, "by-receipt: {}", body);
    let rows = body.as_array().expect("by-receipt should return array");
    assert!(
        !rows.is_empty(),
        "by-receipt should return seeded inspections"
    );
    println!("  by-receipt -> 200 ok ({} rows)", rows.len());

    // ========================================================================
    // Step 16: Query by-wo (returns in-process + final inspections)
    // ========================================================================
    println!("\n-- Step 16: GET /api/quality-inspection/inspections/by-wo --");
    let resp = client
        .get(format!("{base}/api/quality-inspection/inspections/by-wo"))
        .bearer_auth(&jwt)
        .query(&[("wo_id", wo_id.to_string())])
        .send()
        .await
        .expect("by-wo failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("by-wo body");
    assert_eq!(status, StatusCode::OK, "by-wo: {}", body);
    let rows = body.as_array().expect("by-wo should return array");
    assert!(!rows.is_empty(), "by-wo should return seeded inspections");
    println!("  by-wo -> 200 ok ({} rows)", rows.len());

    // ========================================================================
    // Step 17: Query by-lot (returns lot-tagged inspections)
    // ========================================================================
    println!("\n-- Step 17: GET /api/quality-inspection/inspections/by-lot --");
    let resp = client
        .get(format!("{base}/api/quality-inspection/inspections/by-lot"))
        .bearer_auth(&jwt)
        .query(&[("lot_id", lot_id.to_string())])
        .send()
        .await
        .expect("by-lot failed");
    let status = resp.status();
    let body: Value = resp.json().await.expect("by-lot body");
    assert_eq!(status, StatusCode::OK, "by-lot: {}", body);
    let rows = body.as_array().expect("by-lot should return array");
    assert!(!rows.is_empty(), "by-lot should return seeded inspections");
    println!("  by-lot -> 200 ok ({} rows)", rows.len());

    println!("\n=== smoke_quality_inspection PASSED (15 routes) ===");
}
