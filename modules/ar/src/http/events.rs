use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::events;
use crate::models::{ApiError, Event, ListEventsQuery, PaginatedResponse};

/// GET /api/ar/events - List events with filtering
#[utoipa::path(get, path = "/api/ar/events", tag = "Events",
    params(ListEventsQuery),
    responses(
        (status = 200, description = "Paginated events", body = PaginatedResponse<Event>),
    ),
    security(("bearer" = [])))]
pub async fn list_events(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListEventsQuery>,
) -> Result<Json<PaginatedResponse<Event>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(100).min(1000);
    let offset = query.offset.unwrap_or(0);

    let event_list = events::list_events(&db, &app_id, &query, limit, offset)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list events: {}", e);
            ApiError::internal("Failed to list events")
        })?;

    let total_items = events::count_events(&db, &app_id)
        .await
        .unwrap_or(event_list.len() as i64);

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(event_list, page, limit as i64, total_items)))
}

/// GET /api/ar/events/{id} - Get a single event by ID
#[utoipa::path(get, path = "/api/ar/events/{id}", tag = "Events",
    params(("id" = i32, Path, description = "Event ID")),
    responses(
        (status = 200, description = "Event found", body = Event),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_event(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Event>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let event = events::fetch_by_id(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to fetch event {}: {}", id, e);
            ApiError::internal("Failed to fetch event")
        })?;

    match event {
        Some(e) => Ok(Json(e)),
        None => Err(ApiError::not_found(format!("Event {} not found", id))),
    }
}
