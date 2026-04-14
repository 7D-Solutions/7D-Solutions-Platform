use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::subscriptions;
use crate::models::{ApiError, ListSubscriptionsQuery, PaginatedResponse, Subscription};

/// GET /api/ar/subscriptions/:id - Get subscription by ID
#[utoipa::path(get, path = "/api/ar/subscriptions/{id}", tag = "Subscriptions",
    params(("id" = i32, Path, description = "Subscription ID")),
    responses(
        (status = 200, description = "Subscription found", body = Subscription),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_subscription(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Subscription>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let subscription = subscriptions::fetch_with_tenant(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Database error fetching subscription");
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Subscription {} not found", id)))?;

    Ok(Json(subscription))
}

/// GET /api/ar/subscriptions - List subscriptions (with optional filtering)
#[utoipa::path(get, path = "/api/ar/subscriptions", tag = "Subscriptions",
    params(ListSubscriptionsQuery),
    responses(
        (status = 200, description = "Paginated subscriptions", body = PaginatedResponse<Subscription>),
    ),
    security(("bearer" = [])))]
pub async fn list_subscriptions(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListSubscriptionsQuery>,
) -> Result<Json<PaginatedResponse<Subscription>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    let total_items =
        subscriptions::count_subscriptions(&db, &app_id, query.customer_id, query.status.clone())
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "Database error counting subscriptions");
                ApiError::internal("Internal database error")
            })?;

    let subscription_list = subscriptions::list_subscriptions(
        &db,
        &app_id,
        query.customer_id,
        query.status,
        limit,
        offset,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Database error listing subscriptions");
        ApiError::internal("Internal database error")
    })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        subscription_list,
        page,
        limit as i64,
        total_items,
    )))
}
