use chrono::Utc;
use platform_sdk::PlatformClient;
use quality_inspection_rs::domain::models::*;
use quality_inspection_rs::domain::service;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use workforce_competence_rs::domain::{
    models::{ArtifactType, AssignCompetenceRequest, RegisterArtifactRequest},
    service as wc_service,
};

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

async fn setup_wc_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("WORKFORCE_COMPETENCE_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://wc_user:wc_pass@localhost:5458/workforce_competence_db?sslmode=require"
            .to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to workforce-competence test DB");

    sqlx::migrate!("../workforce-competence/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run workforce-competence migrations");

    pool
}

fn wc_client() -> PlatformClient {
    let url = std::env::var("WORKFORCE_COMPETENCE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8121".to_string());
    PlatformClient::new(url)
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Register a "quality_inspection" artifact and assign competence to the given operator.
async fn authorize_inspector(wc_pool: &sqlx::PgPool, tenant_id: &str, inspector_id: Uuid) {
    let artifact_req = RegisterArtifactRequest {
        tenant_id: tenant_id.to_string(),
        artifact_type: ArtifactType::Qualification,
        name: "Quality Inspection Disposition Authority".to_string(),
        code: "quality_inspection".to_string(),
        description: Some("Authorization to perform inspection dispositions".to_string()),
        valid_duration_days: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("test".to_string()),
        causation_id: None,
    };

    let (artifact, _) = wc_service::register_artifact(wc_pool, &artifact_req)
        .await
        .expect("register quality_inspection artifact");

    let assign_req = AssignCompetenceRequest {
        tenant_id: tenant_id.to_string(),
        operator_id: inspector_id,
        artifact_id: artifact.id,
        awarded_at: Utc::now() - chrono::Duration::hours(1),
        expires_at: None,
        evidence_ref: Some("test-fixture".to_string()),
        awarded_by: Some("test-harness".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("test".to_string()),
        causation_id: None,
    };

    wc_service::assign_competence(wc_pool, &assign_req)
        .await
        .expect("assign quality_inspection competence");
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
    let wc_pool = setup_wc_db().await;
    let wc = wc_client();
    let tenant = unique_tenant();
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

    // Grant authority via WC
    authorize_inspector(&wc_pool, &tenant, inspector).await;

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
    let _wc_pool = setup_wc_db().await;
    let wc = wc_client();
    let tenant = unique_tenant();
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
        &qi_pool, &wc, &tenant, inspection.id, None, None, &corr, None,
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
    let _wc_pool = setup_wc_db().await;
    let wc = wc_client();
    let tenant = unique_tenant();
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
