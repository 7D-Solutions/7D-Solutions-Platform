use axum::{
    body::Body,
    extract::Request as AxumRequest,
    middleware::{self, Next},
    response::Response,
    Router,
};
use http_body_util::BodyExt;
use production_rs::domain::bom_client::BomRevisionClient;
use production_rs::domain::numbering_client::NumberingClient;
use production_rs::domain::operations::OperationRepo;
use production_rs::domain::routings::{AddRoutingStepRequest, CreateRoutingRequest, RoutingRepo};
use production_rs::domain::work_orders::{
    CompositeCreateWorkOrderRequest, CreateWorkOrderRequest, DerivedStatus, WorkOrderError,
    WorkOrderRepo,
};
use production_rs::domain::workcenters::{CreateWorkcenterRequest, WorkcenterRepo};
use production_rs::metrics::ProductionMetrics;
use production_rs::AppState;
use security::{ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
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

async fn setup_numbering_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("NUMBERING_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://numbering_user:numbering_pass@localhost:5456/numbering_db".to_string()
    });

    PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to numbering test DB")
}

async fn setup_bom_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("BOM_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://bom_user:bom_pass@localhost:5450/bom_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to BOM test DB");

    sqlx::migrate!("../bom/db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run BOM migrations");

    pool
}

fn make_test_claims(tenant_id: &str) -> VerifiedClaims {
    use chrono::Duration;
    VerifiedClaims {
        user_id: Uuid::new_v4(),
        tenant_id: Uuid::parse_str(tenant_id)
            .unwrap_or_else(|_| Uuid::new_v4()),
        app_id: None,
        roles: vec!["admin".to_string()],
        perms: vec![
            "production.read".to_string(),
            "production.mutate".to_string(),
        ],
        actor_type: ActorType::User,
        issued_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + Duration::hours(1),
        token_id: Uuid::new_v4(),
        version: "1".to_string(),
    }
}

fn unique_tenant() -> String {
    format!("test-tenant-{}", Uuid::new_v4())
}

fn wo_request(tenant: &str, order_num: &str) -> CreateWorkOrderRequest {
    CreateWorkOrderRequest {
        tenant_id: tenant.to_string(),
        order_number: order_num.to_string(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Uuid::new_v4(),
        routing_template_id: None,
        planned_quantity: 10,
        planned_start: None,
        planned_end: None,
        correlation_id: None,
    }
}

// ============================================================================
// Full lifecycle: draft → released → closed
// ============================================================================

#[tokio::test]
#[serial]
async fn work_order_full_lifecycle() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    // Create (draft)
    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-001"), &corr, None)
        .await
        .expect("create");
    assert_eq!(wo.status, "draft");
    assert!(wo.actual_start.is_none());
    assert!(wo.actual_end.is_none());

    // Release
    let released =
        WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
            .await
            .expect("release");
    assert_eq!(released.status, "released");
    assert!(released.actual_start.is_some());
    assert!(released.actual_end.is_none());

    // Close
    let closed =
        WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
            .await
            .expect("close");
    assert_eq!(closed.status, "closed");
    assert!(closed.actual_end.is_some());
}

// ============================================================================
// Events emitted for each transition
// ============================================================================

#[tokio::test]
#[serial]
async fn work_order_events_emitted_for_each_transition() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-EVT"), &corr, None)
        .await
        .expect("create");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release");

    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close");

    let events = sqlx::query_as::<_, (String,)>(
        "SELECT event_type FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(wo.work_order_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch events");

    let types: Vec<&str> = events.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(
        types,
        vec![
            "production.work_order_created",
            "production.work_order_released",
            "production.work_order_closed",
        ]
    );
}

// ============================================================================
// Illegal transition: draft → closed rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_close_draft_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DRAFT-CLOSE"), &corr, None)
        .await
        .expect("create");

    let err = WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject draft→closed");

    match err {
        WorkOrderError::InvalidTransition { from, to } => {
            assert_eq!(from, "draft");
            assert_eq!(to, "closed");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Illegal transition: released → released rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_release_already_released_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DBL-REL"), &corr, None)
        .await
        .expect("create");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("first release");

    let err = WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject released→released");

    match err {
        WorkOrderError::InvalidTransition { from, to } => {
            assert_eq!(from, "released");
            assert_eq!(to, "released");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Illegal transition: closed → released rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn cannot_release_closed_work_order() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-CLOSED-REL"), &corr, None)
        .await
        .expect("create");

    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release");

    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close");

    let err = WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect_err("should reject closed→released");

    match err {
        WorkOrderError::InvalidTransition { from, to } => {
            assert_eq!(from, "closed");
            assert_eq!(to, "released");
        }
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

// ============================================================================
// Correlation chain: same correlation_id across all events
// ============================================================================

#[tokio::test]
#[serial]
async fn correlation_id_chains_across_events() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-CORR"), &corr, None)
        .await
        .expect("create");

    // Use same correlation_id for the full lifecycle to simulate a single business flow
    WorkOrderRepo::release(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("release");
    WorkOrderRepo::close(&pool, wo.work_order_id, &tenant, &corr, None)
        .await
        .expect("close");

    // Verify all outbox rows for this WO carry the same correlation_id
    let rows = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT event_type, correlation_id FROM production_outbox WHERE aggregate_id = $1 ORDER BY created_at",
    )
    .bind(wo.work_order_id.to_string())
    .fetch_all(&pool)
    .await
    .expect("fetch outbox");

    assert_eq!(rows.len(), 3);
    for (event_type, row_corr) in &rows {
        assert_eq!(
            row_corr.as_deref(),
            Some(corr.as_str()),
            "Event {} should carry correlation_id",
            event_type
        );
    }
}

// ============================================================================
// Duplicate correlation_id returns existing WO (idempotency)
// ============================================================================

#[tokio::test]
#[serial]
async fn duplicate_correlation_id_returns_existing_wo() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();
    let dedup_corr = Uuid::new_v4().to_string();

    let mut req = wo_request(&tenant, "WO-IDEM");
    req.correlation_id = Some(dedup_corr.clone());

    let first = WorkOrderRepo::create(&pool, &req, &corr, None)
        .await
        .expect("first create");

    // Second create with same correlation_id: different order number, should return first
    let mut req2 = wo_request(&tenant, "WO-IDEM-2");
    req2.correlation_id = Some(dedup_corr);

    let second = WorkOrderRepo::create(&pool, &req2, &corr, None)
        .await
        .expect("second create should succeed via idempotency");

    assert_eq!(first.work_order_id, second.work_order_id);
    assert_eq!(second.order_number, "WO-IDEM"); // original order number

    // Only one outbox event should exist (from first create)
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM production_outbox WHERE aggregate_id = $1 AND event_type = 'production.work_order_created'",
    )
    .bind(first.work_order_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count.0, 1, "Duplicate request should not produce extra events");
}

// ============================================================================
// Duplicate order number rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn duplicate_order_number_rejected() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DUP"), &corr, None)
        .await
        .expect("first create");

    let err = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DUP"), &corr, None)
        .await
        .expect_err("should reject duplicate order number");

    let msg = format!("{}", err);
    assert!(msg.contains("WO-DUP"), "Error should mention order number: {}", msg);
}

// ============================================================================
// Validation: planned_quantity must be > 0
// ============================================================================

#[tokio::test]
#[serial]
async fn create_rejects_zero_quantity() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let mut req = wo_request(&tenant, "WO-ZERO");
    req.planned_quantity = 0;

    let err = WorkOrderRepo::create(&pool, &req, &corr, None)
        .await
        .expect_err("should reject zero quantity");

    match err {
        WorkOrderError::Validation(msg) => {
            assert!(msg.contains("planned_quantity"), "msg: {}", msg);
        }
        other => panic!("Expected Validation, got: {:?}", other),
    }
}

// ============================================================================
// Helpers for derived_status tests (require routing + operations setup)
// ============================================================================

async fn create_test_workcenter_for_wo(pool: &sqlx::PgPool, tenant: &str) -> Uuid {
    let corr = Uuid::new_v4().to_string();
    WorkcenterRepo::create(
        pool,
        &CreateWorkcenterRequest {
            tenant_id: tenant.to_string(),
            code: format!("WC-{}", &Uuid::new_v4().to_string()[..8]),
            name: "Test Workcenter".to_string(),
            description: None,
            capacity: Some(8),
            cost_rate_minor: None,
            idempotency_key: None,
        },
        &corr,
        None,
    )
    .await
    .expect("create workcenter")
    .workcenter_id
}

/// Create a released WO with a one-step routing; return (wo_id, operation_id after initialize).
async fn setup_released_wo_with_one_op(
    pool: &sqlx::PgPool,
    tenant: &str,
) -> (Uuid, Uuid) {
    let corr = Uuid::new_v4().to_string();
    let wc_id = create_test_workcenter_for_wo(pool, tenant).await;

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
            workcenter_id: wc_id,
            operation_name: "Assemble".to_string(),
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
    .expect("create wo");

    WorkOrderRepo::release(pool, wo.work_order_id, tenant, &corr, None)
        .await
        .expect("release wo");

    let ops = OperationRepo::initialize(pool, wo.work_order_id, tenant, &corr, None)
        .await
        .expect("initialize ops");

    (wo.work_order_id, ops[0].operation_id)
}

// ============================================================================
// derived_status: WO with 0 operations → not_started
// ============================================================================

#[tokio::test]
#[serial]
async fn derived_status_no_operations_is_not_started() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-DS-NONE"), &corr, None)
        .await
        .expect("create");

    let resp = WorkOrderRepo::find_by_id_with_derived(&pool, wo.work_order_id, &tenant)
        .await
        .expect("find_by_id_with_derived")
        .expect("should exist");

    assert_eq!(resp.derived_status, DerivedStatus::NotStarted);
}

// ============================================================================
// derived_status: WO with 1 started operation → in_progress
// ============================================================================

#[tokio::test]
#[serial]
async fn derived_status_with_started_op_is_in_progress() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id) = setup_released_wo_with_one_op(&pool, &tenant).await;

    OperationRepo::start(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("start op");

    let resp = WorkOrderRepo::find_by_id_with_derived(&pool, wo_id, &tenant)
        .await
        .expect("find_by_id_with_derived")
        .expect("should exist");

    assert_eq!(resp.derived_status, DerivedStatus::InProgress);
}

// ============================================================================
// derived_status: WO with all operations completed → complete
// ============================================================================

#[tokio::test]
#[serial]
async fn derived_status_all_ops_complete_is_complete() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let (wo_id, op_id) = setup_released_wo_with_one_op(&pool, &tenant).await;

    OperationRepo::start(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("start op");

    OperationRepo::complete(&pool, wo_id, op_id, &tenant, &corr, None)
        .await
        .expect("complete op");

    let resp = WorkOrderRepo::find_by_id_with_derived(&pool, wo_id, &tenant)
        .await
        .expect("find_by_id_with_derived")
        .expect("should exist");

    assert_eq!(resp.derived_status, DerivedStatus::Complete);
}

// ============================================================================
// list_with_derived: derived_status appears in list response
// ============================================================================

#[tokio::test]
#[serial]
async fn list_with_derived_includes_derived_status() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-LIST-A"), &corr, None)
        .await
        .expect("create A");
    WorkOrderRepo::create(&pool, &wo_request(&tenant, "WO-LIST-B"), &corr, None)
        .await
        .expect("create B");

    let (items, total) = WorkOrderRepo::list_with_derived(&pool, &tenant, 1, 50)
        .await
        .expect("list_with_derived");

    assert_eq!(total, 2);
    assert_eq!(items.len(), 2);
    for item in &items {
        assert_eq!(item.derived_status, DerivedStatus::NotStarted);
    }
}

// ============================================================================
// Helpers for HTTP-level batch tests
// ============================================================================

/// Inject VerifiedClaims from X-Tenant-Id header (test-only — no real JWT).
async fn inject_production_claims(req: AxumRequest, next: Next) -> Response {
    use chrono::Duration;
    let tenant_id = req
        .headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok());
    match tenant_id {
        Some(tid) => {
            let claims = VerifiedClaims {
                user_id: Uuid::new_v4(),
                tenant_id: tid,
                app_id: None,
                roles: vec!["admin".to_string()],
                perms: vec![
                    "production.read".to_string(),
                    "production.mutate".to_string(),
                ],
                actor_type: ActorType::User,
                issued_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + Duration::hours(1),
                token_id: Uuid::new_v4(),
                version: "1".to_string(),
            };
            let mut req = req;
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        None => next.run(req).await,
    }
}

fn build_test_app(pool: sqlx::PgPool, numbering: NumberingClient) -> Router {
    build_test_app_with_bom(pool, numbering, BomRevisionClient::permissive())
}

fn build_test_app_with_bom(
    pool: sqlx::PgPool,
    numbering: NumberingClient,
    bom: BomRevisionClient,
) -> Router {
    // Prometheus registers metrics globally — share the instance across tests.
    static METRICS: std::sync::OnceLock<Arc<ProductionMetrics>> = std::sync::OnceLock::new();
    let metrics = METRICS
        .get_or_init(|| Arc::new(ProductionMetrics::new().expect("metrics init")))
        .clone();
    let state = Arc::new(AppState {
        pool,
        metrics,
        numbering: Arc::new(numbering),
        bom: Arc::new(bom),
    });
    production_rs::http::router(state)
        .layer(middleware::from_fn(inject_production_claims))
}

async fn body_json(resp: axum::response::Response<Body>) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ============================================================================
// Batch fetch: domain-level happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_fetch_returns_five_work_orders() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let mut ids = Vec::new();
    for i in 0..5 {
        let wo = WorkOrderRepo::create(
            &pool,
            &wo_request(&tenant, &format!("WO-BATCH-{}", i)),
            &corr,
            None,
        )
        .await
        .expect("create");
        ids.push(wo.work_order_id);
    }

    let result = WorkOrderRepo::fetch_batch(&pool, &ids, &tenant, false, false)
        .await
        .expect("fetch_batch");

    assert_eq!(result.len(), 5, "all 5 WOs returned");
    for wo in &result {
        assert!(ids.contains(&wo.work_order_id), "unexpected id in result");
        assert!(wo.operations.is_none(), "operations not requested");
    }
}

#[tokio::test]
#[serial]
async fn batch_fetch_with_operations_returns_nested_ops() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let (wo_id, _op_id) = setup_released_wo_with_one_op(&pool, &tenant).await;

    let result = WorkOrderRepo::fetch_batch(&pool, &[wo_id], &tenant, true, false)
        .await
        .expect("fetch_batch");

    assert_eq!(result.len(), 1);
    let ops = result[0].operations.as_ref().expect("operations present");
    assert_eq!(ops.len(), 1, "one operation included");
    assert_eq!(ops[0].work_order_id, wo_id);
}

// ============================================================================
// Batch fetch: HTTP-level validation (real routes, no mocks)
// ============================================================================

#[tokio::test]
#[serial]
async fn http_batch_fetch_empty_ids_returns_400() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let app = build_test_app(pool, NumberingClient::direct(num_pool));
    let tenant = Uuid::new_v4();

    let resp = app
        .oneshot(
            axum::http::Request::get(
                "/api/production/work-orders?ids=",
            )
            .header("x-tenant-id", tenant.to_string())
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(resp.status(), 400);
    let body = body_json(resp).await;
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("empty"),
        "body: {}",
        body
    );
}

#[tokio::test]
#[serial]
async fn http_batch_fetch_over_50_ids_returns_400() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let app = build_test_app(pool, NumberingClient::direct(num_pool));
    let tenant = Uuid::new_v4();

    let ids: Vec<String> = (0..51).map(|_| Uuid::new_v4().to_string()).collect();
    let ids_param = ids.join(",");
    let uri = format!("/api/production/work-orders?ids={}", ids_param);

    let resp = app
        .oneshot(
            axum::http::Request::get(uri.as_str())
                .header("x-tenant-id", tenant.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(resp.status(), 400);
    let body = body_json(resp).await;
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("50"),
        "body: {}",
        body
    );
}

#[tokio::test]
#[serial]
async fn http_batch_fetch_five_ids_returns_all() {
    let pool = setup_db().await;
    // Use a UUID-format tenant so it round-trips through VerifiedClaims.tenant_id
    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let corr = Uuid::new_v4().to_string();
    let num_pool = setup_numbering_db().await;
    let app = build_test_app(pool.clone(), NumberingClient::direct(num_pool));

    let mut ids = Vec::new();
    for i in 0..5 {
        let wo = WorkOrderRepo::create(
            &pool,
            &wo_request(&tenant, &format!("WO-HTTP-BATCH-{}", i)),
            &corr,
            None,
        )
        .await
        .expect("create");
        ids.push(wo.work_order_id.to_string());
    }

    let ids_param = ids.join(",");
    let uri = format!("/api/production/work-orders?ids={}", ids_param);

    let resp = app
        .oneshot(
            axum::http::Request::get(uri.as_str())
                .header("x-tenant-id", tenant_uuid.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(resp.status(), 200);
    let body = body_json(resp).await;
    let arr = body.as_array().expect("response is an array");
    assert_eq!(arr.len(), 5, "all 5 WOs returned in HTTP response");
}

// ============================================================================
// Batch fetch: response-time gate for 10 WOs
// ============================================================================

#[tokio::test]
#[serial]
async fn batch_fetch_10_wos_under_200ms() {
    let pool = setup_db().await;
    let tenant = unique_tenant();
    let corr = Uuid::new_v4().to_string();

    let mut ids = Vec::new();
    for i in 0..10 {
        let wo = WorkOrderRepo::create(
            &pool,
            &wo_request(&tenant, &format!("WO-PERF-{}", i)),
            &corr,
            None,
        )
        .await
        .expect("create");
        ids.push(wo.work_order_id);
    }

    let start = std::time::Instant::now();
    let _result = WorkOrderRepo::fetch_batch(&pool, &ids, &tenant, true, false)
        .await
        .expect("fetch_batch");
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 200,
        "fetch_batch for 10 WOs took {}ms (expected < 200ms)",
        elapsed.as_millis()
    );
}

// ============================================================================
// Composite create: allocates WO number and creates WO in one call
// ============================================================================

#[tokio::test]
#[serial]
async fn composite_create_allocates_wo_number() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let numbering = NumberingClient::direct(num_pool);

    // Use a UUID tenant so numbering direct-mode can parse it.
    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let idempotency_key = format!("numbering:wo:{}:1", tenant_uuid);
    let corr = Uuid::new_v4().to_string();

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Some(Uuid::new_v4()),
        routing_template_id: None,
        planned_quantity: 5,
        planned_start: None,
        planned_end: None,
        idempotency_key: idempotency_key.clone(),
    };

    let bom = BomRevisionClient::permissive();
    let wo = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr, None)
        .await
        .expect("composite_create");

    // WO number must be allocated (WO-NNNNN format)
    assert!(
        wo.order_number.starts_with("WO-"),
        "order_number should start with WO-, got: {}",
        wo.order_number
    );
    assert_eq!(wo.status, "draft");
    assert_eq!(wo.tenant_id, tenant);
    assert_eq!(wo.bom_revision_id, req.bom_revision_id);
    assert_eq!(wo.routing_template_id, None);
}

#[tokio::test]
#[serial]
async fn composite_create_without_optional_fields_creates_number_only_wo() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let numbering = NumberingClient::direct(num_pool);

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let corr = Uuid::new_v4().to_string();

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: None,
        routing_template_id: None,
        planned_quantity: 10,
        planned_start: None,
        planned_end: None,
        idempotency_key: format!("numbering:wo:{}:no-bom", tenant_uuid),
    };

    let bom = BomRevisionClient::permissive();
    let wo = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr, None)
        .await
        .expect("composite_create without BOM");

    assert!(wo.order_number.starts_with("WO-"));
    assert_eq!(wo.status, "draft");
    assert_eq!(wo.bom_revision_id, None);
    assert_eq!(wo.routing_template_id, None);
}

#[tokio::test]
#[serial]
async fn composite_create_idempotency_returns_same_number() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let numbering = NumberingClient::direct(num_pool);

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let idem_key = format!("numbering:wo:{}:idem", tenant_uuid);
    let corr1 = Uuid::new_v4().to_string();
    let corr2 = Uuid::new_v4().to_string();

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: None,
        routing_template_id: None,
        planned_quantity: 3,
        planned_start: None,
        planned_end: None,
        idempotency_key: idem_key.clone(),
    };

    let bom = BomRevisionClient::permissive();
    let wo1 = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr1, None)
        .await
        .expect("first composite_create");

    // Second call with same idempotency_key allocates the SAME number.
    // Because the number is already taken, a fresh req with the same idem key
    // gets the same formatted number back from the numbering service, but the
    // DB INSERT fails with duplicate order_number.  The idempotency protection
    // for the WO itself (dedup) is a separate concern — this test verifies
    // that numbering is idempotent (same key → same number).
    let wo2_req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        idempotency_key: idem_key.clone(),
        item_id: Uuid::new_v4(),
        ..req
    };
    let result = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &wo2_req, &claims, &corr2, None)
        .await;

    // Same number → duplicate order_number → either returns same WO or
    // DuplicateOrderNumber error.  Both outcomes are acceptable; what matters
    // is that the allocated number matches.
    match result {
        Ok(wo2) => {
            assert_eq!(wo1.order_number, wo2.order_number, "same idempotency_key → same number");
        }
        Err(production_rs::domain::work_orders::WorkOrderError::DuplicateOrderNumber(num, _)) => {
            assert_eq!(wo1.order_number, num, "duplicate error names the same number");
        }
        Err(e) => panic!("Unexpected error: {:?}", e),
    }
}

#[tokio::test]
#[serial]
async fn composite_create_with_routing_attaches_routing() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let numbering = NumberingClient::direct(num_pool);

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);
    let corr = Uuid::new_v4().to_string();

    // Create a routing to attach.
    let rt = RoutingRepo::create(
        &pool,
        &CreateRoutingRequest {
            tenant_id: tenant.clone(),
            name: "Test Routing".to_string(),
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

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Some(Uuid::new_v4()),
        routing_template_id: Some(rt.routing_template_id),
        planned_quantity: 2,
        planned_start: None,
        planned_end: None,
        idempotency_key: format!("numbering:wo:{}:with-routing", tenant_uuid),
    };

    let bom = BomRevisionClient::permissive();
    let wo = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr, None)
        .await
        .expect("composite_create with routing");

    assert!(wo.order_number.starts_with("WO-"));
    assert_eq!(wo.routing_template_id, Some(rt.routing_template_id));
}

// ============================================================================
// BOM revision validation tests
// ============================================================================

#[tokio::test]
#[serial]
async fn composite_create_accepts_effective_bom_revision() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool.clone());

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);

    // Insert a BOM header and an effective revision directly into the BOM DB.
    let bom_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO bom_headers (id, tenant_id, part_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(bom_id)
    .bind(tenant.clone())
    .bind(Uuid::new_v4())
    .execute(&bom_pool)
    .await
    .expect("insert bom_header");

    sqlx::query(
        "INSERT INTO bom_revisions (id, bom_id, tenant_id, revision_label, status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'effective', now(), now())",
    )
    .bind(revision_id)
    .bind(bom_id)
    .bind(tenant.clone())
    .bind("rev-001")
    .execute(&bom_pool)
    .await
    .expect("insert bom_revision");

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Some(revision_id),
        routing_template_id: None,
        planned_quantity: 1,
        planned_start: None,
        planned_end: None,
        idempotency_key: format!("numbering:wo:{}:bom-effective", tenant_uuid),
    };
    let corr = Uuid::new_v4().to_string();

    let wo = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr, None)
        .await
        .expect("composite_create with effective BOM revision");

    assert!(wo.order_number.starts_with("WO-"));
    assert_eq!(wo.bom_revision_id, Some(revision_id));
}

#[tokio::test]
#[serial]
async fn composite_create_rejects_draft_bom_revision() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool.clone());

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);

    let bom_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO bom_headers (id, tenant_id, part_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(bom_id)
    .bind(tenant.clone())
    .bind(Uuid::new_v4())
    .execute(&bom_pool)
    .await
    .expect("insert bom_header");

    sqlx::query(
        "INSERT INTO bom_revisions (id, bom_id, tenant_id, revision_label, status, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 'draft', now(), now())",
    )
    .bind(revision_id)
    .bind(bom_id)
    .bind(tenant.clone())
    .bind("rev-draft")
    .execute(&bom_pool)
    .await
    .expect("insert draft bom_revision");

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Some(revision_id),
        routing_template_id: None,
        planned_quantity: 1,
        planned_start: None,
        planned_end: None,
        idempotency_key: format!("numbering:wo:{}:bom-draft", tenant_uuid),
    };
    let corr = Uuid::new_v4().to_string();

    let result = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr, None).await;

    match result {
        Err(WorkOrderError::Validation(msg)) => {
            assert!(
                msg.contains("draft"),
                "expected error to mention 'draft', got: {msg}"
            );
        }
        other => panic!("expected Validation error for draft revision, got: {other:?}"),
    }
}

#[tokio::test]
#[serial]
async fn composite_create_rejects_missing_bom_revision() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool);

    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let claims = make_test_claims(&tenant);

    // Use a random UUID that was never inserted — guaranteed not found.
    let missing_revision_id = Uuid::new_v4();

    let req = CompositeCreateWorkOrderRequest {
        tenant_id: tenant.clone(),
        item_id: Uuid::new_v4(),
        bom_revision_id: Some(missing_revision_id),
        routing_template_id: None,
        planned_quantity: 1,
        planned_start: None,
        planned_end: None,
        idempotency_key: format!("numbering:wo:{}:bom-missing", tenant_uuid),
    };
    let corr = Uuid::new_v4().to_string();

    let result = WorkOrderRepo::composite_create(&pool, &numbering, &bom, &req, &claims, &corr, None).await;

    match result {
        Err(WorkOrderError::Validation(msg)) => {
            assert!(
                msg.contains("not found"),
                "expected error to mention 'not found', got: {msg}"
            );
        }
        other => panic!("expected Validation error for missing revision, got: {other:?}"),
    }
}

// ============================================================================
// Composite create: HTTP-level happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn http_composite_create_returns_201_with_allocated_number() {
    let pool = setup_db().await;
    let num_pool = setup_numbering_db().await;
    let app = build_test_app(pool, NumberingClient::direct(num_pool));

    let tenant_uuid = Uuid::new_v4();

    let body = serde_json::json!({
        "item_id": Uuid::new_v4(),
        "bom_revision_id": Uuid::new_v4(),
        "planned_quantity": 4,
        "idempotency_key": format!("numbering:wo:{}:http", tenant_uuid)
    });

    let resp = app
        .oneshot(
            axum::http::Request::post("/api/production/work-orders/create")
                .header("x-tenant-id", tenant_uuid.to_string())
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .expect("request");

    assert_eq!(resp.status(), 201, "expected 201 Created");
    let body = body_json(resp).await;
    assert!(
        body["order_number"]
            .as_str()
            .unwrap_or("")
            .starts_with("WO-"),
        "order_number should start with WO-, got: {}",
        body
    );
    assert_eq!(body["status"].as_str(), Some("draft"));
}
