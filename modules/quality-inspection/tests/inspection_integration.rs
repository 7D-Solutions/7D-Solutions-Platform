use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use platform_sdk::PlatformClient;
use quality_inspection_rs::domain::models::*;
use quality_inspection_rs::domain::service;
use serde::Serialize;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    jti: String,
    tenant_id: String,
    roles: Vec<String>,
    perms: Vec<String>,
    actor_type: String,
    ver: String,
}

fn sign_jwt(tenant_id: &str, perms: &[&str]) -> String {
    dotenvy::from_filename_override(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env"),
    )
    .ok();
    let pem = std::env::var("JWT_PRIVATE_KEY_PEM")
        .expect("JWT_PRIVATE_KEY_PEM must be set in .env");
    let encoding =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("failed to parse JWT_PRIVATE_KEY_PEM");
    let now = Utc::now();
    let claims = TestClaims {
        sub: Uuid::new_v4().to_string(),
        iss: "auth-rs".to_string(),
        aud: "7d-platform".to_string(),
        iat: now.timestamp(),
        exp: (now + chrono::Duration::minutes(15)).timestamp(),
        jti: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        roles: vec!["operator".to_string()],
        perms: perms.iter().map(|s| s.to_string()).collect(),
        actor_type: "service".to_string(),
        ver: "1".to_string(),
    };
    jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding)
        .expect("failed to sign JWT")
}

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://quality_inspection_user:quality_inspection_pass@localhost:5459/quality_inspection_db?sslmode=require".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to quality-inspection test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run quality-inspection migrations");

    pool
}

/// Register a "quality_inspection" artifact and assign competence to the given operator
/// via the running workforce-competence HTTP service.
async fn authorize_inspector(tenant_id: &str, inspector_id: Uuid) {
    let url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    let token = sign_jwt(tenant_id, &["service.internal"]);
    let client = reqwest::Client::new();

    // Register the "quality_inspection" artifact
    let artifact_resp = client
        .post(format!("{url}/api/workforce-competence/artifacts"))
        .bearer_auth(&token)
        .header("x-tenant-id", tenant_id)
        .header("x-correlation-id", Uuid::new_v4().to_string())
        .header("x-actor-id", Uuid::nil().to_string())
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "artifact_type": "qualification",
            "name": "Quality Inspection Disposition Authority",
            "code": "quality_inspection",
            "description": "Authorization to perform inspection dispositions",
            "valid_duration_days": null,
            "idempotency_key": Uuid::new_v4().to_string(),
            "correlation_id": "test",
            "causation_id": null
        }))
        .send()
        .await
        .expect("WC artifact registration HTTP call failed");

    let status = artifact_resp.status();
    assert!(
        status.is_success(),
        "register_artifact failed with {status}: {}",
        artifact_resp.text().await.unwrap_or_default()
    );

    let artifact: serde_json::Value = artifact_resp
        .json()
        .await
        .expect("parse artifact response");
    let artifact_id = artifact["id"].as_str().expect("artifact.id must be a string");

    // Assign competence to the inspector
    let assign_resp = client
        .post(format!("{url}/api/workforce-competence/assignments"))
        .bearer_auth(&token)
        .header("x-tenant-id", tenant_id)
        .header("x-correlation-id", Uuid::new_v4().to_string())
        .header("x-actor-id", Uuid::nil().to_string())
        .json(&serde_json::json!({
            "tenant_id": tenant_id,
            "operator_id": inspector_id,
            "artifact_id": artifact_id,
            "awarded_at": (Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
            "expires_at": null,
            "evidence_ref": "test-fixture",
            "awarded_by": "test-harness",
            "idempotency_key": Uuid::new_v4().to_string(),
            "correlation_id": "test",
            "causation_id": null
        }))
        .send()
        .await
        .expect("WC assign competence HTTP call failed");

    let status = assign_resp.status();
    assert!(
        status.is_success(),
        "assign_competence failed with {status}: {}",
        assign_resp.text().await.unwrap_or_default()
    );
}

fn wc_client(tenant_id: &str) -> PlatformClient {
    let url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    let token = sign_jwt(tenant_id, &["workforce_competence.read"]);
    PlatformClient::new(url).with_bearer_token(token)
}

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
}

// ============================================================================
// Create inspection plan with characteristics
// ============================================================================

#[tokio::test]
#[serial]
async fn create_inspection_plan_with_characteristics() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let part_id = Uuid::new_v4();
    let corr = Uuid::new_v4().to_string();

    let plan = service::create_inspection_plan(
        &pool,
        &tenant,
        &CreateInspectionPlanRequest {
            part_id,
            plan_name: "Receiving Plan for Fastener".to_string(),
            revision: Some("B".to_string()),
            characteristics: vec![
                Characteristic {
                    name: "Diameter".to_string(),
                    characteristic_type: "dimensional".to_string(),
                    key_characteristic: false,
                    nominal: Some(10.0),
                    tolerance_low: Some(9.95),
                    tolerance_high: Some(10.05),
                    uom: Some("mm".to_string()),
                },
                Characteristic {
                    name: "Surface finish".to_string(),
                    characteristic_type: "visual".to_string(),
                    key_characteristic: false,
                    nominal: None,
                    tolerance_low: None,
                    tolerance_high: None,
                    uom: None,
                },
            ],
            sampling_method: Some("aql".to_string()),
            sample_size: Some(13),
        },
        &corr,
        None,
    )
    .await
    .expect("create_inspection_plan");

    assert_eq!(plan.tenant_id, tenant);
    assert_eq!(plan.part_id, part_id);
    assert_eq!(plan.plan_name, "Receiving Plan for Fastener");
    assert_eq!(plan.revision, "B");
    assert_eq!(plan.status, "draft");
    assert_eq!(plan.sampling_method, "aql");
    assert_eq!(plan.sample_size, Some(13));

    let chars: Vec<Characteristic> =
        serde_json::from_value(plan.characteristics).expect("parse characteristics");
    assert_eq!(chars.len(), 2);
    assert_eq!(chars[0].name, "Diameter");
    assert!((chars[0].nominal.unwrap() - 10.0).abs() < f64::EPSILON);
}

// ============================================================================
// Activate plan
// ============================================================================

#[tokio::test]
#[serial]
async fn activate_inspection_plan() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let plan = service::create_inspection_plan(
        &pool,
        &tenant,
        &CreateInspectionPlanRequest {
            part_id: Uuid::new_v4(),
            plan_name: "Plan A".to_string(),
            revision: None,
            characteristics: vec![],
            sampling_method: None,
            sample_size: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    assert_eq!(plan.status, "draft");

    let activated = service::activate_plan(&pool, &tenant, plan.id)
        .await
        .expect("activate_plan");
    assert_eq!(activated.status, "active");
}

// ============================================================================
// Create receiving inspection + query by receipt
// ============================================================================

#[tokio::test]
#[serial]
async fn create_receiving_inspection_and_query_by_receipt() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let receipt_id = Uuid::new_v4();
    let part_id = Uuid::new_v4();

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(receipt_id),
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("C".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: Some("All dimensions within spec".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("create_receiving_inspection");

    assert_eq!(inspection.tenant_id, tenant);
    assert_eq!(inspection.inspection_type, "receiving");
    assert_eq!(inspection.result, "pass");
    assert_eq!(inspection.receipt_id, Some(receipt_id));
    assert_eq!(inspection.part_id, Some(part_id));
    assert_eq!(inspection.part_revision.as_deref(), Some("C"));
    assert!(inspection.inspected_at.is_some());

    // Query by receipt
    let by_receipt = service::list_inspections_by_receipt(&pool, &tenant, receipt_id)
        .await
        .expect("list_by_receipt");
    assert_eq!(by_receipt.len(), 1);
    assert_eq!(by_receipt[0].id, inspection.id);
}

// ============================================================================
// Query by part revision
// ============================================================================

#[tokio::test]
#[serial]
async fn query_inspections_by_part_revision() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let part_id = Uuid::new_v4();

    // Create two inspections for same part, different revisions
    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pass".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(part_id),
            part_revision: Some("B".to_string()),
            inspector_id: None,
            result: Some("fail".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Query all for part
    let all = service::list_inspections_by_part_rev(&pool, &tenant, part_id, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // Query specific revision
    let rev_a = service::list_inspections_by_part_rev(&pool, &tenant, part_id, Some("A"))
        .await
        .unwrap();
    assert_eq!(rev_a.len(), 1);
    assert_eq!(rev_a[0].result, "pass");

    let rev_b = service::list_inspections_by_part_rev(&pool, &tenant, part_id, Some("B"))
        .await
        .unwrap();
    assert_eq!(rev_b.len(), 1);
    assert_eq!(rev_b[0].result, "fail");
}

// ============================================================================
// Events emitted to outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn events_emitted_to_outbox() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create plan → 1 event
    let plan = service::create_inspection_plan(
        &pool,
        &tenant,
        &CreateInspectionPlanRequest {
            part_id: Uuid::new_v4(),
            plan_name: "Outbox test plan".to_string(),
            revision: None,
            characteristics: vec![],
            sampling_method: None,
            sample_size: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Create inspection → 1 event
    service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: Some(plan.id),
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Verify outbox
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = event_types.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "quality_inspection.plan_created",
            "quality_inspection.inspection_recorded"
        ]
    );

    // Verify envelope metadata
    let payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.plan_created'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payload.0["source_module"], "quality-inspection");
    assert_eq!(payload.0["replay_safe"], true);
    assert!(payload.0["event_id"].is_string());
    assert!(payload.0["correlation_id"].is_string());
    assert_eq!(payload.0["mutation_class"], "DATA_MUTATION");
}

// ============================================================================
// Disposition state machine: hold -> accept (with authorized inspector)
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_hold_then_accept() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    authorize_inspector(&tenant, inspector).await;

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    assert_eq!(inspection.disposition, "pending");

    // Hold
    let held = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Awaiting further review"),
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(held.disposition, "held");

    // Accept from held
    let accepted = service::accept_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("All criteria met"),
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(accepted.disposition, "accepted");
}

// ============================================================================
// Disposition state machine: hold -> reject
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_hold_then_reject() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    authorize_inspector(&tenant, inspector).await;

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let held = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(held.disposition, "held");

    let rejected = service::reject_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Out of tolerance"),
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(rejected.disposition, "rejected");
}

// ============================================================================
// Disposition state machine: hold -> release
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_hold_then_release() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    authorize_inspector(&tenant, inspector).await;

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let held = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(held.disposition, "held");

    let released = service::release_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Released for production use"),
        &corr,
        None,
    )
    .await
    .unwrap();
    assert_eq!(released.disposition, "released");
}

// ============================================================================
// Disposition: reject illegal transitions (with authorized inspector)
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_rejects_illegal_transitions() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    authorize_inspector(&tenant, inspector).await;

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Cannot accept directly from pending (must hold first)
    let err = service::accept_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("Cannot transition"));

    // Cannot reject directly from pending
    let err = service::reject_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());

    // Cannot release directly from pending
    let err = service::release_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());

    // Hold it
    service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await
    .unwrap();

    // Cannot hold again
    let err = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());

    // Accept it
    service::accept_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await
    .unwrap();

    // Cannot transition from accepted
    let err = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
    let err = service::release_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
}

// ============================================================================
// Disposition events emitted to outbox
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_events_emitted() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    authorize_inspector(&tenant, inspector).await;

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: Some(inspector),
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Hold then release
    service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Quality hold"),
        &corr,
        None,
    )
    .await
    .unwrap();

    service::release_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Cleared"),
        &corr,
        None,
    )
    .await
    .unwrap();

    // Verify outbox: inspection_recorded + held + released
    let event_types: Vec<(String,)> = sqlx::query_as(
        "SELECT event_type FROM quality_inspection_outbox WHERE tenant_id = $1 ORDER BY created_at",
    )
    .bind(&tenant)
    .fetch_all(&pool)
    .await
    .unwrap();

    let types: Vec<&str> = event_types.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "quality_inspection.inspection_recorded",
            "quality_inspection.held",
            "quality_inspection.released",
        ]
    );

    // Verify the release event payload
    let payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM quality_inspection_outbox WHERE tenant_id = $1 AND event_type = 'quality_inspection.released'",
    )
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payload.0["source_module"], "quality-inspection");
    assert_eq!(payload.0["payload"]["previous_disposition"], "held");
    assert_eq!(payload.0["payload"]["new_disposition"], "released");
    assert_eq!(payload.0["payload"]["reason"], "Cleared");
    assert_eq!(payload.0["replay_safe"], true);
}

// ============================================================================
// Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_isolation() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let plan = service::create_inspection_plan(
        &pool,
        &tenant_a,
        &CreateInspectionPlanRequest {
            part_id: Uuid::new_v4(),
            plan_name: "Secret plan".to_string(),
            revision: None,
            characteristics: vec![],
            sampling_method: None,
            sample_size: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // Tenant B cannot see it
    let result = service::get_inspection_plan(&pool, &tenant_b, plan.id).await;
    assert!(result.is_err());

    // Tenant B cannot activate it
    let result = service::activate_plan(&pool, &tenant_b, plan.id).await;
    assert!(result.is_err());
}

// ============================================================================
// Disposition requires inspector_id (None → validation error)
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_requires_inspector_id() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let err = service::hold_inspection(
        &pool, &wc, &tenant, inspection.id, None, None, &corr, None,
    )
    .await;
    assert!(err.is_err());
    assert!(
        err.unwrap_err()
            .to_string()
            .contains("inspector_id is required")
    );
}

// ============================================================================
// Unauthorized inspector → 403
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_rejects_unauthorized_inspector() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let unauthorized_inspector = Uuid::new_v4(); // not authorized in WC

    let inspection = service::create_receiving_inspection(
        &pool,
        &tenant,
        &CreateReceivingInspectionRequest {
            plan_id: None,
            receipt_id: Some(Uuid::new_v4()),
            lot_id: None,
            part_id: Some(Uuid::new_v4()),
            part_revision: Some("A".to_string()),
            inspector_id: None,
            result: Some("pending".to_string()),
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let err = service::hold_inspection(
        &pool,
        &wc,
        &tenant,
        inspection.id,
        Some(unauthorized_inspector),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
    assert!(
        err.unwrap_err()
            .to_string()
            .contains("not authorized for quality inspection")
    );
}
