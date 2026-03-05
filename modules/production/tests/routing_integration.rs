use chrono::NaiveDate;
use production_rs::domain::routings::{
    AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo, UpdateRoutingRequest,
};
use production_rs::domain::workcenters::{CreateWorkcenterRequest, WorkcenterRepo};
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

async fn create_test_workcenter(pool: &sqlx::PgPool, tenant: &str) -> Uuid {
    let corr = Uuid::new_v4().to_string();
    let wc = WorkcenterRepo::create(
        pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.to_string(),
            code: format!("WC-{}", Uuid::new_v4().to_string().split('-').next().unwrap()),
            name: "Test Workcenter".to_string(),
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
// Create routing
// ============================================================================

#[tokio::test]
#[serial]
async fn create_routing_persists_and_emits_event() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let item_id = Uuid::new_v4();

    let rt = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Main Assembly Routing".to_string(),
            description: Some("Standard routing for assembly".to_string()),
            item_id: Some(item_id),
            bom_revision_id: None,
            revision: Some("A".to_string()),
            effective_from_date: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
        },
        &corr,
        None,
    )
    .await
    .expect("create routing");

    assert_eq!(rt.tenant_id, tenant);
    assert_eq!(rt.name, "Main Assembly Routing");
    assert_eq!(rt.revision, "A");
    assert_eq!(rt.status, "draft");
    assert_eq!(rt.item_id, Some(item_id));
    assert!(rt.is_active);

    // Verify outbox event
    let outbox_row = sqlx::query_as::<_, (String, String)>(
        "SELECT event_type, aggregate_id FROM production_outbox WHERE aggregate_id = $1 AND tenant_id = $2 AND event_type = 'production.routing_created'",
    )
    .bind(rt.routing_template_id.to_string())
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("outbox row");

    assert_eq!(outbox_row.0, "production.routing_created");
}

// ============================================================================
// Duplicate revision rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn create_routing_rejects_duplicate_revision() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let item_id = Uuid::new_v4();

    let req = CreateRoutingRequest {
        tenant_id: tenant.clone(),
        name: "Routing Rev A".to_string(),
        description: None,
        item_id: Some(item_id),
        bom_revision_id: None,
        revision: Some("A".to_string()),
        effective_from_date: None,
    };

    RoutingRepo::create(&pool, &req, &corr, None)
        .await
        .expect("first create");

    let err = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Routing Rev A duplicate".to_string(),
            description: None,
            item_id: Some(item_id),
            bom_revision_id: None,
            revision: Some("A".to_string()),
            effective_from_date: None,
        },
        &corr,
        None,
    )
    .await
    .expect_err("should reject duplicate revision");

    let msg = format!("{}", err);
    assert!(msg.contains("Duplicate revision"), "Error: {}", msg);
}

// ============================================================================
// Query routing by part + date
// ============================================================================

#[tokio::test]
#[serial]
async fn query_routing_by_item_and_date() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let item_id = Uuid::new_v4();

    // Create two routings with different effective dates
    RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Early Routing".to_string(),
            description: None,
            item_id: Some(item_id),
            bom_revision_id: None,
            revision: Some("1".to_string()),
            effective_from_date: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
        },
        &corr,
        None,
    )
    .await
    .expect("create early");

    RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Later Routing".to_string(),
            description: None,
            item_id: Some(item_id),
            bom_revision_id: None,
            revision: Some("2".to_string()),
            effective_from_date: Some(NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()),
        },
        &corr,
        None,
    )
    .await
    .expect("create later");

    // Query for date between — should return only the early one
    let results = RoutingRepo::find_by_item_and_date(
        &pool,
        &tenant,
        item_id,
        NaiveDate::from_ymd_opt(2026, 3, 15).unwrap(),
    )
    .await
    .expect("query");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Early Routing");

    // Query for date after both — should return both, most recent first
    let results2 = RoutingRepo::find_by_item_and_date(
        &pool,
        &tenant,
        item_id,
        NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
    )
    .await
    .expect("query2");

    assert_eq!(results2.len(), 2);
    assert_eq!(results2[0].name, "Later Routing");
}

// ============================================================================
// Add steps with workcenter validation
// ============================================================================

#[tokio::test]
#[serial]
async fn add_step_validates_workcenter() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let rt = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Step Test Routing".to_string(),
            description: None,
            item_id: None,
            bom_revision_id: None,
            revision: None,
            effective_from_date: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create routing");

    // Try to add step with invalid workcenter
    let err = RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: Uuid::new_v4(), // non-existent
            operation_name: "Assemble".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: None,
        },
        &corr,
        None,
    )
    .await
    .expect_err("should reject invalid workcenter");

    let msg = format!("{}", err);
    assert!(
        msg.contains("not found or inactive"),
        "Error: {}",
        msg
    );

    // Now add with valid workcenter
    let wc_id = create_test_workcenter(&pool, &tenant).await;

    let step = RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc_id,
            operation_name: "Assemble".to_string(),
            description: Some("Main assembly step".to_string()),
            setup_time_minutes: Some(15),
            run_time_minutes: Some(45),
            is_required: Some(true),
        },
        &corr,
        None,
    )
    .await
    .expect("add step");

    assert_eq!(step.sequence_number, 10);
    assert_eq!(step.workcenter_id, wc_id);
    assert_eq!(step.operation_name, "Assemble");
    assert!(step.is_required);
}

// ============================================================================
// Steps enforce sequence ordering
// ============================================================================

#[tokio::test]
#[serial]
async fn steps_enforce_unique_sequence() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc_id = create_test_workcenter(&pool, &tenant).await;

    let rt = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Sequence Test".to_string(),
            description: None,
            item_id: None,
            bom_revision_id: None,
            revision: None,
            effective_from_date: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create");

    // Add step at sequence 10
    RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc_id,
            operation_name: "First Op".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: None,
        },
        &corr,
        None,
    )
    .await
    .expect("add step 10");

    // Duplicate sequence should fail
    let err = RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc_id,
            operation_name: "Duplicate Op".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: None,
        },
        &corr,
        None,
    )
    .await
    .expect_err("should reject duplicate sequence");

    let msg = format!("{}", err);
    assert!(msg.contains("Duplicate sequence"), "Error: {}", msg);

    // Different sequence should succeed
    RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 20,
            workcenter_id: wc_id,
            operation_name: "Second Op".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: None,
        },
        &corr,
        None,
    )
    .await
    .expect("add step 20");

    // List steps — should be ordered by sequence
    let steps = RoutingRepo::list_steps(&pool, rt.routing_template_id, &tenant)
        .await
        .expect("list steps");

    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].sequence_number, 10);
    assert_eq!(steps[1].sequence_number, 20);
    assert_eq!(steps[0].operation_name, "First Op");
    assert_eq!(steps[1].operation_name, "Second Op");
}

// ============================================================================
// Release routing + immutability
// ============================================================================

#[tokio::test]
#[serial]
async fn release_routing_emits_event_and_blocks_modification() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wc_id = create_test_workcenter(&pool, &tenant).await;

    let rt = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Release Test".to_string(),
            description: None,
            item_id: None,
            bom_revision_id: None,
            revision: None,
            effective_from_date: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create");

    assert_eq!(rt.status, "draft");

    // Release
    let released = RoutingRepo::release(&pool, rt.routing_template_id, &tenant, &corr, None)
        .await
        .expect("release");

    assert_eq!(released.status, "released");

    // Verify release event
    let event_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.routing_released'",
    )
    .bind(rt.routing_template_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(event_count.0, 1);

    // Update should fail on released routing
    let err = RoutingRepo::update(
        &pool,
        rt.routing_template_id,
        &UpdateRoutingRequest {
            tenant_id: tenant.clone(),
            name: Some("Changed Name".to_string()),
            description: None,
            effective_from_date: None,
        },
        &corr,
        None,
    )
    .await
    .expect_err("should reject update on released routing");

    let msg = format!("{}", err);
    assert!(msg.contains("released"), "Error: {}", msg);

    // Adding steps should also fail
    let step_err = RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc_id,
            operation_name: "Should Fail".to_string(),
            description: None,
            setup_time_minutes: None,
            run_time_minutes: None,
            is_required: None,
        },
        &corr,
        None,
    )
    .await
    .expect_err("should reject step on released routing");

    let step_msg = format!("{}", step_err);
    assert!(step_msg.contains("released"), "Error: {}", step_msg);
}

// ============================================================================
// Full workflow: create workcenter → create routing → add ops → release
// ============================================================================

#[tokio::test]
#[serial]
async fn full_routing_workflow() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let item_id = Uuid::new_v4();

    // Create two workcenters
    let wc1 = create_test_workcenter(&pool, &tenant).await;
    let wc2 = create_test_workcenter(&pool, &tenant).await;

    // Create routing
    let rt = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Full Workflow Routing".to_string(),
            description: Some("End-to-end test".to_string()),
            item_id: Some(item_id),
            bom_revision_id: None,
            revision: Some("B".to_string()),
            effective_from_date: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
        },
        &corr,
        None,
    )
    .await
    .expect("create routing");

    // Add operations in order
    RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 10,
            workcenter_id: wc1,
            operation_name: "Cut".to_string(),
            description: Some("Cut raw material".to_string()),
            setup_time_minutes: Some(10),
            run_time_minutes: Some(30),
            is_required: Some(true),
        },
        &corr,
        None,
    )
    .await
    .expect("add cut");

    RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 20,
            workcenter_id: wc2,
            operation_name: "Assemble".to_string(),
            description: Some("Final assembly".to_string()),
            setup_time_minutes: Some(5),
            run_time_minutes: Some(60),
            is_required: Some(true),
        },
        &corr,
        None,
    )
    .await
    .expect("add assemble");

    RoutingRepo::add_step(
        &pool,
        rt.routing_template_id,
        &AddRoutingStepRequest {
            tenant_id: tenant.clone(),
            sequence_number: 30,
            workcenter_id: wc1,
            operation_name: "Clean".to_string(),
            description: Some("Optional cleaning".to_string()),
            setup_time_minutes: None,
            run_time_minutes: Some(15),
            is_required: Some(false),
        },
        &corr,
        None,
    )
    .await
    .expect("add clean");

    // Verify steps
    let steps = RoutingRepo::list_steps(&pool, rt.routing_template_id, &tenant)
        .await
        .expect("list steps");

    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0].operation_name, "Cut");
    assert_eq!(steps[1].operation_name, "Assemble");
    assert_eq!(steps[2].operation_name, "Clean");
    assert!(steps[0].is_required);
    assert!(steps[1].is_required);
    assert!(!steps[2].is_required);

    // Release
    let released = RoutingRepo::release(&pool, rt.routing_template_id, &tenant, &corr, None)
        .await
        .expect("release");

    assert_eq!(released.status, "released");

    // Query by item+date
    let found = RoutingRepo::find_by_item_and_date(
        &pool,
        &tenant,
        item_id,
        NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
    )
    .await
    .expect("find by item");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].routing_template_id, rt.routing_template_id);
}
