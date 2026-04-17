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
use production_rs::metrics::ProductionMetrics;
use production_rs::{AppState, OutsideProcessingClient};
use platform_sdk::PlatformClient;
use security::{ActorType, VerifiedClaims};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

// ============================================================================
// DB setup helpers (mirrors work_order_integration.rs)
// ============================================================================

async fn setup_prod_db() -> sqlx::PgPool {
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
    let url = std::env::var("BOM_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bom_user:bom_pass@localhost:5450/bom_db".to_string());

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

// ============================================================================
// HTTP test helpers
// ============================================================================

/// Inject VerifiedClaims from the X-Tenant-Id header (test-only, no real JWT).
async fn inject_claims(req: AxumRequest, next: Next) -> Response {
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

fn build_app(
    prod_pool: sqlx::PgPool,
    numbering: NumberingClient,
    bom: BomRevisionClient,
) -> Router {
    static METRICS: std::sync::OnceLock<Arc<ProductionMetrics>> = std::sync::OnceLock::new();
    let metrics = METRICS
        .get_or_init(|| Arc::new(ProductionMetrics::new().expect("metrics init")))
        .clone();
    let op_client = Arc::new(OutsideProcessingClient::new(
        PlatformClient::new("http://localhost:1".to_string()),
    ));
    let state = Arc::new(AppState {
        pool: prod_pool,
        metrics,
        numbering: Arc::new(numbering),
        bom: Arc::new(bom),
        op_client,
    });
    production_rs::http::router(state).layer(middleware::from_fn(inject_claims))
}

async fn body_json(resp: axum::response::Response<Body>) -> serde_json::Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ============================================================================
// Test fixture: set up superseded rev_a + effective rev_b linked via ECO
// ============================================================================

struct EcoFixture {
    tenant_uuid: Uuid,
    rev_a_id: Uuid, // superseded
    rev_b_id: Uuid, // effective
    eco_number: String,
}

async fn insert_eco_fixture(bom_pool: &sqlx::PgPool) -> EcoFixture {
    let tenant_uuid = Uuid::new_v4();
    let tenant = tenant_uuid.to_string();
    let eco_number = format!("ECO-{}", &Uuid::new_v4().to_string()[..8].to_uppercase());

    let bom_id = Uuid::new_v4();
    let rev_a_id = Uuid::new_v4();
    let rev_b_id = Uuid::new_v4();
    let eco_id = Uuid::new_v4();

    // BOM header
    sqlx::query(
        "INSERT INTO bom_headers (id, tenant_id, part_id, created_at, updated_at) \
         VALUES ($1, $2, $3, now(), now())",
    )
    .bind(bom_id)
    .bind(&tenant)
    .bind(Uuid::new_v4())
    .execute(bom_pool)
    .await
    .expect("insert bom_header");

    // Rev A — superseded (no effectivity to avoid constraint)
    sqlx::query(
        "INSERT INTO bom_revisions \
         (id, bom_id, tenant_id, revision_label, status, created_at, updated_at) \
         VALUES ($1, $2, $3, 'rev-A', 'superseded', now(), now())",
    )
    .bind(rev_a_id)
    .bind(bom_id)
    .bind(&tenant)
    .execute(bom_pool)
    .await
    .expect("insert rev_a");

    // Rev B — effective (no effectivity range: unbounded)
    sqlx::query(
        "INSERT INTO bom_revisions \
         (id, bom_id, tenant_id, revision_label, status, created_at, updated_at) \
         VALUES ($1, $2, $3, 'rev-B', 'effective', now(), now())",
    )
    .bind(rev_b_id)
    .bind(bom_id)
    .bind(&tenant)
    .execute(bom_pool)
    .await
    .expect("insert rev_b");

    // ECO — applied status
    sqlx::query(
        "INSERT INTO ecos \
         (id, tenant_id, eco_number, title, status, created_by, created_at, updated_at) \
         VALUES ($1, $2, $3, 'Rev A → B upgrade', 'applied', 'test-user', now(), now())",
    )
    .bind(eco_id)
    .bind(&tenant)
    .bind(&eco_number)
    .execute(bom_pool)
    .await
    .expect("insert eco");

    // ECO-BOM revision link: before=rev_a, after=rev_b
    sqlx::query(
        "INSERT INTO eco_bom_revisions \
         (eco_id, tenant_id, bom_id, before_revision_id, after_revision_id, created_at) \
         VALUES ($1, $2, $3, $4, $5, now())",
    )
    .bind(eco_id)
    .bind(&tenant)
    .bind(bom_id)
    .bind(rev_a_id)
    .bind(rev_b_id)
    .execute(bom_pool)
    .await
    .expect("insert eco_bom_revisions");

    EcoFixture {
        tenant_uuid,
        rev_a_id,
        rev_b_id,
        eco_number,
    }
}

// ============================================================================
// Tests: composite_create_work_order (POST /api/production/work-orders/create)
// ============================================================================

/// Superseded BOM revision → 422 BOM_REVISION_SUPERSEDED with ECO reference.
#[tokio::test]
#[serial]
async fn eco_enforcement_composite_create_rejects_superseded_rev() {
    let prod_pool = setup_prod_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let fixture = insert_eco_fixture(&bom_pool).await;
    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool);
    let app = build_app(prod_pool, numbering, bom);

    let body = serde_json::json!({
        "item_id": Uuid::new_v4(),
        "bom_revision_id": fixture.rev_a_id,
        "planned_quantity": 1,
        "idempotency_key": format!("eco-test:{}", Uuid::new_v4()),
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/production/work-orders/create")
                .header("content-type", "application/json")
                .header("x-tenant-id", fixture.tenant_uuid.to_string())
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for superseded revision"
    );

    let json = body_json(resp).await;
    assert_eq!(
        json["error"].as_str().unwrap_or(""),
        "BOM_REVISION_SUPERSEDED",
        "error code should be BOM_REVISION_SUPERSEDED, got: {json}"
    );
    let message = json["message"].as_str().unwrap_or("");
    assert!(
        message.contains(&fixture.eco_number),
        "error message should reference ECO number '{}', got: {}",
        fixture.eco_number,
        message
    );
    assert!(
        message.contains(&fixture.rev_b_id.to_string()),
        "error message should reference new revision id '{}', got: {}",
        fixture.rev_b_id,
        message
    );
}

/// Released (effective) BOM revision → 201 WO created.
#[tokio::test]
#[serial]
async fn eco_enforcement_composite_create_accepts_effective_rev() {
    let prod_pool = setup_prod_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let fixture = insert_eco_fixture(&bom_pool).await;
    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool);
    let app = build_app(prod_pool, numbering, bom);

    let body = serde_json::json!({
        "item_id": Uuid::new_v4(),
        "bom_revision_id": fixture.rev_b_id,
        "planned_quantity": 1,
        "idempotency_key": format!("eco-test:{}", Uuid::new_v4()),
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/production/work-orders/create")
                .header("content-type", "application/json")
                .header("x-tenant-id", fixture.tenant_uuid.to_string())
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        axum::http::StatusCode::CREATED,
        "expected 201 for effective revision, body: {}",
        body_json(resp).await
    );
}

// ============================================================================
// Tests: create_work_order (POST /api/production/work-orders)
// ============================================================================

/// Superseded BOM revision → 422 BOM_REVISION_SUPERSEDED via plain create.
#[tokio::test]
#[serial]
async fn eco_enforcement_create_rejects_superseded_rev() {
    let prod_pool = setup_prod_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let fixture = insert_eco_fixture(&bom_pool).await;
    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool);
    let app = build_app(prod_pool, numbering, bom);

    let body = serde_json::json!({
        "order_number": format!("WO-ECO-{}", &Uuid::new_v4().to_string()[..8]),
        "item_id": Uuid::new_v4(),
        "bom_revision_id": fixture.rev_a_id,
        "planned_quantity": 1,
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/production/work-orders")
                .header("content-type", "application/json")
                .header("x-tenant-id", fixture.tenant_uuid.to_string())
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "expected 422 for superseded revision"
    );

    let json = body_json(resp).await;
    assert_eq!(
        json["error"].as_str().unwrap_or(""),
        "BOM_REVISION_SUPERSEDED",
        "error code should be BOM_REVISION_SUPERSEDED, got: {json}"
    );
    let message = json["message"].as_str().unwrap_or("");
    assert!(
        message.contains(&fixture.eco_number),
        "error message should reference ECO number '{}', got: {}",
        fixture.eco_number,
        message
    );
}

/// Released (effective) BOM revision → 201 via plain create.
#[tokio::test]
#[serial]
async fn eco_enforcement_create_accepts_effective_rev() {
    let prod_pool = setup_prod_db().await;
    let num_pool = setup_numbering_db().await;
    let bom_pool = setup_bom_db().await;

    let fixture = insert_eco_fixture(&bom_pool).await;
    let numbering = NumberingClient::direct(num_pool);
    let bom = BomRevisionClient::direct(bom_pool);
    let app = build_app(prod_pool, numbering, bom);

    let body = serde_json::json!({
        "order_number": format!("WO-ECO-OK-{}", &Uuid::new_v4().to_string()[..8]),
        "item_id": Uuid::new_v4(),
        "bom_revision_id": fixture.rev_b_id,
        "planned_quantity": 1,
    });

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/production/work-orders")
                .header("content-type", "application/json")
                .header("x-tenant-id", fixture.tenant_uuid.to_string())
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        axum::http::StatusCode::CREATED,
        "expected 201 for effective revision, body: {}",
        body_json(resp).await
    );
}
