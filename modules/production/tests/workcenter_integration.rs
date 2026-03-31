use production_rs::domain::workcenters::{
    CreateWorkcenterRequest, UpdateWorkcenterRequest, WorkcenterRepo,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db".to_string()
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

// ============================================================================
// Create workcenter
// ============================================================================

#[tokio::test]
#[serial]
async fn create_workcenter_persists_and_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: "WC-001".to_string(),
            name: "Assembly Line 1".to_string(),
            description: Some("Main assembly line".to_string()),
            capacity: Some(10),
            cost_rate_minor: Some(5000),
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create workcenter");

    assert_eq!(wc.tenant_id, tenant);
    assert_eq!(wc.code, "WC-001");
    assert_eq!(wc.name, "Assembly Line 1");
    assert_eq!(wc.description.as_deref(), Some("Main assembly line"));
    assert_eq!(wc.capacity, Some(10));
    assert_eq!(wc.cost_rate_minor, Some(5000));
    assert!(wc.is_active);

    // Verify outbox event was created
    let outbox_row = sqlx::query_as::<_, (String, String)>(
        "SELECT event_type, aggregate_id FROM production_outbox WHERE aggregate_id = $1 AND tenant_id = $2",
    )
    .bind(wc.workcenter_id.to_string())
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox row");

    assert_eq!(outbox_row.0, "production.workcenter_created");
    assert_eq!(outbox_row.1, wc.workcenter_id.to_string());
}

// ============================================================================
// Duplicate code rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn create_workcenter_rejects_duplicate_code() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let req = CreateWorkcenterRequest {
        tenant_id: tenant.clone(),
        code: "DUP-001".to_string(),
        name: "First".to_string(),
        description: None,
        capacity: None,
        cost_rate_minor: None,
        idempotency_key: None,
    };

    WorkcenterRepo::create(&pool, &req, &corr, None)
        .await
        .expect("first create");

    let err = WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: "DUP-001".to_string(),
            name: "Second".to_string(),
            description: None,
            capacity: None,
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect_err("should reject duplicate");

    let msg = format!("{}", err);
    assert!(msg.contains("DUP-001"), "Error should mention code: {}", msg);
}

// ============================================================================
// Update workcenter
// ============================================================================

#[tokio::test]
#[serial]
async fn update_workcenter_changes_fields_and_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: "UPD-001".to_string(),
            name: "Original Name".to_string(),
            description: None,
            capacity: Some(5),
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create");

    let updated = WorkcenterRepo::update(
        &pool,
        wc.workcenter_id,
        &UpdateWorkcenterRequest {
            tenant_id: tenant.clone(),
            name: Some("New Name".to_string()),
            description: Some("Now has description".to_string()),
            capacity: Some(20),
            cost_rate_minor: None,
        },
        &corr,
        None,
    )
    .await
    .expect("update");

    assert_eq!(updated.name, "New Name");
    assert_eq!(updated.description.as_deref(), Some("Now has description"));
    assert_eq!(updated.capacity, Some(20));

    // Verify update event
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.workcenter_updated'",
    )
    .bind(wc.workcenter_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count.0, 1, "Should have one update event");
}

// ============================================================================
// List workcenters
// ============================================================================

#[tokio::test]
#[serial]
async fn list_workcenters_returns_tenant_scoped_results() {
    let pool = setup_db().await;
    let tenant_a = unique_tenant();
    let tenant_b = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create 2 in tenant_a, 1 in tenant_b
    for code in ["LIST-A1", "LIST-A2"] {
        WorkcenterRepo::create(
            &pool,
            &CreateWorkcenterRequest {
                tenant_id: tenant_a.clone(),
                code: code.to_string(),
                name: format!("WC {}", code),
                description: None,
                capacity: None,
                cost_rate_minor: None,
                idempotency_key: None,
            },
            &corr,
            None,
        )
        .await
        .expect("create a");
    }

    WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant_b.clone(),
            code: "LIST-B1".to_string(),
            name: "WC B1".to_string(),
            description: None,
            capacity: None,
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create b");

    let (list_a, total_a) = WorkcenterRepo::list(&pool, &tenant_a, 1, 50).await.expect("list a");
    let (list_b, total_b) = WorkcenterRepo::list(&pool, &tenant_b, 1, 50).await.expect("list b");

    assert_eq!(list_a.len(), 2);
    assert_eq!(total_a, 2);
    assert_eq!(list_b.len(), 1);
    assert_eq!(total_b, 1);
    // Ordered by code
    assert_eq!(list_a[0].code, "LIST-A1");
    assert_eq!(list_a[1].code, "LIST-A2");
}

// ============================================================================
// Deactivate workcenter
// ============================================================================

#[tokio::test]
#[serial]
async fn deactivate_workcenter_sets_inactive_and_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: "DEACT-001".to_string(),
            name: "To Deactivate".to_string(),
            description: None,
            capacity: None,
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create");

    assert!(wc.is_active);

    let deactivated =
        WorkcenterRepo::deactivate(&pool, wc.workcenter_id, &tenant, &corr, None)
            .await
            .expect("deactivate");

    assert!(!deactivated.is_active);

    // Verify deactivation event
    let outbox_row = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.workcenter_deactivated'",
    )
    .bind(wc.workcenter_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("deactivation event");

    assert_eq!(outbox_row.0, "production.workcenter_deactivated");
}

// ============================================================================
// Deactivate is idempotent
// ============================================================================

#[tokio::test]
#[serial]
async fn deactivate_workcenter_is_idempotent() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: "IDEM-001".to_string(),
            name: "Idempotent".to_string(),
            description: None,
            capacity: None,
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create");

    // Deactivate twice
    let first = WorkcenterRepo::deactivate(&pool, wc.workcenter_id, &tenant, &corr, None)
        .await
        .expect("first deactivate");
    assert!(!first.is_active);

    let second = WorkcenterRepo::deactivate(&pool, wc.workcenter_id, &tenant, &corr, None)
        .await
        .expect("second deactivate should not fail");
    assert!(!second.is_active);
}

// ============================================================================
// Event replay-safety: same event_id not duplicated
// ============================================================================

#[tokio::test]
#[serial]
async fn outbox_events_have_unique_event_ids() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        &pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.clone(),
            code: "REPLAY-001".to_string(),
            name: "Replay Test".to_string(),
            description: None,
            capacity: None,
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create");

    // Update to generate a second event
    WorkcenterRepo::update(
        &pool,
        wc.workcenter_id,
        &UpdateWorkcenterRequest {
            tenant_id: tenant.clone(),
            name: Some("Updated".to_string()),
            description: None,
            capacity: None,
            cost_rate_minor: None,
        },
        &corr,
        None,
    )
    .await
    .expect("update");

    // All event_ids should be unique
    let rows = sqlx::query_as::<_, (uuid::Uuid,)>(
        "SELECT event_id FROM production_outbox WHERE aggregate_id = $1",
    )
    .bind(wc.workcenter_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch event_ids");

    let ids: std::collections::HashSet<_> = rows.iter().map(|r| r.0).collect();
    assert_eq!(ids.len(), rows.len(), "All event_ids must be unique");
}
