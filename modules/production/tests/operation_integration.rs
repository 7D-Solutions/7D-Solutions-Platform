use production_rs::domain::operations::{OperationError, OperationRepo};
use production_rs::domain::routings::{AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo};
use production_rs::domain::work_orders::{CreateWorkOrderRequest, WorkOrderRepo};
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
            code: format!("WC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Test Workcenter".to_string(),
            description: None,
            capacity: Some(10),
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create workcenter");
    wc.workcenter_id
}

/// Helper: create a released routing with N steps, then a released work order linked to it.
async fn setup_wo_with_routing(
    pool: &sqlx::PgPool,
    tenant: &str,
    steps: &[(i32, &str, bool)], // (seq, name, is_required)
) -> (Uuid, Uuid) {
    let corr = Uuid::new_v4().to_string();
    let wc_id = create_test_workcenter(pool, tenant).await;

    // Create routing
    let rt = RoutingRepo::create(
        pool,
        &CreateRoutingRequest {
            tenant_id: tenant.to_string(),
            name: format!("Routing-{}", &Uuid::new_v4().to_string()[..8]),
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

    // Add steps
    for (seq, name, required) in steps {
        RoutingRepo::add_step(
            pool,
            rt.routing_template_id,
            &AddRoutingStepRequest {
                tenant_id: tenant.to_string(),
                sequence_number: *seq,
                workcenter_id: wc_id,
                operation_name: name.to_string(),
                description: None,
                setup_time_minutes: None,
                run_time_minutes: None,
                is_required: Some(*required),
                idempotency_key: None,
            },
            &corr,
            None,
        )
        .await
        .expect("add step");
    }

    // Release routing
    RoutingRepo::release(pool, rt.routing_template_id, tenant, &corr, None)
        .await
        .expect("release routing");

    // Create work order linked to routing
    let wo = WorkOrderRepo::create(
        pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.to_string(),
            order_number: format!("WO-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: Uuid::new_v4(),
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: Some(rt.routing_template_id),
            planned_quantity: 10,
            planned_start: None,
            planned_end: None,
            correlation_id: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create work order");

    // Release work order
    WorkOrderRepo::release(pool, wo.work_order_id, tenant, &corr, None)
        .await
        .expect("release work order");

    (wo.work_order_id, rt.routing_template_id)
}

// ============================================================================
// Initialize operations from routing
// ============================================================================

#[tokio::test]
#[serial]
async fn initialize_creates_operations_from_routing_steps() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(
        &pool,
        &tenant,
        &[(10, "Cut", true), (20, "Weld", true), (30, "Paint", false)],
    )
    .await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");

    assert_eq!(ops.len(), 3);
    assert_eq!(ops[0].sequence_number, 10);
    assert_eq!(ops[0].operation_name, "Cut");
    assert_eq!(ops[0].status, "pending");
    assert_eq!(ops[1].sequence_number, 20);
    assert_eq!(ops[1].operation_name, "Weld");
    assert_eq!(ops[2].sequence_number, 30);
    assert_eq!(ops[2].operation_name, "Paint");

    // All should have routing_step_id set
    for op in &ops {
        assert!(op.routing_step_id.is_some());
        assert_eq!(op.work_order_id, wo_id);
        assert_eq!(op.tenant_id, tenant);
    }
}

// ============================================================================
// Double initialize rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn initialize_rejects_double_initialization() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(&pool, &tenant, &[(10, "Op1", true)]).await;

    OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("first init");

    let err = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect_err("second init should fail");

    assert!(
        matches!(err, OperationError::AlreadyInitialized),
        "Expected AlreadyInitialized, got: {:?}",
        err
    );
}

// ============================================================================
// Start + complete happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn start_and_complete_operation() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(&pool, &tenant, &[(10, "Drill", true)]).await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");

    let op_id = ops[0].operation_id;

    // Start
    let started = OperationRepo::start(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("start");
    assert_eq!(started.status, "in_progress");
    assert!(started.started_at.is_some());

    // Complete
    let completed = OperationRepo::complete(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("complete");
    assert_eq!(completed.status, "completed");
    assert!(completed.completed_at.is_some());
}

// ============================================================================
// Events emitted for start and complete
// ============================================================================

#[tokio::test]
#[serial]
async fn operation_events_emitted() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(&pool, &tenant, &[(10, "Inspect", true)]).await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");
    let op_id = ops[0].operation_id;

    OperationRepo::start(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("start");
    OperationRepo::complete(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("complete");

    let events = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(op_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch events");

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "production.operation_started",
            "production.operation_completed",
        ]
    );
}

// ============================================================================
// Ordering enforcement: cannot start op if required predecessor incomplete
// ============================================================================

#[tokio::test]
#[serial]
async fn ordering_enforced_on_required_predecessors() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(
        &pool,
        &tenant,
        &[(10, "First", true), (20, "Second", true)],
    )
    .await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");

    let first_id = ops[0].operation_id;
    let second_id = ops[1].operation_id;

    // Try to start second before first is complete — should fail
    let err = OperationRepo::start(&pool, wo_id, second_id, &tenant, &corr, None)
        .await
        .expect_err("should block on predecessor");

    match err {
        OperationError::PredecessorNotComplete(seq) => {
            assert_eq!(seq, 10);
        }
        other => panic!("Expected PredecessorNotComplete, got: {:?}", other),
    }

    // Complete first, then second should work
    OperationRepo::start(&pool, wo_id, first_id, &tenant, &corr, None)
        .await
        .expect("start first");
    OperationRepo::complete(&pool, wo_id, first_id, &tenant, &corr, None)
        .await
        .expect("complete first");

    OperationRepo::start(&pool, wo_id, second_id, &tenant, &corr, None)
        .await
        .expect("start second should now succeed");
}

// ============================================================================
// Optional (non-required) predecessor can be skipped
// ============================================================================

#[tokio::test]
#[serial]
async fn optional_predecessor_can_be_skipped() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(
        &pool,
        &tenant,
        &[
            (10, "Required Op", true),
            (20, "Optional Op", false),
            (30, "Final Op", true),
        ],
    )
    .await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");

    let first_id = ops[0].operation_id;
    let final_id = ops[2].operation_id;

    // Complete the required first op
    OperationRepo::start(&pool, wo_id, first_id, &tenant, &corr, None)
        .await
        .expect("start first");
    OperationRepo::complete(&pool, wo_id, first_id, &tenant, &corr, None)
        .await
        .expect("complete first");

    // Skip optional op (seq 20), go straight to final (seq 30)
    OperationRepo::start(&pool, wo_id, final_id, &tenant, &corr, None)
        .await
        .expect("start final — optional predecessor should not block");
}

// ============================================================================
// Invalid transitions rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_complete_pending_operation() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(&pool, &tenant, &[(10, "Op", true)]).await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");

    // Try to complete without starting
    let err = OperationRepo::complete(&pool, wo_id, ops[0].operation_id, &tenant, &corr, None)
        .await
        .expect_err("should reject pending→completed");

    match err {
        OperationError::InvalidTransition { from, to } => {
            assert_eq!(from, "pending");
            assert_eq!(to, "completed");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

#[tokio::test]
#[serial]
async fn cannot_start_already_started_operation() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(&pool, &tenant, &[(10, "Op", true)]).await;

    let ops = OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");
    let op_id = ops[0].operation_id;

    OperationRepo::start(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("start");

    let err = OperationRepo::start(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect_err("should reject in_progress→in_progress");

    match err {
        OperationError::InvalidTransition { from, to } => {
            assert_eq!(from, "in_progress");
            assert_eq!(to, "in_progress");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Initialize requires released work order
// ============================================================================

#[tokio::test]
#[serial]
async fn initialize_rejects_draft_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create a draft WO (not released)
    let wo = WorkOrderRepo::create(
        &pool,
        &CreateWorkOrderRequest {
            tenant_id: tenant.clone(),
            order_number: format!("WO-{}", &Uuid::new_v4().to_string()[..8]),
            item_id: Uuid::new_v4(),
            bom_revision_id: Uuid::new_v4(),
            routing_template_id: Some(Uuid::new_v4()),
            planned_quantity: 5,
            planned_start: None,
            planned_end: None,
            correlation_id: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create draft WO");

    let err = OperationRepo::initialize(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject draft WO");

    assert!(
        matches!(err, OperationError::WorkOrderNotReleased),
        "Expected WorkOrderNotReleased, got: {:?}",
        err
    );
}

// ============================================================================
// List operations returns ordered by sequence
// ============================================================================

#[tokio::test]
#[serial]
async fn list_operations_ordered_by_sequence() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _) = setup_wo_with_routing(
        &pool,
        &tenant,
        &[(30, "Third", true), (10, "First", true), (20, "Second", true)],
    )
    .await;

    OperationRepo::initialize(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("initialize");

    let ops = OperationRepo::list(&pool, wo_id, &tenant)
        .await
        .expect("list");

    assert_eq!(ops.len(), 3);
    assert_eq!(ops[0].sequence_number, 10);
    assert_eq!(ops[1].sequence_number, 20);
    assert_eq!(ops[2].sequence_number, 30);
}
