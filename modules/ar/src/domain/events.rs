//! Event repository — all SQL operations for the events domain.

use sqlx::PgExecutor;

use crate::models::{Event, ListEventsQuery};

/// List events with filtering and pagination.
pub async fn list_events<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
    query: &ListEventsQuery,
    limit: i64,
    offset: i64,
) -> Result<Vec<Event>, sqlx::Error> {
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

    let mut q = sqlx::query_as::<_, Event>(&sql).bind(app_id);

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

    q.bind(limit).bind(offset).fetch_all(executor).await
}

/// Count all events for a tenant (base filter only).
pub async fn count_events<'e>(
    executor: impl PgExecutor<'e>,
    app_id: &str,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM ar_events WHERE app_id = $1")
        .bind(app_id)
        .fetch_one(executor)
        .await
}

/// Fetch a single event by ID with tenant isolation.
pub async fn fetch_by_id<'e>(
    executor: impl PgExecutor<'e>,
    id: i32,
    app_id: &str,
) -> Result<Option<Event>, sqlx::Error> {
    sqlx::query_as::<_, Event>(
        r#"
        SELECT id, app_id, event_type, source, entity_type, entity_id, payload, created_at
        FROM ar_events
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(executor)
    .await
}
