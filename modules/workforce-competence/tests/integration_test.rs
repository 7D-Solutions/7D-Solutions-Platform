//! Integration tests for workforce competence service.
//!
//! Tests run against a real PostgreSQL database.
//! Set DATABASE_URL to the workforce competence database connection string.
//!
//! Coverage:
//! 1. Register artifact: happy path
//! 2. Register artifact: idempotency replay
//! 3. Register artifact: idempotency conflict
//! 4. Assign competence: happy path (with auto-expiry from artifact)
//! 5. Assign competence: idempotency replay
//! 6. Authorization check: authorized (valid competence)
//! 7. Authorization check: unauthorized (expired)
//! 8. Authorization check: unauthorized (revoked)
//! 9. Authorization check: unauthorized (wrong tenant)
//! 10. Authorization check: unauthorized (before award date)

use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use workforce_competence_rs::domain::{
    models::{ArtifactType, AssignCompetenceRequest, AuthorizationQuery, RegisterArtifactRequest},
    service::{self, ServiceError},
};

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://wc_user:wc_pass@localhost:5458/workforce_competence_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to workforce competence test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

fn unique_code() -> String {
    format!("CODE-{}", Uuid::new_v4().to_string()[..8].to_uppercase())
}

fn register_req(tenant_id: &str) -> RegisterArtifactRequest {
    RegisterArtifactRequest {
        tenant_id: tenant_id.to_string(),
        artifact_type: ArtifactType::Certification,
        name: "IPC-A-610 Soldering Cert".to_string(),
        code: unique_code(),
        description: Some("IPC soldering certification".to_string()),
        valid_duration_days: Some(365),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
    }
}

// ============================================================================
// 1. Register artifact: happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn register_artifact_happy_path() {
    let pool = setup_db().await;
    let req = register_req("tenant-wc-1");

    let (artifact, is_replay) = service::register_artifact(&pool, &req)
        .await
        .expect("register_artifact should succeed");

    assert!(!is_replay);
    assert_eq!(artifact.tenant_id, "tenant-wc-1");
    assert_eq!(artifact.artifact_type, ArtifactType::Certification);
    assert_eq!(artifact.code, req.code);
    assert!(artifact.is_active);
    assert_eq!(artifact.valid_duration_days, Some(365));

    // Verify outbox event was created
    let outbox_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM wc_outbox WHERE event_type = $1 AND tenant_id = $2")
            .bind("workforce_competence.artifact_registered")
            .bind("tenant-wc-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(outbox_count.0 >= 1, "outbox event must exist");
}

// ============================================================================
// 2. Register artifact: idempotency replay
// ============================================================================

#[tokio::test]
#[serial]
async fn register_artifact_idempotency_replay() {
    let pool = setup_db().await;
    let req = register_req("tenant-wc-2");

    let (first, _) = service::register_artifact(&pool, &req)
        .await
        .expect("first register should succeed");

    let (replayed, is_replay) = service::register_artifact(&pool, &req)
        .await
        .expect("replay should succeed");

    assert!(is_replay);
    assert_eq!(first.id, replayed.id);
}

// ============================================================================
// 3. Register artifact: idempotency conflict
// ============================================================================

#[tokio::test]
#[serial]
async fn register_artifact_idempotency_conflict() {
    let pool = setup_db().await;
    let mut req = register_req("tenant-wc-3");

    let _ = service::register_artifact(&pool, &req)
        .await
        .expect("first register should succeed");

    // Same idempotency key, different body
    req.name = "Different Name".to_string();
    let err = service::register_artifact(&pool, &req).await.unwrap_err();

    assert!(
        matches!(err, ServiceError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Assign competence: happy path (with auto-expiry)
// ============================================================================

#[tokio::test]
#[serial]
async fn assign_competence_happy_path() {
    let pool = setup_db().await;
    let reg_req = register_req("tenant-wc-4");
    let (artifact, _) = service::register_artifact(&pool, &reg_req)
        .await
        .expect("register should succeed");

    let operator_id = Uuid::new_v4();
    let awarded_at = Utc::now();

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-4".to_string(),
        operator_id,
        artifact_id: artifact.id,
        awarded_at,
        expires_at: None, // should auto-compute from artifact's valid_duration_days
        evidence_ref: Some("cert-scan.pdf".to_string()),
        awarded_by: Some("QA Manager".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };

    let (assignment, is_replay) = service::assign_competence(&pool, &assign_req)
        .await
        .expect("assign should succeed");

    assert!(!is_replay);
    assert_eq!(assignment.operator_id, operator_id);
    assert_eq!(assignment.artifact_id, artifact.id);
    assert!(!assignment.is_revoked);

    // Auto-expiry should be awarded_at + 365 days
    let expected_expiry = awarded_at + Duration::days(365);
    let actual_expiry = assignment.expires_at.expect("expires_at should be set");
    let diff = (expected_expiry - actual_expiry).num_seconds().abs();
    assert!(diff < 2, "expiry should be ~365 days from awarded_at");
}

// ============================================================================
// 5. Assign competence: idempotency replay
// ============================================================================

#[tokio::test]
#[serial]
async fn assign_competence_idempotency_replay() {
    let pool = setup_db().await;
    let reg_req = register_req("tenant-wc-5");
    let (artifact, _) = service::register_artifact(&pool, &reg_req).await.unwrap();

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-5".to_string(),
        operator_id: Uuid::new_v4(),
        artifact_id: artifact.id,
        awarded_at: Utc::now(),
        expires_at: None,
        evidence_ref: None,
        awarded_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };

    let (first, _) = service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();
    let (replayed, is_replay) = service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();

    assert!(is_replay);
    assert_eq!(first.id, replayed.id);
}

// ============================================================================
// 6. Authorization check: authorized (valid competence)
// ============================================================================

#[tokio::test]
#[serial]
async fn authorization_check_authorized() {
    let pool = setup_db().await;
    let reg_req = register_req("tenant-wc-6");
    let code = reg_req.code.clone();
    let (artifact, _) = service::register_artifact(&pool, &reg_req).await.unwrap();

    let operator_id = Uuid::new_v4();
    let awarded_at = Utc::now() - Duration::days(30);

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-6".to_string(),
        operator_id,
        artifact_id: artifact.id,
        awarded_at,
        expires_at: None, // 365 days from awarded_at
        evidence_ref: None,
        awarded_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();

    // Check authorization at current time (within validity)
    let query = AuthorizationQuery {
        tenant_id: "tenant-wc-6".to_string(),
        operator_id,
        artifact_code: code,
        at_time: Utc::now(),
    };
    let result = service::check_authorization(&pool, &query).await.unwrap();

    assert!(result.authorized);
    assert!(result.assignment_id.is_some());
}

// ============================================================================
// 7. Authorization check: unauthorized (expired)
// ============================================================================

#[tokio::test]
#[serial]
async fn authorization_check_expired() {
    let pool = setup_db().await;

    let mut reg_req = register_req("tenant-wc-7");
    reg_req.valid_duration_days = Some(30); // expires after 30 days
    let code = reg_req.code.clone();
    let (artifact, _) = service::register_artifact(&pool, &reg_req).await.unwrap();

    let operator_id = Uuid::new_v4();
    let awarded_at = Utc::now() - Duration::days(60); // awarded 60 days ago

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-7".to_string(),
        operator_id,
        artifact_id: artifact.id,
        awarded_at,
        expires_at: None, // auto: awarded_at + 30 days = 30 days ago (expired)
        evidence_ref: None,
        awarded_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();

    // Check authorization now — should be expired
    let query = AuthorizationQuery {
        tenant_id: "tenant-wc-7".to_string(),
        operator_id,
        artifact_code: code,
        at_time: Utc::now(),
    };
    let result = service::check_authorization(&pool, &query).await.unwrap();

    assert!(
        !result.authorized,
        "expired competence should not be authorized"
    );
}

// ============================================================================
// 8. Authorization check: unauthorized (revoked)
// ============================================================================

#[tokio::test]
#[serial]
async fn authorization_check_revoked() {
    let pool = setup_db().await;
    let reg_req = register_req("tenant-wc-8");
    let code = reg_req.code.clone();
    let (artifact, _) = service::register_artifact(&pool, &reg_req).await.unwrap();

    let operator_id = Uuid::new_v4();

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-8".to_string(),
        operator_id,
        artifact_id: artifact.id,
        awarded_at: Utc::now() - Duration::days(10),
        expires_at: None,
        evidence_ref: None,
        awarded_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let (assignment, _) = service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();

    // Manually revoke the assignment
    sqlx::query(
        "UPDATE wc_operator_competences SET is_revoked = true, revoked_at = NOW() WHERE id = $1",
    )
    .bind(assignment.id)
    .execute(&pool)
    .await
    .unwrap();

    let query = AuthorizationQuery {
        tenant_id: "tenant-wc-8".to_string(),
        operator_id,
        artifact_code: code,
        at_time: Utc::now(),
    };
    let result = service::check_authorization(&pool, &query).await.unwrap();

    assert!(
        !result.authorized,
        "revoked competence should not be authorized"
    );
}

// ============================================================================
// 9. Authorization check: unauthorized (wrong tenant)
// ============================================================================

#[tokio::test]
#[serial]
async fn authorization_check_wrong_tenant() {
    let pool = setup_db().await;
    let reg_req = register_req("tenant-wc-9");
    let code = reg_req.code.clone();
    let (artifact, _) = service::register_artifact(&pool, &reg_req).await.unwrap();

    let operator_id = Uuid::new_v4();

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-9".to_string(),
        operator_id,
        artifact_id: artifact.id,
        awarded_at: Utc::now() - Duration::days(10),
        expires_at: None,
        evidence_ref: None,
        awarded_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();

    // Query with different tenant
    let query = AuthorizationQuery {
        tenant_id: "tenant-WRONG".to_string(),
        operator_id,
        artifact_code: code,
        at_time: Utc::now(),
    };
    let result = service::check_authorization(&pool, &query).await.unwrap();

    assert!(!result.authorized, "cross-tenant should not be authorized");
}

// ============================================================================
// 10. Authorization check: unauthorized (before award date)
// ============================================================================

#[tokio::test]
#[serial]
async fn authorization_check_before_award() {
    let pool = setup_db().await;
    let reg_req = register_req("tenant-wc-10");
    let code = reg_req.code.clone();
    let (artifact, _) = service::register_artifact(&pool, &reg_req).await.unwrap();

    let operator_id = Uuid::new_v4();
    let awarded_at = Utc::now() + Duration::days(30); // awarded in the future

    let assign_req = AssignCompetenceRequest {
        tenant_id: "tenant-wc-10".to_string(),
        operator_id,
        artifact_id: artifact.id,
        awarded_at,
        expires_at: None,
        evidence_ref: None,
        awarded_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    service::assign_competence(&pool, &assign_req)
        .await
        .unwrap();

    // Query now — before the award date
    let query = AuthorizationQuery {
        tenant_id: "tenant-wc-10".to_string(),
        operator_id,
        artifact_code: code,
        at_time: Utc::now(),
    };
    let result = service::check_authorization(&pool, &query).await.unwrap();

    assert!(
        !result.authorized,
        "should not be authorized before award date"
    );
}
