//! Integration tests for acceptance authority register.
//!
//! Tests run against a real PostgreSQL database on port 5458.
//!
//! Coverage:
//! 1. Grant authority E2E
//! 2. Time-window test (before/during/after)
//! 3. Revocation test
//! 4. Tenant isolation test
//! 5. Idempotency test
//! 6. Outbox event test
//! 7. Authorization query test (various combinations)

use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;
use workforce_competence_rs::domain::acceptance_authority::{
    self, AcceptanceAuthorityQuery, GrantAuthorityRequest, RevokeAuthorityRequest,
};
use workforce_competence_rs::domain::service::ServiceError;

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

fn grant_req(tenant: &str, scope: &str) -> GrantAuthorityRequest {
    GrantAuthorityRequest {
        tenant_id: tenant.to_string(),
        operator_id: Uuid::new_v4(),
        capability_scope: scope.to_string(),
        constraints: None,
        effective_from: Utc::now() - Duration::days(1),
        effective_until: Some(Utc::now() + Duration::days(365)),
        granted_by: Some("QA Director".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: Some("corr-test".to_string()),
        causation_id: None,
    }
}

// ============================================================================
// 1. Grant authority E2E
// ============================================================================

#[tokio::test]
#[serial]
async fn grant_authority_e2e() {
    let pool = setup_db().await;
    let req = grant_req("tenant-aa-1", "incoming-inspection-pcb");

    let (authority, is_replay) = acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .expect("grant should succeed");

    assert!(!is_replay);
    assert_eq!(authority.tenant_id, "tenant-aa-1");
    assert_eq!(authority.capability_scope, "incoming-inspection-pcb");
    assert_eq!(authority.operator_id, req.operator_id);
    assert!(!authority.is_revoked);
    assert!(authority.effective_until.is_some());
    assert_eq!(authority.granted_by.as_deref(), Some("QA Director"));

    // Verify it's queryable
    let check = AcceptanceAuthorityQuery {
        tenant_id: "tenant-aa-1".to_string(),
        operator_id: req.operator_id,
        capability_scope: "incoming-inspection-pcb".to_string(),
        at_time: Utc::now(),
    };
    let result = acceptance_authority::check_acceptance_authority(&pool, &check)
        .await
        .unwrap();
    assert!(result.allowed);
    assert_eq!(result.authority_id, Some(authority.id));
}

// ============================================================================
// 2. Time-window test
// ============================================================================

#[tokio::test]
#[serial]
async fn time_window_test() {
    let pool = setup_db().await;
    let operator_id = Uuid::new_v4();
    let scope = format!("scope-tw-{}", &Uuid::new_v4().to_string()[..8]);

    // Grant authority effective from day 10 to day 40 from now
    let effective_from = Utc::now() + Duration::days(10);
    let effective_until = Utc::now() + Duration::days(40);

    let req = GrantAuthorityRequest {
        tenant_id: "tenant-aa-2".to_string(),
        operator_id,
        capability_scope: scope.clone(),
        constraints: None,
        effective_from,
        effective_until: Some(effective_until),
        granted_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .unwrap();

    // Before effective window (now) → denied
    let before = AcceptanceAuthorityQuery {
        tenant_id: "tenant-aa-2".to_string(),
        operator_id,
        capability_scope: scope.clone(),
        at_time: Utc::now(),
    };
    let result = acceptance_authority::check_acceptance_authority(&pool, &before)
        .await
        .unwrap();
    assert!(!result.allowed, "should be denied before effective window");
    assert_eq!(result.denial_reason.as_deref(), Some("not_yet_effective"));

    // During effective window (day 20) → allowed
    let during = AcceptanceAuthorityQuery {
        tenant_id: "tenant-aa-2".to_string(),
        operator_id,
        capability_scope: scope.clone(),
        at_time: Utc::now() + Duration::days(20),
    };
    let result = acceptance_authority::check_acceptance_authority(&pool, &during)
        .await
        .unwrap();
    assert!(result.allowed, "should be allowed during effective window");

    // After effective window (day 50) → denied
    let after = AcceptanceAuthorityQuery {
        tenant_id: "tenant-aa-2".to_string(),
        operator_id,
        capability_scope: scope.clone(),
        at_time: Utc::now() + Duration::days(50),
    };
    let result = acceptance_authority::check_acceptance_authority(&pool, &after)
        .await
        .unwrap();
    assert!(!result.allowed, "should be denied after effective window");
    assert_eq!(result.denial_reason.as_deref(), Some("authority_expired"));
}

// ============================================================================
// 3. Revocation test
// ============================================================================

#[tokio::test]
#[serial]
async fn revocation_test() {
    let pool = setup_db().await;
    let req = grant_req("tenant-aa-3", "final-inspection");

    let (authority, _) = acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .unwrap();

    // Verify authorized before revocation
    let check = AcceptanceAuthorityQuery {
        tenant_id: "tenant-aa-3".to_string(),
        operator_id: req.operator_id,
        capability_scope: "final-inspection".to_string(),
        at_time: Utc::now(),
    };
    let result = acceptance_authority::check_acceptance_authority(&pool, &check)
        .await
        .unwrap();
    assert!(result.allowed, "should be allowed before revocation");

    // Revoke
    let revoke_req = RevokeAuthorityRequest {
        tenant_id: "tenant-aa-3".to_string(),
        authority_id: authority.id,
        revocation_reason: "Certification lapsed".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    let (revoked, _) = acceptance_authority::revoke_acceptance_authority(&pool, &revoke_req)
        .await
        .unwrap();
    assert!(revoked.is_revoked);
    assert!(revoked.revoked_at.is_some());
    assert_eq!(
        revoked.revocation_reason.as_deref(),
        Some("Certification lapsed")
    );

    // Verify denied after revocation
    let result = acceptance_authority::check_acceptance_authority(&pool, &check)
        .await
        .unwrap();
    assert!(!result.allowed, "should be denied after revocation");
    assert_eq!(result.denial_reason.as_deref(), Some("authority_revoked"));
}

// ============================================================================
// 4. Tenant isolation test
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_isolation_test() {
    let pool = setup_db().await;
    let operator_id = Uuid::new_v4();
    let scope = format!("scope-ti-{}", &Uuid::new_v4().to_string()[..8]);

    // Grant under tenant_A
    let req = GrantAuthorityRequest {
        tenant_id: "tenant-aa-4a".to_string(),
        operator_id,
        capability_scope: scope.clone(),
        constraints: None,
        effective_from: Utc::now() - Duration::days(1),
        effective_until: Some(Utc::now() + Duration::days(365)),
        granted_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .unwrap();

    // Query as tenant_B → should get zero results
    let check = AcceptanceAuthorityQuery {
        tenant_id: "tenant-aa-4b".to_string(),
        operator_id,
        capability_scope: scope.clone(),
        at_time: Utc::now(),
    };
    let result = acceptance_authority::check_acceptance_authority(&pool, &check)
        .await
        .unwrap();
    assert!(!result.allowed, "cross-tenant query must return denied");
    assert_eq!(result.denial_reason.as_deref(), Some("no_authority_found"));
}

// ============================================================================
// 5. Idempotency test
// ============================================================================

#[tokio::test]
#[serial]
async fn idempotency_test() {
    let pool = setup_db().await;
    let req = grant_req("tenant-aa-5", "source-inspection");

    let (first, first_replay) = acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .unwrap();
    assert!(!first_replay);

    let (second, second_replay) = acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .unwrap();
    assert!(
        second_replay,
        "second call with same key should be a replay"
    );
    assert_eq!(first.id, second.id, "replayed result should have same ID");

    // Verify no duplicate rows
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM wc_acceptance_authorities
         WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind("tenant-aa-5")
    .bind(&req.idempotency_key)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 1, "must be exactly one row, no duplicates");
}

// ============================================================================
// 6. Outbox event test
// ============================================================================

#[tokio::test]
#[serial]
async fn outbox_event_test() {
    let pool = setup_db().await;
    let req = grant_req("tenant-aa-6", "ndt-inspection");

    let (authority, _) = acceptance_authority::grant_acceptance_authority(&pool, &req)
        .await
        .unwrap();

    // Verify grant event in outbox
    let grant_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM wc_outbox
         WHERE event_type = 'workforce_competence.acceptance_authority_granted'
           AND tenant_id = $1 AND aggregate_id = $2",
    )
    .bind("tenant-aa-6")
    .bind(authority.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(grant_count.0 >= 1, "grant outbox event must exist");

    // Verify event payload contains correct tenant_id
    let payload: (serde_json::Value,) = sqlx::query_as(
        "SELECT payload FROM wc_outbox
         WHERE event_type = 'workforce_competence.acceptance_authority_granted'
           AND aggregate_id = $1
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(authority.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    let tenant_in_payload = payload.0["payload"]["tenant_id"].as_str();
    assert_eq!(tenant_in_payload, Some("tenant-aa-6"));

    // Revoke and verify revocation event
    let revoke_req = RevokeAuthorityRequest {
        tenant_id: "tenant-aa-6".to_string(),
        authority_id: authority.id,
        revocation_reason: "Policy change".to_string(),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    acceptance_authority::revoke_acceptance_authority(&pool, &revoke_req)
        .await
        .unwrap();

    let revoke_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM wc_outbox
         WHERE event_type = 'workforce_competence.acceptance_authority_revoked'
           AND tenant_id = $1 AND aggregate_id = $2",
    )
    .bind("tenant-aa-6")
    .bind(authority.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(revoke_count.0 >= 1, "revocation outbox event must exist");
}

// ============================================================================
// 7. Authorization query test (various combos)
// ============================================================================

#[tokio::test]
#[serial]
async fn authorization_query_combos() {
    let pool = setup_db().await;
    let operator_a = Uuid::new_v4();
    let operator_b = Uuid::new_v4();
    let scope_x = format!("scope-x-{}", &Uuid::new_v4().to_string()[..8]);
    let scope_y = format!("scope-y-{}", &Uuid::new_v4().to_string()[..8]);
    let tenant = "tenant-aa-7";

    // Grant operator_a → scope_x (active now)
    let req_a_x = GrantAuthorityRequest {
        tenant_id: tenant.to_string(),
        operator_id: operator_a,
        capability_scope: scope_x.clone(),
        constraints: Some(serde_json::json!({"max_value": 10000})),
        effective_from: Utc::now() - Duration::days(30),
        effective_until: Some(Utc::now() + Duration::days(330)),
        granted_by: Some("Director".to_string()),
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    acceptance_authority::grant_acceptance_authority(&pool, &req_a_x)
        .await
        .unwrap();

    // Grant operator_a → scope_y (not yet effective)
    let req_a_y = GrantAuthorityRequest {
        tenant_id: tenant.to_string(),
        operator_id: operator_a,
        capability_scope: scope_y.clone(),
        constraints: None,
        effective_from: Utc::now() + Duration::days(30),
        effective_until: None,
        granted_by: None,
        idempotency_key: Uuid::new_v4().to_string(),
        correlation_id: None,
        causation_id: None,
    };
    acceptance_authority::grant_acceptance_authority(&pool, &req_a_y)
        .await
        .unwrap();

    // Test 1: operator_a + scope_x + now → allowed
    let r1 = acceptance_authority::check_acceptance_authority(
        &pool,
        &AcceptanceAuthorityQuery {
            tenant_id: tenant.to_string(),
            operator_id: operator_a,
            capability_scope: scope_x.clone(),
            at_time: Utc::now(),
        },
    )
    .await
    .unwrap();
    assert!(r1.allowed, "operator_a should be allowed for scope_x now");

    // Test 2: operator_a + scope_y + now → denied (not yet effective)
    let r2 = acceptance_authority::check_acceptance_authority(
        &pool,
        &AcceptanceAuthorityQuery {
            tenant_id: tenant.to_string(),
            operator_id: operator_a,
            capability_scope: scope_y.clone(),
            at_time: Utc::now(),
        },
    )
    .await
    .unwrap();
    assert!(!r2.allowed, "operator_a should be denied for scope_y now");

    // Test 3: operator_b + scope_x + now → denied (no grant)
    let r3 = acceptance_authority::check_acceptance_authority(
        &pool,
        &AcceptanceAuthorityQuery {
            tenant_id: tenant.to_string(),
            operator_id: operator_b,
            capability_scope: scope_x.clone(),
            at_time: Utc::now(),
        },
    )
    .await
    .unwrap();
    assert!(!r3.allowed, "operator_b should be denied for scope_x");
    assert_eq!(r3.denial_reason.as_deref(), Some("no_authority_found"));

    // Test 4: operator_a + scope_y + future (day 60) → allowed
    let r4 = acceptance_authority::check_acceptance_authority(
        &pool,
        &AcceptanceAuthorityQuery {
            tenant_id: tenant.to_string(),
            operator_id: operator_a,
            capability_scope: scope_y.clone(),
            at_time: Utc::now() + Duration::days(60),
        },
    )
    .await
    .unwrap();
    assert!(
        r4.allowed,
        "operator_a should be allowed for scope_y in future"
    );

    // Test 5: idempotency conflict — same key, different body
    let mut conflicting = req_a_x.clone();
    conflicting.capability_scope = "different-scope".to_string();
    let err = acceptance_authority::grant_acceptance_authority(&pool, &conflicting)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ServiceError::ConflictingIdempotencyKey),
        "expected ConflictingIdempotencyKey, got: {:?}",
        err
    );
}
