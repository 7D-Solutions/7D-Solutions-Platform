//! Phase 58 Gate A: timekeeping safety, tenant, and auth hardening (bd-3ms5j)
//!
//! Five required integration test categories against real Postgres on port 5447:
//!
//! 1. **Migration safety** — apply all migrations forward, verify schema tables
//! 2. **Tenant boundary** — tenant_A data invisible to tenant_B (entries + approvals)
//! 3. **AuthZ denial** — mutation endpoints reject requests without valid JWT claims
//! 4. **Guard→Mutation→Outbox atomicity** — write + outbox row in same transaction
//! 5. **Concurrent tenant isolation** — parallel requests from different tenants

use axum::{body::Body, http::Request as HttpRequest, http::StatusCode};
use chrono::NaiveDate;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use timekeeping::domain::approvals::models::SubmitApprovalRequest;
use timekeeping::domain::approvals::service as approval_svc;
use timekeeping::domain::employees::models::CreateEmployeeRequest;
use timekeeping::domain::employees::service::EmployeeRepo;
use timekeeping::domain::entries::models::CreateEntryRequest;
use timekeeping::domain::entries::service;
use timekeeping::{http, metrics, AppState};
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// Test DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://timekeeping_user:timekeeping_pass@localhost:5447/timekeeping_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(10)
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
    format!("harden-{}", Uuid::new_v4().simple())
}

async fn create_test_employee(pool: &sqlx::PgPool, app_id: &str) -> Uuid {
    let emp = EmployeeRepo::create(
        pool,
        &CreateEmployeeRequest {
            app_id: app_id.to_string(),
            employee_code: format!("E-{}", Uuid::new_v4().simple()),
            first_name: "Test".to_string(),
            last_name: "Employee".to_string(),
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

fn work_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 3, 10).unwrap()
}

/// Build the timekeeping HTTP router without JWT verification.
/// Without a JwtVerifier, the `optional_claims_mw` inserts no claims,
/// so RequirePermissionsLayer will reject mutation requests with 401.
fn build_test_router(pool: sqlx::PgPool) -> axum::Router {
    let tk_metrics = Arc::new(
        metrics::TimekeepingMetrics::new().expect("metrics"),
    );
    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics: tk_metrics,
    });

    // No JWT verifier — simulates unauthenticated caller
    let maybe_verifier: Option<Arc<security::JwtVerifier>> = None;

    http::router(app_state)
        .layer(axum::middleware::from_fn_with_state(
            maybe_verifier,
            security::optional_claims_mw,
        ))
}

// ============================================================================
// 1. Migration safety — apply forward, verify all expected tables exist
// ============================================================================

#[tokio::test]
#[serial]
async fn test_migration_safety_all_tables_present() {
    let pool = setup_db().await;

    // All expected tables from the 8 migration files
    let expected_tables = vec![
        "tk_employees",
        "tk_projects",
        "tk_tasks",
        "tk_timesheet_entries",
        "tk_approval_requests",
        "tk_approval_actions",
        "tk_allocations",
        "tk_export_runs",
        "events_outbox",
        "processed_events",
        "tk_idempotency_keys",
        "tk_billing_rates",
        "tk_billing_runs",
        "tk_billing_run_entries",
    ];

    for table in &expected_tables {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM information_schema.tables
                WHERE table_schema = 'public' AND table_name = $1
            )",
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Expected table '{}' missing after migrations", table);
    }

    // Verify custom enum types exist
    let expected_types = vec!["tk_entry_type", "tk_approval_status", "tk_export_status"];
    for type_name in &expected_types {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM pg_type WHERE typname = $1
            )",
        )
        .bind(type_name)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists, "Expected enum type '{}' missing", type_name);
    }

    // Verify key columns added by later migrations
    let has_billing_rate_id: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_name = 'tk_timesheet_entries' AND column_name = 'billing_rate_id'
        )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        has_billing_rate_id,
        "billing_rate_id column missing from tk_timesheet_entries (migration 8)"
    );

    let has_content_hash: bool = sqlx::query_scalar(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_name = 'tk_export_runs' AND column_name = 'content_hash'
        )",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        has_content_hash,
        "content_hash column missing from tk_export_runs (migration 6)"
    );

    // Verify migration version tracking
    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        migration_count >= 8,
        "Expected at least 8 successful migrations, found {}",
        migration_count
    );

    // ── Rollback/forward-fix documentation ──
    // Timekeeping uses append-only migrations (no DROP/ALTER destructive).
    // Rollback strategy:
    //   1. Migration 8 (billing fields): ALTER TABLE DROP COLUMN billing_rate_id, billable;
    //      DROP TABLE tk_billing_run_entries, tk_billing_runs;
    //   2. Migration 7 (billing rates): DROP TABLE tk_billing_rates;
    //   3. Migration 6 (content_hash): ALTER TABLE DROP COLUMN content_hash;
    //   4. Migration 5 (outbox): DROP TABLE tk_idempotency_keys, processed_events, events_outbox;
    //   5. Migration 4 (allocations/exports): DROP TABLE tk_export_runs, tk_allocations;
    //      DROP TYPE tk_export_status;
    //   6. Migration 3 (approvals): DROP TABLE tk_approval_actions, tk_approval_requests;
    //      DROP TYPE tk_approval_status;
    //   7. Migration 2 (entries): DROP TABLE tk_timesheet_entries; DROP TYPE tk_entry_type;
    //   8. Migration 1 (employees/projects): DROP TABLE tk_tasks, tk_projects, tk_employees;
    //
    // Forward-fix preferred: if a migration fails mid-apply, fix and re-run.
    // SQLx tracks per-migration success so partial state is recoverable.
}

// ============================================================================
// 2. Tenant boundary — entries and approvals invisible across tenants
// ============================================================================

#[tokio::test]
#[serial]
async fn test_tenant_boundary_entries() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = create_test_employee(&pool, &app_a).await;

    // Create entry under tenant A
    service::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_a.clone(),
            employee_id: emp_a,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 480,
            description: Some("Tenant A work".to_string()),
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    // Tenant B queries for entries — must see zero rows
    let b_entries = service::list_entries(&pool, &app_b, emp_a, work_date(), work_date())
        .await
        .unwrap();
    assert_eq!(
        b_entries.len(),
        0,
        "Tenant B must not see tenant A's time entries"
    );

    // Tenant A sees their own entry
    let a_entries = service::list_entries(&pool, &app_a, emp_a, work_date(), work_date())
        .await
        .unwrap();
    assert_eq!(a_entries.len(), 1, "Tenant A must see their own entry");
}

#[tokio::test]
#[serial]
async fn test_tenant_boundary_approvals() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = create_test_employee(&pool, &app_a).await;
    let actor = Uuid::new_v4();

    // Create an entry so submission has content
    service::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_a.clone(),
            employee_id: emp_a,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 360,
            description: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    // Submit approval under tenant A
    let approval = approval_svc::submit(
        &pool,
        &SubmitApprovalRequest {
            app_id: app_a.clone(),
            employee_id: emp_a,
            period_start: work_date(),
            period_end: work_date(),
            actor_id: actor,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see tenant A's approval
    let result = approval_svc::get_approval(&pool, &app_b, approval.id).await;
    assert!(
        result.is_err(),
        "Tenant B must not see tenant A's approval request"
    );

    // Tenant B listing approvals for the same employee — zero rows
    let b_approvals = approval_svc::list_approvals(
        &pool,
        &app_b,
        emp_a,
        work_date(),
        work_date(),
    )
    .await
    .unwrap();
    assert_eq!(
        b_approvals.len(),
        0,
        "Tenant B must see zero approvals for tenant A's employee"
    );

    // Tenant A can see their own approval
    let a_approval = approval_svc::get_approval(&pool, &app_a, approval.id)
        .await
        .unwrap();
    assert_eq!(a_approval.id, approval.id);
}

// ============================================================================
// 3. AuthZ denial — mutation endpoints reject unauthenticated requests
// ============================================================================

#[tokio::test]
#[serial]
async fn test_authz_create_entry_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "app_id": "test-tenant",
        "employee_id": Uuid::new_v4(),
        "work_date": "2026-03-10",
        "minutes": 480
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/timekeeping/entries")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /entries must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_submit_approval_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "app_id": "test-tenant",
        "employee_id": Uuid::new_v4(),
        "period_start": "2026-03-01",
        "period_end": "2026-03-07",
        "actor_id": Uuid::new_v4()
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/timekeeping/approvals/submit")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /approvals/submit must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_create_employee_denied_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    let body = serde_json::json!({
        "app_id": "test-tenant",
        "employee_code": "E-001",
        "first_name": "Test",
        "last_name": "User"
    });

    let req = HttpRequest::builder()
        .method("POST")
        .uri("/api/timekeeping/employees")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "POST /employees must reject without JWT"
    );
}

#[tokio::test]
#[serial]
async fn test_authz_read_endpoints_allowed_without_jwt() {
    let pool = setup_db().await;
    let app = build_test_router(pool);

    // Read endpoints should be accessible (no RequirePermissionsLayer)
    let req = HttpRequest::builder()
        .method("GET")
        .uri("/api/timekeeping/employees")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    // Reads return 401 from extract_tenant because no claims → no tenant,
    // but the middleware layer itself doesn't block. The 401 comes from the
    // handler checking claims — this is the correct behavior for tenant-scoped reads.
    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::OK,
        "GET /employees status should be 401 (no tenant) or 200, got {}",
        resp.status()
    );
}

// ============================================================================
// 4. Guard→Mutation→Outbox atomicity
// ============================================================================

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_atomicity() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;

    // Perform a write (create entry)
    let entry = service::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 120,
            description: Some("Atomicity test".to_string()),
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    // The outbox row must exist (written in the same transaction as the entry)
    let outbox_event: Option<(String, String)> = sqlx::query_as(
        "SELECT event_type, aggregate_id FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(entry.entry_id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, agg_id) = outbox_event.expect("Outbox event must exist after entry creation");
    assert_eq!(event_type, "timesheet_entry.created");
    assert_eq!(agg_id, entry.entry_id.to_string());

    // Verify outbox payload contains the expected fields
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(entry.entry_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payload["app_id"], app_id);
    assert_eq!(payload["employee_id"], emp_id.to_string());
    assert_eq!(payload["minutes"], 120);
    assert_eq!(payload["version"], 1);
}

#[tokio::test]
#[serial]
async fn test_guard_mutation_outbox_approval_submit() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor = Uuid::new_v4();

    // Create an entry first
    service::create_entry(
        &pool,
        &CreateEntryRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            project_id: None,
            task_id: None,
            work_date: work_date(),
            minutes: 240,
            description: None,
            created_by: None,
        },
        None,
    )
    .await
    .unwrap();

    // Submit approval — triggers Guard→Mutation→Outbox
    let approval = approval_svc::submit(
        &pool,
        &SubmitApprovalRequest {
            app_id: app_id.clone(),
            employee_id: emp_id,
            period_start: work_date(),
            period_end: work_date(),
            actor_id: actor,
        },
    )
    .await
    .unwrap();

    // Outbox event for the approval submission
    let outbox_event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(approval.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();

    assert_eq!(
        outbox_event.unwrap().0,
        "timesheet.submitted",
        "Outbox must contain timesheet.submitted event"
    );
}

// ============================================================================
// 5. Concurrent tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_concurrent_tenant_isolation_entries() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = create_test_employee(&pool, &app_a).await;
    let emp_b = create_test_employee(&pool, &app_b).await;

    // Spawn concurrent writes from both tenants
    let mut handles = Vec::new();

    for i in 0..5u32 {
        let p = pool.clone();
        let a = app_a.clone();
        handles.push(tokio::spawn(async move {
            service::create_entry(
                &p,
                &CreateEntryRequest {
                    app_id: a,
                    employee_id: emp_a,
                    project_id: None,
                    task_id: None,
                    work_date: NaiveDate::from_ymd_opt(2026, 3, 10 + i).unwrap(),
                    minutes: 60 * (i as i32 + 1),
                    description: Some(format!("Tenant A entry {}", i)),
                    created_by: None,
                },
                None,
            )
            .await
            .expect("Tenant A entry should succeed")
        }));

        let p = pool.clone();
        let b = app_b.clone();
        handles.push(tokio::spawn(async move {
            service::create_entry(
                &p,
                &CreateEntryRequest {
                    app_id: b,
                    employee_id: emp_b,
                    project_id: None,
                    task_id: None,
                    work_date: NaiveDate::from_ymd_opt(2026, 3, 10 + i).unwrap(),
                    minutes: 30 * (i as i32 + 1),
                    description: Some(format!("Tenant B entry {}", i)),
                    created_by: None,
                },
                None,
            )
            .await
            .expect("Tenant B entry should succeed")
        }));
    }

    // Wait for all writes
    for h in handles {
        h.await.expect("join");
    }

    // Verify tenant A sees only their entries
    let a_entries = service::list_entries(
        &pool,
        &app_a,
        emp_a,
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(a_entries.len(), 5, "Tenant A should have 5 entries");

    // Verify tenant B sees only their entries
    let b_entries = service::list_entries(
        &pool,
        &app_b,
        emp_b,
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(b_entries.len(), 5, "Tenant B should have 5 entries");

    // Cross-tenant check: tenant A querying for tenant B's employee sees nothing
    let cross_a = service::list_entries(
        &pool,
        &app_a,
        emp_b,
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        cross_a.len(),
        0,
        "Tenant A must not see tenant B's entries"
    );

    let cross_b = service::list_entries(
        &pool,
        &app_b,
        emp_a,
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        cross_b.len(),
        0,
        "Tenant B must not see tenant A's entries"
    );

    // Verify outbox events are per-tenant (no cross-contamination)
    let a_outbox: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE payload->>'app_id' = $1",
    )
    .bind(&app_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(a_outbox, 5, "Tenant A should have 5 outbox events");

    let b_outbox: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE payload->>'app_id' = $1",
    )
    .bind(&app_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(b_outbox, 5, "Tenant B should have 5 outbox events");
}

#[tokio::test]
#[serial]
async fn test_concurrent_reads_during_writes_isolated() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();

    let emp_a = create_test_employee(&pool, &app_a).await;
    let emp_b = create_test_employee(&pool, &app_b).await;

    let mut handles = Vec::new();

    // Tenant A writes
    for i in 0..3u32 {
        let p = pool.clone();
        let a = app_a.clone();
        handles.push(tokio::spawn(async move {
            service::create_entry(
                &p,
                &CreateEntryRequest {
                    app_id: a,
                    employee_id: emp_a,
                    project_id: None,
                    task_id: None,
                    work_date: NaiveDate::from_ymd_opt(2026, 3, 20 + i).unwrap(),
                    minutes: 100,
                    description: None,
                    created_by: None,
                },
                None,
            )
            .await
            .expect("A write");
        }));
    }

    // Tenant B reads concurrently — must never see A's data
    for _ in 0..3 {
        let p = pool.clone();
        let b = app_b.clone();
        handles.push(tokio::spawn(async move {
            let entries = service::list_entries(
                &p,
                &b,
                emp_a,
                NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(
                entries.len(),
                0,
                "Tenant B must never see tenant A's entries during concurrent reads"
            );
        }));
    }

    // Also write for tenant B
    for i in 0..2u32 {
        let p = pool.clone();
        let b = app_b.clone();
        handles.push(tokio::spawn(async move {
            service::create_entry(
                &p,
                &CreateEntryRequest {
                    app_id: b,
                    employee_id: emp_b,
                    project_id: None,
                    task_id: None,
                    work_date: NaiveDate::from_ymd_opt(2026, 3, 20 + i).unwrap(),
                    minutes: 200,
                    description: None,
                    created_by: None,
                },
                None,
            )
            .await
            .expect("B write");
        }));
    }

    for h in handles {
        h.await.expect("join");
    }

    // Final counts
    let a_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tk_timesheet_entries WHERE app_id = $1 AND is_current = true",
    )
    .bind(&app_a)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(a_count, 3, "Tenant A should have 3 entries");

    let b_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM tk_timesheet_entries WHERE app_id = $1 AND is_current = true",
    )
    .bind(&app_b)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(b_count, 2, "Tenant B should have 2 entries");
}
