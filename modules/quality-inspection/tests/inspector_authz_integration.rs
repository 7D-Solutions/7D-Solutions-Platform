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
    let pem =
        std::env::var("JWT_PRIVATE_KEY_PEM").expect("JWT_PRIVATE_KEY_PEM must be set in .env");
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

async fn setup_qi_db() -> sqlx::PgPool {
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

fn wc_client(tenant_id: &str) -> PlatformClient {
    let url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    let token = sign_jwt(tenant_id, &["workforce_competence.read"]);
    PlatformClient::new(url).with_bearer_token(token)
}

fn unique_tenant() -> String {
    Uuid::new_v4().to_string()
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

    let artifact: serde_json::Value = artifact_resp.json().await.expect("parse artifact response");
    let artifact_id = artifact["id"]
        .as_str()
        .expect("artifact.id must be a string");

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

fn create_pending_inspection_req() -> CreateReceivingInspectionRequest {
    CreateReceivingInspectionRequest {
        plan_id: None,
        receipt_id: Some(Uuid::new_v4()),
        lot_id: None,
        part_id: Some(Uuid::new_v4()),
        part_revision: Some("A".to_string()),
        inspector_id: None,
        result: Some("pending".to_string()),
        notes: None,
    }
}

// ============================================================================
// Full authorize → dispose flow: unauthorized then authorized
// ============================================================================

#[tokio::test]
#[serial]
async fn authorize_then_dispose_flow() {
    let qi_pool = setup_qi_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let inspector = Uuid::new_v4();

    // Create a pending inspection
    let inspection = service::create_receiving_inspection(
        &qi_pool,
        &tenant,
        &create_pending_inspection_req(),
        &corr,
        None,
    )
    .await
    .expect("create inspection");

    // Attempt hold WITHOUT authorization → should fail
    let err = service::hold_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Trying to hold"),
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
    let err_msg = err.unwrap_err().to_string();
    assert!(
        err_msg.contains("not authorized"),
        "Expected 'not authorized' but got: {}",
        err_msg
    );

    // Grant authority via WC HTTP API
    authorize_inspector(&tenant, inspector).await;

    // Now hold should succeed
    let held = service::hold_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Material under review"),
        &corr,
        None,
    )
    .await
    .expect("hold should succeed after authorization");
    assert_eq!(held.disposition, "held");

    // Accept should also succeed
    let accepted = service::accept_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(inspector),
        Some("Material passes requirements"),
        &corr,
        None,
    )
    .await
    .expect("accept should succeed");
    assert_eq!(accepted.disposition, "accepted");
}

// ============================================================================
// Inspector None → validation error (not unauthorized)
// ============================================================================

#[tokio::test]
#[serial]
async fn disposition_without_inspector_id_returns_validation_error() {
    let qi_pool = setup_qi_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();

    let inspection = service::create_receiving_inspection(
        &qi_pool,
        &tenant,
        &create_pending_inspection_req(),
        &corr,
        None,
    )
    .await
    .unwrap();

    let err = service::hold_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        None,
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
    assert!(err
        .unwrap_err()
        .to_string()
        .contains("inspector_id is required"));
}

// ============================================================================
// Creating inspection without inspector is fine (no auth check at creation)
// ============================================================================

#[tokio::test]
#[serial]
async fn create_inspection_without_inspector_succeeds() {
    let qi_pool = setup_qi_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let inspection = service::create_receiving_inspection(
        &qi_pool,
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
    .await;

    assert!(inspection.is_ok());
    assert!(inspection.unwrap().inspector_id.is_none());
}

// ============================================================================
// All 4 disposition actions enforce authorization
// ============================================================================

#[tokio::test]
#[serial]
async fn all_disposition_actions_enforce_authorization() {
    let qi_pool = setup_qi_db().await;
    let tenant = unique_tenant();
    let wc = wc_client(&tenant);
    let corr = Uuid::new_v4().to_string();
    let unauthorized = Uuid::new_v4();

    let inspection = service::create_receiving_inspection(
        &qi_pool,
        &tenant,
        &create_pending_inspection_req(),
        &corr,
        None,
    )
    .await
    .unwrap();

    // Hold: unauthorized
    let err = service::hold_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(unauthorized),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());

    // Accept: unauthorized
    let err = service::accept_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(unauthorized),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());

    // Reject: unauthorized
    let err = service::reject_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(unauthorized),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());

    // Release: unauthorized
    let err = service::release_inspection(
        &qi_pool,
        &wc,
        &tenant,
        inspection.id,
        Some(unauthorized),
        None,
        &corr,
        None,
    )
    .await;
    assert!(err.is_err());
}
