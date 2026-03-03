//! Integration tests for clock in/out sessions (bd-1aalb).
//!
//! Covers:
//! 1. Clock in/out E2E — clock in, clock out, verify duration and persistence
//! 2. Concurrent session guard — second clock-in rejected while open
//! 3. Tenant isolation — sessions invisible across app boundaries
//! 4. Idempotency — duplicate clock-in with same key yields no duplicate
//! 5. Outbox events — clock_in and clock_out events written correctly

use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use timekeeping::domain::clock::models::{ClockError, ClockInRequest, ClockOutRequest};
use timekeeping::domain::clock::service::{clock_in, clock_out, list_sessions};
use timekeeping::domain::employees::models::CreateEmployeeRequest;
use timekeeping::domain::employees::service::EmployeeRepo;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://timekeeping_user:timekeeping_pass@localhost:5447/timekeeping_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to timekeeping test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run timekeeping migrations");

    pool
}

fn unique_app() -> String {
    format!("clock-test-{}", Uuid::new_v4().simple())
}

async fn create_test_employee(pool: &sqlx::PgPool, app_id: &str) -> Uuid {
    let emp = EmployeeRepo::create(
        pool,
        &CreateEmployeeRequest {
            app_id: app_id.to_string(),
            employee_code: format!("E-{}", Uuid::new_v4().simple()),
            first_name: "Clock".to_string(),
            last_name: "Tester".to_string(),
            email: None,
            department: None,
            external_payroll_id: None,
            hourly_rate_minor: None,
            currency: None,
        },
    )
    .await
    .unwrap();
    emp.id
}

// ============================================================================
// 1. Clock in/out E2E — duration calculated, record persisted
// ============================================================================

#[tokio::test]
#[serial]
async fn test_clock_in_out_e2e() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    // Clock in
    let session = clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(session.app_id, app_id);
    assert_eq!(session.employee_id, emp_id);
    assert_eq!(session.status, "open");
    assert!(session.clock_out_at.is_none());
    assert!(session.duration_minutes.is_none());

    // Clock out
    let closed = clock_out(
        &pool,
        &ClockOutRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(closed.id, session.id);
    assert_eq!(closed.status, "closed");
    assert!(closed.clock_out_at.is_some());
    assert!(
        closed.duration_minutes.is_some(),
        "Duration must be calculated on clock-out"
    );
    // Duration should be >= 0 (test runs fast, so likely 0 or 1 minute)
    assert!(
        closed.duration_minutes.unwrap() >= 0,
        "Duration must be non-negative"
    );

    // Verify persistence via list
    let sessions = list_sessions(&pool, &app_id, emp_id).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].status, "closed");
    assert_eq!(sessions[0].id, session.id);
}

// ============================================================================
// 2. Concurrent session guard — second clock-in rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_concurrent_session_guard() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    // Clock in — should succeed
    clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Second clock-in without clock-out — must fail
    let err = clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, ClockError::ConcurrentSession(_)),
        "Expected ConcurrentSession, got: {err:?}"
    );

    // After clock-out, a new clock-in should succeed
    clock_out(
        &pool,
        &ClockOutRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    let second_session = clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(second_session.status, "open");
}

// ============================================================================
// 3. Tenant isolation — sessions invisible across app boundaries
// ============================================================================

#[tokio::test]
#[serial]
async fn test_clock_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = create_test_employee(&pool, &app_a).await;

    // Clock in under tenant A
    clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_a.clone(),
            employee_id: emp_a,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see tenant A's sessions
    let sessions_b = list_sessions(&pool, &app_b, emp_a).await.unwrap();
    assert!(
        sessions_b.is_empty(),
        "Tenant B must not see tenant A's clock sessions"
    );

    // Tenant B cannot clock out tenant A's session
    let err = clock_out(
        &pool,
        &ClockOutRequest {
            app_id: app_b.clone(),
            employee_id: emp_a,
            idempotency_key: None,
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, ClockError::NoOpenSession(_)),
        "Tenant B must not be able to clock out tenant A's employee: {err:?}"
    );
}

// ============================================================================
// 4. Idempotency — duplicate clock-in with same key, no duplicate session
// ============================================================================

#[tokio::test]
#[serial]
async fn test_clock_in_idempotency() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let idem_key = format!("idem-{}", Uuid::new_v4());

    // First clock-in with idempotency key
    let session = clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: Some(idem_key.clone()),
        },
    )
    .await
    .unwrap();

    assert_eq!(session.status, "open");

    // Close the session so the concurrent-session guard doesn't fire
    clock_out(
        &pool,
        &ClockOutRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Second clock-in with the SAME idempotency key — should get IdempotentReplay
    let err = clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: Some(idem_key.clone()),
        },
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, ClockError::IdempotentReplay { .. }),
        "Expected IdempotentReplay, got: {err:?}"
    );

    // Verify no duplicate session was created — should have exactly 1 session (the closed one)
    let sessions = list_sessions(&pool, &app_id, emp_id).await.unwrap();
    assert_eq!(
        sessions.len(),
        1,
        "Idempotent replay must not create a duplicate session"
    );
}

// ============================================================================
// 5. Outbox events — clock_in and clock_out events written
// ============================================================================

#[tokio::test]
#[serial]
async fn test_clock_outbox_events() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    // Clock in
    let session = clock_in(
        &pool,
        &ClockInRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Verify clock_in outbox event
    let clock_in_event: Option<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM events_outbox \
         WHERE aggregate_id = $1 AND event_type = 'clock_session.clocked_in'",
    )
    .bind(session.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, payload) = clock_in_event.expect("clock_in outbox event must exist");
    assert_eq!(event_type, "clock_session.clocked_in");
    assert_eq!(payload["app_id"], app_id);
    assert_eq!(payload["employee_id"], emp_id.to_string());

    // Clock out
    clock_out(
        &pool,
        &ClockOutRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            idempotency_key: None,
        },
    )
    .await
    .unwrap();

    // Verify clock_out outbox event
    let clock_out_event: Option<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT event_type, payload FROM events_outbox \
         WHERE aggregate_id = $1 AND event_type = 'clock_session.clocked_out'",
    )
    .bind(session.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, payload) = clock_out_event.expect("clock_out outbox event must exist");
    assert_eq!(event_type, "clock_session.clocked_out");
    assert_eq!(payload["app_id"], app_id);
    assert_eq!(payload["employee_id"], emp_id.to_string());
    assert!(
        payload.get("duration_minutes").is_some(),
        "clock_out event payload must include duration_minutes"
    );
}
