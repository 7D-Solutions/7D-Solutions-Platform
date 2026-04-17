pub mod blankets;
pub mod labels;
pub mod orders;
pub mod releases;

use axum::{extract::State, Json};
use health::{build_ready_response, db_check_with_pool, ready_response_to_axum, PoolMetrics};
use std::sync::Arc;
use std::time::Instant;
use utoipa::OpenApi;

use crate::AppState;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Sales Orders Service",
        version = "0.1.0",
        description = "Customer order management: draft → booked → in_fulfillment → shipped → closed, blanket orders with releases.",
    ),
    paths(
        orders::create_order,
        orders::list_orders,
        orders::get_order,
        orders::update_order,
        orders::book_order,
        orders::cancel_order,
        orders::add_line,
        orders::update_line,
        orders::remove_line,
        blankets::create_blanket,
        blankets::list_blankets,
        blankets::get_blanket,
        blankets::update_blanket,
        blankets::activate_blanket,
        blankets::add_blanket_line,
        releases::create_release,
        releases::list_releases,
        labels::list_labels,
        labels::upsert_label,
        labels::delete_label,
    ),
    components(schemas(
        crate::domain::orders::SalesOrder,
        crate::domain::orders::SalesOrderLine,
        crate::domain::orders::SalesOrderWithLines,
        crate::domain::orders::CreateOrderRequest,
        crate::domain::orders::UpdateOrderRequest,
        crate::domain::orders::BookOrderRequest,
        crate::domain::orders::CancelOrderRequest,
        crate::domain::orders::CreateOrderLineRequest,
        crate::domain::orders::UpdateOrderLineRequest,
        crate::domain::blankets::BlanketOrder,
        crate::domain::blankets::BlanketOrderLine,
        crate::domain::blankets::BlanketOrderRelease,
        crate::domain::blankets::BlanketOrderWithLines,
        crate::domain::blankets::CreateBlanketRequest,
        crate::domain::blankets::UpdateBlanketRequest,
        crate::domain::blankets::ActivateBlanketRequest,
        crate::domain::blankets::CreateBlanketLineRequest,
        crate::domain::blankets::CreateReleaseRequest,
        crate::domain::labels::StatusLabel,
        crate::domain::labels::UpsertLabelRequest,
    )),
    tags(
        (name = "SalesOrders", description = "Sales order lifecycle"),
        (name = "BlanketOrders", description = "Blanket order management"),
        (name = "BlanketReleases", description = "Release against blanket lines"),
        (name = "SalesOrderLabels", description = "Status label configuration"),
    )
)]
pub struct ApiDoc;

pub async fn health_check(State(state): State<Arc<AppState>>) -> impl axum::response::IntoResponse {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .fetch_one(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;
    let pool_metrics = PoolMetrics {
        size: state.pool.size(),
        idle: state.pool.num_idle() as u32,
        active: state.pool.size().saturating_sub(state.pool.num_idle() as u32),
    };
    let resp = build_ready_response(
        "sales-orders",
        env!("CARGO_PKG_VERSION"),
        vec![db_check_with_pool(latency, db_err, pool_metrics)],
    );
    ready_response_to_axum(resp)
}

pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
