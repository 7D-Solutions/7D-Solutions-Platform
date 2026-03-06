use production_rs::domain::downtime::{
    DowntimeError, DowntimeRepo, EndDowntimeRequest, StartDowntimeRequest,
};
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

async fn create_test_workcenter(pool: &sqlx::PgPool, tenant: &str, code: &str) -> Uuid {
    let corr = Uuid::new_v4().to_string();
    let wc = WorkcenterRepo::create(
        pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.to_string(),
            code: code.to_string(),
            name: format!("Test WC {}", code),
            description: None,
            capacity: Some(10),
            cost_rate_minor: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create workcenter");
    wc.workcenter_id
}

// ============================================================================
// Start downtime
// ============================================================================

#[tokio::test]
#[serial]
async fn start_downtime_creates_record_and_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc_id = create_test_workcenter(&pool, &tenant, "DT-START-001").await;
    let corr = Uuid::new_v4().to_string();

    let dt = DowntimeRepo::start(
        &pool,
        &StartDowntimeRequest {
            tenant_id: tenant.clone(),
            workcenter_id: wc_id,
            reason: "Machine overheating".to_string(),
            reason_code: Some("OVERHEAT".to_string()),
            started_by: Some("operator-1".to_string()),
            notes: Some("Noticed at shift change".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("start downtime");

    assert_eq!(dt.tenant_id, tenant);
    assert_eq!(dt.workcenter_id, wc_id);
    assert_eq!(dt.reason, "Machine overheating");
    assert_eq!(dt.reason_code.as_deref(), Some("OVERHEAT"));
    assert!(dt.ended_at.is_none());

    // Verify outbox event
    let outbox_row = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 AND tenant_id = $2",
    )
    .bind(dt.downtime_id.to_string())
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox row");
    assert_eq!(outbox_row.0, "production.downtime.started");
}

// ============================================================================
// End downtime
// ============================================================================

#[tokio::test]
#[serial]
async fn end_downtime_updates_record_and_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc_id = create_test_workcenter(&pool, &tenant, "DT-END-001").await;
    let corr = Uuid::new_v4().to_string();

    let dt = DowntimeRepo::start(
        &pool,
        &StartDowntimeRequest {
            tenant_id: tenant.clone(),
            workcenter_id: wc_id,
            reason: "Planned maintenance".to_string(),
            reason_code: None,
            started_by: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .expect("start downtime");

    let ended = DowntimeRepo::end(
        &pool,
        dt.downtime_id,
        &EndDowntimeRequest {
            tenant_id: tenant.clone(),
            ended_by: Some("supervisor-1".to_string()),
        },
        &corr,
        None,
    )
    .await
    .expect("end downtime");

    assert!(ended.ended_at.is_some());
    assert_eq!(ended.ended_by.as_deref(), Some("supervisor-1"));

    // Verify outbox events (started + ended)
    let events = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(dt.downtime_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("outbox events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].0, "production.downtime.started");
    assert_eq!(events[1].0, "production.downtime.ended");
}

// ============================================================================
// Cannot end already-ended downtime
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_end_already_ended_downtime() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc_id = create_test_workcenter(&pool, &tenant, "DT-DOUBLE-001").await;
    let corr = Uuid::new_v4().to_string();

    let dt = DowntimeRepo::start(
        &pool,
        &StartDowntimeRequest {
            tenant_id: tenant.clone(),
            workcenter_id: wc_id,
            reason: "Test".to_string(),
            reason_code: None,
            started_by: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    DowntimeRepo::end(
        &pool,
        dt.downtime_id,
        &EndDowntimeRequest {
            tenant_id: tenant.clone(),
            ended_by: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    let err = DowntimeRepo::end(
        &pool,
        dt.downtime_id,
        &EndDowntimeRequest {
            tenant_id: tenant.clone(),
            ended_by: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DowntimeError::AlreadyEnded));
}

// ============================================================================
// Workcenter not found
// ============================================================================

#[tokio::test]
#[serial]
async fn start_downtime_rejects_unknown_workcenter() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let err = DowntimeRepo::start(
        &pool,
        &StartDowntimeRequest {
            tenant_id: tenant.clone(),
            workcenter_id: Uuid::new_v4(),
            reason: "Unknown WC".to_string(),
            reason_code: None,
            started_by: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DowntimeError::WorkcenterNotFound));
}

// ============================================================================
// List active and per-workcenter
// ============================================================================

#[tokio::test]
#[serial]
async fn list_active_and_per_workcenter() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let wc1 = create_test_workcenter(&pool, &tenant, "DT-LIST-A").await;
    let wc2 = create_test_workcenter(&pool, &tenant, "DT-LIST-B").await;
    let corr = Uuid::new_v4().to_string();

    // Start downtime on both
    let dt1 = DowntimeRepo::start(
        &pool,
        &StartDowntimeRequest {
            tenant_id: tenant.clone(),
            workcenter_id: wc1,
            reason: "Downtime A".to_string(),
            reason_code: None,
            started_by: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    DowntimeRepo::start(
        &pool,
        &StartDowntimeRequest {
            tenant_id: tenant.clone(),
            workcenter_id: wc2,
            reason: "Downtime B".to_string(),
            reason_code: None,
            started_by: None,
            notes: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // End one
    DowntimeRepo::end(
        &pool,
        dt1.downtime_id,
        &EndDowntimeRequest {
            tenant_id: tenant.clone(),
            ended_by: None,
        },
        &corr,
        None,
    )
    .await
    .unwrap();

    // List active: only wc2 should be active
    let active = DowntimeRepo::list_active(&pool, &tenant).await.unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].workcenter_id, wc2);

    // List per workcenter: wc1 should have 1 record (ended)
    let wc1_history = DowntimeRepo::list_for_workcenter(&pool, wc1, &tenant)
        .await
        .unwrap();
    assert_eq!(wc1_history.len(), 1);
    assert!(wc1_history[0].ended_at.is_some());
}
