use production_rs::consumers::item_issued::handle_item_issued;
use production_rs::consumers::time_entry_approved::handle_time_entry_approved;
use production_rs::domain::cost_tracking::{
    CostRepo, CostTrackingError, PostCostRequest, PostingCategory,
};
use production_rs::domain::operations::OperationRepo;
use production_rs::domain::routings::{AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo};
use production_rs::domain::time_entries::{
    ApproveTimeEntryRequest, ManualEntryRequest, TimeEntryRepo,
};
use production_rs::domain::work_orders::{CreateWorkOrderRequest, WorkOrderRepo};
use production_rs::domain::workcenters::{CreateWorkcenterRequest, WorkcenterRepo};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ============================================================================
// DB helpers
// ============================================================================

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://production_user:production_pass@localhost:5461/production_db?sslmode=require"
            .to_string()
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
        .expect("Failed to run migrations");

    pool
}

fn tid() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

/// Create released WO with one operation.  Returns (wo_id, op_id, workcenter_id).
async fn setup_wo_with_op(
    pool: &sqlx::PgPool,
    tenant: &str,
    cost_rate_minor: Option<i64>,
) -> (Uuid, Uuid, Uuid) {
    let corr = Uuid::new_v4().to_string();

    let wc = WorkcenterRepo::create(
        pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.to_string(),
            code: format!("WC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Test WC".to_string(),
            description: None,
            capacity: Some(1),
            cost_rate_minor,
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
            operation_name: "Op".to_string(),
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

    (wo.work_order_id, ops[0].operation_id, wc.workcenter_id)
}

// ============================================================================
// Scenario 1: Labor posting from time_entry_approved — rate=6000, 30 min → 3000 cents
// ============================================================================

#[tokio::test]
#[serial]
async fn labor_cost_from_time_entry_approved_30_min_6000_rate() {
    let pool = setup_db().await;
    let tenant = tid();
    let corr = Uuid::new_v4().to_string();

    // cost_rate_minor=6000 means $60/hr
    let (wo_id, op_id, _) = setup_wo_with_op(&pool, &tenant, Some(6000)).await;

    // Create and stop a time entry, then approve it
    let entry = TimeEntryRepo::manual_entry(
        &pool,
        &ManualEntryRequest {
            work_order_id: wo_id,
            operation_id: Some(op_id),
            actor_id: "operator-1".to_string(),
            start_ts: chrono::Utc::now() - chrono::Duration::minutes(30),
            end_ts: chrono::Utc::now(),
            minutes: 30,
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("create entry");

    TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "supervisor".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("approve");

    // Now simulate consumer logic inline (consumer calls handle_time_entry_approved)
    // Build the payload matching what the event carries
    let event_id = Uuid::new_v4();
    let payload = production_rs::consumers::time_entry_approved::TimeEntryApprovedPayloadTest {
        time_entry_id: entry.time_entry_id,
        work_order_id: wo_id,
        operation_id: Some(op_id),
        tenant_id: tenant.clone(),
        minutes: 30,
        approved_by: "supervisor".to_string(),
        approved_at: chrono::Utc::now(),
    };
    handle_time_entry_approved(&pool, event_id, &payload)
        .await
        .expect("handle");

    // Verify: 30 min * $60/hr = $30 = 3000 cents
    let postings = CostRepo::list_postings(&pool, wo_id, &tenant)
        .await
        .expect("list");
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].posting_category, "labor");
    assert_eq!(postings[0].amount_cents, 3000);

    let summary = CostRepo::get_summary(&pool, wo_id, &tenant)
        .await
        .expect("get")
        .expect("should exist");
    assert_eq!(summary.labor_cost_cents, 3000);
    assert_eq!(summary.total_cost_cents, 3000);
}

// ============================================================================
// Scenario 2: No labor posting when workcenter cost_rate_minor is NULL
// ============================================================================

#[tokio::test]
#[serial]
async fn labor_cost_skipped_when_workcenter_rate_is_null() {
    let pool = setup_db().await;
    let tenant = tid();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id, _) = setup_wo_with_op(&pool, &tenant, None).await;

    let entry = TimeEntryRepo::manual_entry(
        &pool,
        &ManualEntryRequest {
            work_order_id: wo_id,
            operation_id: Some(op_id),
            actor_id: "operator-2".to_string(),
            start_ts: chrono::Utc::now() - chrono::Duration::minutes(60),
            end_ts: chrono::Utc::now(),
            minutes: 60,
            notes: None,
            idempotency_key: None,
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("create entry");

    TimeEntryRepo::approve_time_entry(
        &pool,
        entry.time_entry_id,
        &ApproveTimeEntryRequest {
            approved_by: "sup".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("approve");

    let event_id = Uuid::new_v4();
    let payload = production_rs::consumers::time_entry_approved::TimeEntryApprovedPayloadTest {
        time_entry_id: entry.time_entry_id,
        work_order_id: wo_id,
        operation_id: Some(op_id),
        tenant_id: tenant.clone(),
        minutes: 60,
        approved_by: "sup".to_string(),
        approved_at: chrono::Utc::now(),
    };
    handle_time_entry_approved(&pool, event_id, &payload)
        .await
        .expect("should not error — just skip");

    // No postings created
    let postings = CostRepo::list_postings(&pool, wo_id, &tenant)
        .await
        .expect("list");
    assert!(
        postings.is_empty(),
        "Expected no postings for null rate, got: {:?}",
        postings.len()
    );
}

// ============================================================================
// Scenario 3: Material posting from inventory.item_issued
// ============================================================================

#[tokio::test]
#[serial]
async fn material_cost_from_item_issued_1250() {
    let pool = setup_db().await;
    let tenant = tid();

    let (wo_id, _, _) = setup_wo_with_op(&pool, &tenant, Some(5000)).await;

    let event_id = Uuid::new_v4();
    let payload = production_rs::consumers::item_issued::ItemIssuedPayloadTest {
        issue_line_id: Uuid::new_v4(),
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        sku: "SKU-TEST".to_string(),
        warehouse_id: Uuid::new_v4(),
        quantity: 5,
        total_cost_minor: 1250,
        currency: "USD".to_string(),
        work_order_id: Some(wo_id),
        operation_id: None,
        issued_at: chrono::Utc::now(),
    };

    handle_item_issued(&pool, event_id, &payload)
        .await
        .expect("handle item_issued");

    let postings = CostRepo::list_postings(&pool, wo_id, &tenant)
        .await
        .expect("list");
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].posting_category, "material");
    assert_eq!(postings[0].amount_cents, 1250);

    let summary = CostRepo::get_summary(&pool, wo_id, &tenant)
        .await
        .expect("get")
        .expect("should exist");
    assert_eq!(summary.material_cost_cents, 1250);
    assert_eq!(summary.total_cost_cents, 1250);
}

// ============================================================================
// Scenario 5: Atomicity — duplicate source_event_id returns error, only 1 row committed
// ============================================================================

#[tokio::test]
#[serial]
async fn duplicate_source_event_idempotency() {
    let pool = setup_db().await;
    let tenant = tid();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _, _) = setup_wo_with_op(&pool, &tenant, Some(5000)).await;

    let source_event = Uuid::new_v4();
    let req = PostCostRequest {
        work_order_id: wo_id,
        operation_id: None,
        posting_category: PostingCategory::Material,
        amount_cents: 500,
        quantity: None,
        source_event_id: Some(source_event),
        posted_by: "test".to_string(),
    };

    CostRepo::post_cost(&pool, &req, &tenant, &corr, None)
        .await
        .expect("first post");

    let err = CostRepo::post_cost(&pool, &req, &tenant, &corr, None)
        .await
        .expect_err("duplicate should fail");

    assert!(
        matches!(err, CostTrackingError::DuplicateSourceEvent),
        "Expected DuplicateSourceEvent, got: {:?}",
        err
    );

    // Exactly one posting — second was rolled back
    let postings = CostRepo::list_postings(&pool, wo_id, &tenant)
        .await
        .expect("list");
    assert_eq!(postings.len(), 1, "duplicate post must not commit");

    // Summary still reflects exactly one posting
    let summary = CostRepo::get_summary(&pool, wo_id, &tenant)
        .await
        .expect("get")
        .expect("should exist");
    assert_eq!(summary.total_cost_cents, 500);
    assert_eq!(summary.posting_count, 1);
}

// ============================================================================
// Scenario 6: After 3 postings, total = sum of individual amounts
// ============================================================================

#[tokio::test]
#[serial]
async fn summary_total_equals_sum_of_postings() {
    let pool = setup_db().await;
    let tenant = tid();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id, _) = setup_wo_with_op(&pool, &tenant, Some(5000)).await;

    let amounts = [1000i64, 2500, 750];
    let categories = [
        PostingCategory::Labor,
        PostingCategory::Material,
        PostingCategory::OutsideProcessing,
    ];

    for (amount, cat) in amounts.iter().zip(categories.iter()) {
        CostRepo::post_cost(
            &pool,
            &PostCostRequest {
                work_order_id: wo_id,
                operation_id: Some(op_id),
                posting_category: *cat,
                amount_cents: *amount,
                quantity: None,
                source_event_id: None,
                posted_by: "test".to_string(),
            },
            &tenant,
            &corr,
            None,
        )
        .await
        .expect("post cost");
    }

    let summary = CostRepo::get_summary(&pool, wo_id, &tenant)
        .await
        .expect("get")
        .expect("should exist");

    let db_sum: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(amount_cents), 0) AS BIGINT) FROM work_order_cost_postings WHERE work_order_id = $1 AND tenant_id = $2",
    )
    .bind(wo_id)
    .bind(&tenant)
    .fetch_one(&pool)
    .await
    .expect("sum query");

    assert_eq!(
        summary.total_cost_cents, db_sum,
        "summary total must equal sum of postings"
    );
    assert_eq!(summary.total_cost_cents, amounts.iter().sum::<i64>());
    assert_eq!(summary.labor_cost_cents, 1000);
    assert_eq!(summary.material_cost_cents, 2500);
    assert_eq!(summary.osp_cost_cents, 750);
    assert_eq!(summary.posting_count, 3);
}

// ============================================================================
// Scenario 7: WO close emits work_order_cost_finalized with category breakdown
// ============================================================================

#[tokio::test]
#[serial]
async fn wo_close_emits_cost_finalized_event() {
    let pool = setup_db().await;
    let tenant = tid();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, _, _) = setup_wo_with_op(&pool, &tenant, Some(5000)).await;

    // Post some cost first
    CostRepo::post_cost(
        &pool,
        &PostCostRequest {
            work_order_id: wo_id,
            operation_id: None,
            posting_category: PostingCategory::Labor,
            amount_cents: 4500,
            quantity: None,
            source_event_id: None,
            posted_by: "test".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("post labor");

    WorkOrderRepo::close(&pool, wo_id, &tenant, &corr, None)
        .await
        .expect("close WO");

    // Check outbox for cost_finalized event
    let events = sqlx::query_as::<_, (String, serde_json::Value)>(
        "SELECT event_type, payload FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(wo_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch events");

    let finalized_event = events
        .iter()
        .find(|(et, _)| et == "production.work_order_cost_finalized");

    assert!(
        finalized_event.is_some(),
        "Expected production.work_order_cost_finalized in outbox"
    );

    // Verify payload has correct cost data
    let envelope = &finalized_event.unwrap().1;
    let payload = &envelope["payload"];
    assert_eq!(payload["total_cost_cents"].as_i64().unwrap(), 4500);
    assert_eq!(payload["labor_cost_cents"].as_i64().unwrap(), 4500);
}

// ============================================================================
// Scenario 8: Manual post_cost creates posting + updates summary + emits event
// ============================================================================

#[tokio::test]
#[serial]
async fn manual_post_cost_creates_row_and_updates_summary() {
    let pool = setup_db().await;
    let tenant = tid();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id, _) = setup_wo_with_op(&pool, &tenant, Some(5000)).await;

    let posting = CostRepo::post_cost(
        &pool,
        &PostCostRequest {
            work_order_id: wo_id,
            operation_id: Some(op_id),
            posting_category: PostingCategory::Scrap,
            amount_cents: 800,
            quantity: Some(2.0),
            source_event_id: None,
            posted_by: "user-123".to_string(),
        },
        &tenant,
        &corr,
        None,
    )
    .await
    .expect("post cost");

    assert_eq!(posting.amount_cents, 800);
    assert_eq!(posting.posting_category, "scrap");
    assert_eq!(posting.posted_by, "user-123");

    let summary = CostRepo::get_summary(&pool, wo_id, &tenant)
        .await
        .expect("get")
        .expect("summary must exist after posting");
    assert_eq!(summary.scrap_cost_cents, 800);
    assert_eq!(summary.total_cost_cents, 800);
    assert_eq!(summary.posting_count, 1);

    // Verify cost_posted event in outbox
    let event_types: Vec<String> =
        sqlx::query_scalar("SELECT event_type FROM production_outbox WHERE aggregate_id = $1")
            .bind(posting.posting_id.to_string())
            .fetch_all(&pool)
            .await
            .expect("fetch events");

    assert!(
        event_types.iter().any(|et| et == "production.cost_posted"),
        "Expected production.cost_posted in outbox"
    );
}

// ============================================================================
// Scenario 9: Tenant isolation — tenant A postings not visible to tenant B
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_isolation_cost_postings() {
    let pool = setup_db().await;
    let tenant_a = tid();
    let tenant_b = tid();
    let corr = Uuid::new_v4().to_string();

    let (wo_a, _, _) = setup_wo_with_op(&pool, &tenant_a, Some(5000)).await;

    CostRepo::post_cost(
        &pool,
        &PostCostRequest {
            work_order_id: wo_a,
            operation_id: None,
            posting_category: PostingCategory::Labor,
            amount_cents: 1200,
            quantity: None,
            source_event_id: None,
            posted_by: "user-a".to_string(),
        },
        &tenant_a,
        &corr,
        None,
    )
    .await
    .expect("post for tenant A");

    // Tenant B queries against tenant A's WO — must see nothing
    let postings_b = CostRepo::list_postings(&pool, wo_a, &tenant_b)
        .await
        .expect("list for B");
    assert!(
        postings_b.is_empty(),
        "Tenant B must not see tenant A cost postings"
    );

    let summary_b = CostRepo::get_summary(&pool, wo_a, &tenant_b)
        .await
        .expect("get summary for B");
    assert!(
        summary_b.is_none(),
        "Tenant B must not see tenant A summary"
    );
}
