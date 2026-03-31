use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, Event, ListEventsQuery};

/// GET /api/ar/events - List events with filtering
pub async fn list_events(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListEventsQuery>,
) -> Result<Json<Vec<Event>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(100).min(1000);
    let offset = query.offset.unwrap_or(0);

    let mut sql = String::from(
        "SELECT id, app_id, event_type, source, entity_type, entity_id, payload, created_at FROM ar_events WHERE app_id = $1",
    );
    let mut param_count = 1;
    let mut conditions = Vec::new();

    if query.entity_id.is_some() {
        param_count += 1;
        conditions.push(format!("entity_id = ${}", param_count));
    }
    if query.entity_type.is_some() {
        param_count += 1;
        conditions.push(format!("entity_type = ${}", param_count));
    }
    if query.event_type.is_some() {
        param_count += 1;
        conditions.push(format!("event_type = ${}", param_count));
    }
    if query.source.is_some() {
        param_count += 1;
        conditions.push(format!("source = ${}", param_count));
    }
    if query.start.is_some() {
        param_count += 1;
        conditions.push(format!("created_at >= ${}", param_count));
    }
    if query.end.is_some() {
        param_count += 1;
        conditions.push(format!("created_at <= ${}", param_count));
    }

    if !conditions.is_empty() {
        sql.push_str(" AND ");
        sql.push_str(&conditions.join(" AND "));
    }

    sql.push_str(" ORDER BY created_at DESC");
    param_count += 1;
    sql.push_str(&format!(" LIMIT ${}", param_count));
    param_count += 1;
    sql.push_str(&format!(" OFFSET ${}", param_count));

    let mut q = sqlx::query_as::<_, Event>(&sql).bind(&app_id);

    if let Some(ref entity_id) = query.entity_id {
        q = q.bind(entity_id);
    }
    if let Some(ref entity_type) = query.entity_type {
        q = q.bind(entity_type);
    }
    if let Some(ref event_type) = query.event_type {
        q = q.bind(event_type);
    }
    if let Some(ref source) = query.source {
        q = q.bind(source);
    }
    if let Some(start) = query.start {
        q = q.bind(start);
    }
    if let Some(end) = query.end {
        q = q.bind(end);
    }

    q = q.bind(limit).bind(offset);

    let events = q.fetch_all(&db).await.map_err(|e| {
        tracing::error!("Failed to list events: {}", e);
        ApiError::internal("Failed to list events")
    })?;

    Ok(Json(events))
}

/// GET /api/ar/events/{id} - Get a single event by ID
pub async fn get_event(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Event>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let event = sqlx::query_as::<_, Event>(
        r#"
        SELECT id, app_id, event_type, source, entity_type, entity_id, payload, created_at
        FROM ar_events
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
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
