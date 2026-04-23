use chrono::Utc;
use production_rs::domain::operations::OperationRepo;
use production_rs::domain::routings::{AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo};
use production_rs::domain::time_entries::{
    ApproveTimeEntryRequest, ManualEntryRequest, RejectTimeEntryRequest, StartTimerRequest,
    StopTimerRequest, TimeEntryError, TimeEntryRepo, TimeEntryStatus,
};
use production_rs::domain::work_orders::{CreateWorkOrderRequest, WorkOrderRepo};
use production_rs::domain::workcenters::{CreateWorkcenterRequest, WorkcenterRepo};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db?sslmode=require".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to production test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run production migrations");

    pool
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Create a released WO with one operation for timekeeping tests.
async fn setup_wo_with_ops(pool: &sqlx::PgPool, tenant: &str) -> (Uuid, Uuid) {
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.to_string(),
            code: format!("WC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Test WC".to_string(),
            description: None,
            capacity: Some(5),
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create workcenter");

    let rt = RoutingRepo::create(
        pool,
        &CreateRoutingRequest {
            tenant_id: tenant.to_string(),
            name: format!("RT-{}", &Uuid::new_v4().to_string()[..8]),
            description: None,
            item_id: None,
            bom_revision_id: None,
            revision: None,
            effective_from_date: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create routing");

    RoutingRepo::add_step(
        pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.to_string(),
            sequence_number: 10,
            workcenter_id: wc.workcenter_id,
            operation_name: "Assembly".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: Some(true),
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("add step");

    RoutingRepo::release(pool, rt.routing_template_id, tenant, &corr, None)
        .await
        .expect("release routing");

    let wo = WorkOrderRepo::create(
        pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.to_string(),
            order_number: format!("WO-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: Uuid::new_v4(),
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: Some(rt.routing_template_id),
            planned_quantity: 5,
            planned_start: None,
            planned_end: None,
            correlation_id: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create WO");

    WorkOrderRepo::release(pool, wo.work_order_id, tenant, &corr, None)
        .await
        .expect("release WO");

    let ops = OperationRepo::initialize(pool, wo.work_order_id, tenant, &corr, None)
        .await
        .expect("initialize ops");

    (wo.work_order_id, ops[0].operation_id)
}

// ============================================================================
// Start timer, then stop — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn start_and_stop_timer() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: Some(op_id),
            actor_id: "operator-1".to_string(),
            notes: Some("starting assembly".to_string()),
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start timer");

    assert!(entry.end_ts.is_none());
    assert!(entry.minutes.is_none());
    assert_eq!(entry.actor_id, "operator-1");
    assert_eq!(entry.work_order_id, wo_id);
    assert_eq!(entry.operation_id, Some(op_id));

    let stopped = TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("stop timer");

    assert!(stopped.end_ts.is_some());
    assert!(stopped.minutes.is_some());
}

// ============================================================================
// Start timer against WO only (no operation)
// ============================================================================

#[tokio::test]
#[serial]
async fn start_timer_wo_only() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-2".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start timer WO-only");

    assert_eq!(entry.operation_id, None);
    assert_eq!(entry.work_order_id, wo_id);
}

// ============================================================================
// Manual entry — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn manual_entry_creates_completed_time_entry() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id) = setup_wo_with_ops(&pool, &tenant).await;

    let start = Utc::now() - chrono::Duration::hours(2);
    let end = Utc::now();

    let entry = TimeEntryRepo::manual_entry(
        &pool,
        &ManualEntryRequest {
            work_order_id: wo_id,
            operation_id: Some(op_id),
            actor_id: "operator-3".to_string(),
            start_ts: start,
            end_ts: end,
            minutes: 120,
            notes: Some("retroactive entry".to_string()),
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("manual entry");

    assert_eq!(entry.minutes, Some(120));
    assert!(entry.end_ts.is_some());
    assert_eq!(entry.notes.as_deref(), Some("retroactive entry"));
}

// ============================================================================
// Manual entry rejects invalid time range
// ============================================================================

#[tokio::test]
#[serial]
async fn manual_entry_rejects_end_before_start() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let now = Utc::now();
    let err = TimeEntryRepo::manual_entry(
        &pool,
        &ManualEntryRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-4".to_string(),
            start_ts: now,
            end_ts: now - chrono::Duration::hours(1),
            minutes: 60,
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("should reject");

    assert!(
        matches!(err, TimeEntryError::InvalidTimeRange),
        "Expected InvalidTimeRange, got: {:?}",
        err
    );
}

// ============================================================================
// Double-stop rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn stop_timer_rejects_double_stop() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-5".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start");

    TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("first stop");

    let err = TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("double stop");

    assert!(
        matches!(err, TimeEntryError::AlreadyStopped),
        "Expected AlreadyStopped, got: {:?}",
        err
    );
}

// ============================================================================
// Events emitted for start and stop
// ============================================================================

#[tokio::test]
#[serial]
async fn time_entry_events_emitted() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-6".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start");

    TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("stop");

    let events = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(entry.time_entry_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch events");

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "production.time_entry_created",
            "production.time_entry_stopped",
        ]
    );
}

// ============================================================================
// List time entries for work order
// ============================================================================

#[tokio::test]
#[serial]
async fn list_time_entries_for_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    // Create two entries
    TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "op-a".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("entry 1");

    TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "op-b".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("entry 2");

    let entries = TimeEntryRepo::list_by_work_order(&pool, wo_id, &tenant)
        .await
        .expect("list");

    assert_eq!(entries.len(), 2);
}

// ============================================================================
// Invalid work order rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn start_timer_rejects_invalid_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let err = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: Uuid::new_v4(),
            operation_id: None,
            actor_id: "operator-x".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("should reject");

    assert!(
        matches!(err, TimeEntryError::WorkOrderNotFound),
        "Expected WorkOrderNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// Invalid operation rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn start_timer_rejects_invalid_operation() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let err = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: Some(Uuid::new_v4()),
            actor_id: "operator-y".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("should reject");

    assert!(
        matches!(err, TimeEntryError::OperationNotFound),
        "Expected OperationNotFound, got: {:?}",
        err
    );
}

// ============================================================================
// Approve running timer → 422 StillRunning
// ============================================================================

#[tokio::test]
#[serial]
async fn approve_running_entry_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-a1".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start timer");

    assert!(entry.end_ts.is_none(), "timer should still be running");

    let err = TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "supervisor-1".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("should reject running entry");

    assert!(
        matches!(err, TimeEntryError::StillRunning),
        "Expected StillRunning, got: {:?}",
        err
    );

    // Verify status unchanged
    let entries = TimeEntryRepo::list_by_work_order(&pool, wo_id, &tenant)
        .await
        .expect("list");
    let e = entries
        .iter()
        .find(|e| e.time_entry_id == entry.time_entry_id)
        .unwrap();
    assert_eq!(e.status, TimeEntryStatus::Pending);
}

// ============================================================================
// Approve stopped entry → status=approved, event in outbox (atomicity)
// ============================================================================

#[tokio::test]
#[serial]
async fn approve_stopped_entry_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: Some(op_id),
            actor_id: "operator-a2".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start");

    TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("stop");

    let approved = TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "supervisor-2".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("approve");

    assert_eq!(approved.status, TimeEntryStatus::Approved);
    assert_eq!(approved.approved_by.as_deref(), Some("supervisor-2"));
    assert!(approved.approved_at.is_some());

    // Verify approval event in outbox (atomicity: both status and event committed together)
    let events = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(entry.time_entry_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch events");

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert!(
        types.contains(&"production.time_entry_approved"),
        "Expected approval event in outbox, got: {:?}",
        types
    );
}

// ============================================================================
// Approve already-approved entry → 409 AlreadyApproved
// ============================================================================

#[tokio::test]
#[serial]
async fn approve_already_approved_entry_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-a3".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start");

    TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("stop");

    TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "supervisor-3".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("first approve");

    let err = TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "supervisor-3".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("double approve");

    assert!(
        matches!(err, TimeEntryError::AlreadyApproved),
        "Expected AlreadyApproved, got: {:?}",
        err
    );
}

// ============================================================================
// Reject entry → status=rejected, no approval event emitted
// ============================================================================

#[tokio::test]
#[serial]
async fn reject_entry_stores_reason_no_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-a4".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start");

    let rejected = TimeEntryRepo::reject_time_entry(
        &pool,
        entry.time_entry_id,
        &RejectTimeEntryRequest {
            rejected_by: "supervisor-4".to_string(),
            rejection_reason: "Incorrect work order".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("reject");

    assert_eq!(rejected.status, TimeEntryStatus::Rejected);
    assert_eq!(rejected.rejected_by.as_deref(), Some("supervisor-4"));
    assert_eq!(
        rejected.rejected_reason.as_deref(),
        Some("Incorrect work order")
    );

    // Verify no approval event was emitted
    let approval_events = sqlx::query_as::<_, (i64,)>(
        "SELECT COUNT(*) FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.time_entry_approved'",
    )
    .bind(entry.time_entry_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(
        approval_events.0, 0,
        "No approval event should be emitted on rejection"
    );
}

// ============================================================================
// Double-reject → 409 AlreadyRejected
// ============================================================================

#[tokio::test]
#[serial]
async fn reject_already_rejected_entry_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-a5".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("start");

    TimeEntryRepo::reject_time_entry(
        &pool,
        entry.time_entry_id,
        &RejectTimeEntryRequest {
            rejected_by: "supervisor-5".to_string(),
            rejection_reason: "Wrong entry".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("first reject");

    let err = TimeEntryRepo::reject_time_entry(
        &pool,
        entry.time_entry_id,
        &RejectTimeEntryRequest {
            rejected_by: "supervisor-5".to_string(),
            rejection_reason: "Wrong entry again".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect_err("double reject");

    assert!(
        matches!(err, TimeEntryError::AlreadyRejected),
        "Expected AlreadyRejected, got: {:?}",
        err
    );
}

// ============================================================================
// Cross-tenant: tenant A approver cannot approve tenant B entry
// ============================================================================

#[tokio::test]
#[serial]
async fn cross_tenant_approve_blocked() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_ops(&pool, &tenant_a).await;

    let entry = TimeEntryRepo::start_timer(
        &pool,
        &StartTimerRequest {
            work_order_id: wo_id,
            operation_id: None,
            actor_id: "operator-a6".to_string(),
            notes: None,
            idempotency_key: None,
        },
        &tenant_a,
        &corr,
        None,
    )
    .await
    .expect("start");

    TimeEntryRepo::stop_timer(
        &pool,
        entry.time_entry_id,
        &StopTimerRequest { end_ts: None },
        &tenant_a,
        &corr,
        None,
    )
    .await
    .expect("stop");

    // Attempt approval from tenant_b — must fail with NotFound (entry invisible cross-tenant)
    let err = TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "supervisor-b".to_string(),
        },
        &tenant_b,
        &corr,
        None,
    )
    .await
    .expect_err("cross-tenant approve must fail");

    assert!(
        matches!(err, TimeEntryError::NotFound),
        "Expected NotFound for cross-tenant approve, got: {:?}",
        err
    );
}
