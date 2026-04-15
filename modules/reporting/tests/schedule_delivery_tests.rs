//! Integration tests for reporting scheduled delivery.
//!
//! All tests run against real Postgres (REPORTING_DATABASE_URL, port 5443).
//! No mocks, no stubs.

mod helpers;

use helpers::{seed_trial_balance, setup_db, unique_tenant};
use reporting::domain::schedules::service::{
    create_schedule, disable_schedule, get_schedule, list_schedules, trigger_schedule,
    update_schedule_interval,
};
use serial_test::serial;

// ═══════════════════════════════════════════════════════════════════════════════
// 1. SCHEDULE CRUD E2E
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn schedule_crud_create_update_verify_persistence() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();

    // Create a delivery schedule
    let schedule = create_schedule(
        &pool,
        &tid,
        "trial_balance",
        "Weekly TB Report",
        None,
        Some(604800), // weekly
        "email",
        "finance@example.com",
        "csv",
        None,
    )
    .await
    .expect("Schedule creation should succeed");

    assert_eq!(schedule.tenant_id, tid);
    assert_eq!(schedule.report_id, "trial_balance");
    assert_eq!(schedule.schedule_name, "Weekly TB Report");
    assert_eq!(schedule.interval_secs, Some(604800));
    assert_eq!(schedule.delivery_channel, "email");
    assert_eq!(schedule.recipient, "finance@example.com");
    assert_eq!(schedule.format, "csv");
    assert_eq!(schedule.status, "active");

    // Update interval to daily
    let updated = update_schedule_interval(&pool, &tid, schedule.id, None, Some(86400))
        .await
        .expect("Update should succeed");

    assert_eq!(updated.interval_secs, Some(86400));
    assert!(updated.updated_at > schedule.updated_at);

    // Verify persistence via get
    let fetched = get_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Get should succeed")
        .expect("Schedule should exist");

    assert_eq!(fetched.id, schedule.id);
    assert_eq!(fetched.interval_secs, Some(86400));

    // Verify in list
    let schedules = list_schedules(&pool, &tid)
        .await
        .expect("List should succeed");
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0].id, schedule.id);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 2. TRIGGER EXECUTION TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn trigger_execution_creates_export_run_and_delivery_request() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();

    // Seed report data
    seed_trial_balance(&pool, &tid, "2026-01-31", "1000", "Cash", "USD", 200_000, 0).await;
    seed_trial_balance(
        &pool,
        &tid,
        "2026-01-31",
        "4000",
        "Revenue",
        "USD",
        0,
        100_000,
    )
    .await;

    // Create schedule
    let schedule = create_schedule(
        &pool,
        &tid,
        "trial_balance",
        "Daily CSV",
        None,
        Some(86400),
        "email",
        "cfo@example.com",
        "csv",
        None,
    )
    .await
    .expect("Schedule creation should succeed");

    // Trigger the schedule
    let execution = trigger_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Trigger should succeed")
        .expect("Execution should be returned for active schedule");

    assert_eq!(execution.schedule_id, schedule.id);
    assert_eq!(execution.tenant_id, tid);
    assert_eq!(execution.status, "completed");
    assert!(execution.export_run_id.is_some());
    assert!(execution.completed_at.is_some());

    // Verify the export run was created
    let export_runs = reporting::domain::exports::service::list_export_runs(&pool, &tid)
        .await
        .expect("List export runs should succeed");

    assert!(
        export_runs
            .iter()
            .any(|r| r.id == execution.export_run_id.unwrap()),
        "Export run from trigger should exist"
    );

    // Verify schedule last_triggered_at was updated
    let updated_schedule = get_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Get should succeed")
        .expect("Schedule should exist");
    assert!(updated_schedule.last_triggered_at.is_some());
}

// ═══════════════════════════════════════════════════════════════════════════════
// 3. TENANT ISOLATION TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn tenant_isolation_schedules_invisible_across_tenants() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant().to_string();
    let tenant_b = unique_tenant().to_string();

    // Create schedule under tenant A
    let _schedule_a = create_schedule(
        &pool,
        &tenant_a,
        "trial_balance",
        "Tenant A Report",
        None,
        Some(86400),
        "email",
        "a@example.com",
        "csv",
        None,
    )
    .await
    .expect("Schedule A creation should succeed");

    // Tenant B should see zero schedules
    let schedules_b = list_schedules(&pool, &tenant_b)
        .await
        .expect("List should succeed");
    assert!(
        schedules_b.is_empty(),
        "Tenant B must not see tenant A's schedules"
    );

    // Tenant A should see their schedule
    let schedules_a = list_schedules(&pool, &tenant_a)
        .await
        .expect("List should succeed");
    assert_eq!(schedules_a.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════════════════
// 4. IDEMPOTENCY TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn idempotent_schedule_creation_no_duplicate() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();
    let key = format!("sched-idem-{}", uuid::Uuid::new_v4());

    // First creation
    let sched1 = create_schedule(
        &pool,
        &tid,
        "trial_balance",
        "Idempotent Schedule",
        None,
        Some(3600),
        "email",
        "test@example.com",
        "csv",
        Some(&key),
    )
    .await
    .expect("First creation should succeed");

    // Second creation with same idempotency key
    let sched2 = create_schedule(
        &pool,
        &tid,
        "trial_balance",
        "Idempotent Schedule",
        None,
        Some(3600),
        "email",
        "test@example.com",
        "csv",
        Some(&key),
    )
    .await
    .expect("Second creation should return existing");

    assert_eq!(
        sched1.id, sched2.id,
        "Same idempotency key must return same schedule"
    );

    // Verify only one schedule exists
    let schedules = list_schedules(&pool, &tid)
        .await
        .expect("List should succeed");
    assert_eq!(
        schedules.len(),
        1,
        "No duplicate schedule should be created"
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// 5. OUTBOX EVENT TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn outbox_events_emitted_on_create_and_trigger() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();

    // Seed data for export
    seed_trial_balance(&pool, &tid, "2026-01-31", "1000", "Cash", "USD", 100_000, 0).await;

    // Create schedule — should emit schedule.created event
    let schedule = create_schedule(
        &pool,
        &tid,
        "trial_balance",
        "Outbox Test Schedule",
        None,
        Some(86400),
        "email",
        "outbox@example.com",
        "xlsx",
        None,
    )
    .await
    .expect("Schedule creation should succeed");

    // Check outbox for schedule.created event
    let created_event: (String, serde_json::Value, String) = sqlx::query_as(
        r#"SELECT event_type, payload, tenant_id
           FROM events_outbox
           WHERE aggregate_type = 'delivery_schedule' AND aggregate_id = $1
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(schedule.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("Schedule created outbox event should exist");

    assert_eq!(created_event.0, "reporting.schedule.created");
    assert_eq!(created_event.2, tid);
    let inner = &created_event.1["payload"];
    assert_eq!(inner["schedule_id"], schedule.id.to_string());
    assert_eq!(inner["delivery_channel"], "email");

    // Trigger schedule — should emit schedule.triggered event
    let execution = trigger_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Trigger should succeed")
        .expect("Execution should exist");

    let triggered_event: (String, serde_json::Value, String) = sqlx::query_as(
        r#"SELECT event_type, payload, tenant_id
           FROM events_outbox
           WHERE aggregate_type = 'schedule_execution' AND aggregate_id = $1
           ORDER BY created_at DESC LIMIT 1"#,
    )
    .bind(execution.id.to_string())
    .fetch_one(&pool)
    .await
    .expect("Schedule triggered outbox event should exist");

    assert_eq!(triggered_event.0, "reporting.schedule.triggered");
    assert_eq!(triggered_event.2, tid);
    let trigger_inner = &triggered_event.1["payload"];
    assert_eq!(trigger_inner["schedule_id"], schedule.id.to_string());
    assert_eq!(trigger_inner["execution_id"], execution.id.to_string());
}

// ═══════════════════════════════════════════════════════════════════════════════
// 6. DISABLED SCHEDULE TEST
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
#[serial]
async fn disabled_schedule_trigger_produces_no_execution() {
    let pool = setup_db().await;
    let tid = unique_tenant().to_string();

    // Seed data
    seed_trial_balance(&pool, &tid, "2026-01-31", "1000", "Cash", "USD", 50_000, 0).await;

    // Create and disable schedule
    let schedule = create_schedule(
        &pool,
        &tid,
        "trial_balance",
        "Disabled Schedule",
        None,
        Some(86400),
        "email",
        "disabled@example.com",
        "csv",
        None,
    )
    .await
    .expect("Schedule creation should succeed");

    disable_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Disable should succeed");

    // Verify status changed
    let disabled = get_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Get should succeed")
        .expect("Schedule should exist");
    assert_eq!(disabled.status, "disabled");

    // Trigger should return None for disabled schedule
    let result = trigger_schedule(&pool, &tid, schedule.id)
        .await
        .expect("Trigger should not error");

    assert!(
        result.is_none(),
        "Disabled schedule must not produce an execution"
    );

    // Verify no execution was logged
    let executions =
        reporting::domain::schedules::service::list_executions(&pool, &tid, schedule.id)
            .await
            .expect("List executions should succeed");
    assert!(
        executions.is_empty(),
        "No executions should exist for disabled schedule"
    );
}
