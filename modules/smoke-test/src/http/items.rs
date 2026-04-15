use axum::{extract::State, http::StatusCode, Extension, Json};
use platform_http_contracts::{ApiError, PaginatedResponse, PaginationMeta};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::AppState;
use platform_sdk::extract_tenant;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateItemRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateItemRequest {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, ToSchema)]
pub struct ItemResponse {
    pub id: Uuid,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn row_to_item(row: &sqlx::postgres::PgRow) -> ItemResponse {
    ItemResponse {
        id: row.get("id"),
        tenant_id: row.get("tenant_id"),
        name: row.get("name"),
        description: row.get("description"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[utoipa::path(post, path = "/api/smoke/items", tag = "Items",
    request_body = CreateItemRequest,
    responses(
        (status = 201, description = "Item created", body = ItemResponse),
        (status = 400, description = "Bad request", body = ApiError),
    ),
    security(("bearer" = [])))]
pub async fn create_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateItemRequest>,
) -> Result<(StatusCode, Json<ItemResponse>), ApiError> {
    let tenant_id = extract_tenant(&claims)?;
    let id = Uuid::new_v4();

    let row = sqlx::query(
        "INSERT INTO items (id, tenant_id, name, description) VALUES ($1, $2, $3, $4) RETURNING id, tenant_id, name, description, created_at, updated_at",
    )
    .bind(id)
    .bind(&tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .fetch_one(&state.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let item = row_to_item(&row);

    // Enqueue outbox event
    let payload = serde_json::json!({
        "item_id": item.id,
        "tenant_id": item.tenant_id,
        "name": item.name,
    });
    sqlx::query("INSERT INTO events_outbox (tenant_id, event_type, payload) VALUES ($1, $2, $3)")
        .bind(&tenant_id)
        .bind("smoke_test.item_created")
        .bind(&payload)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok((StatusCode::CREATED, Json(item)))
}

#[utoipa::path(get, path = "/api/smoke/items/{id}", tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 200, description = "Item found", body = ItemResponse),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<Json<ItemResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    let row = sqlx::query(
        "SELECT id, tenant_id, name, description, created_at, updated_at FROM items WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(&tenant_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Item not found"))?;

    Ok(Json(row_to_item(&row)))
}

#[utoipa::path(get, path = "/api/smoke/items", tag = "Items",
    responses(
        (status = 200, description = "List of items", body = PaginatedResponse<ItemResponse>),
    ),
    security(("bearer" = [])))]
pub async fn list_items(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> Result<Json<PaginatedResponse<ItemResponse>>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    let rows = sqlx::query(
        "SELECT id, tenant_id, name, description, created_at, updated_at FROM items WHERE tenant_id = $1 ORDER BY created_at DESC LIMIT 100",
    )
    .bind(&tenant_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    let total = rows.len() as i64;
    let data: Vec<ItemResponse> = rows.iter().map(|r| row_to_item(r)).collect();

    Ok(Json(PaginatedResponse {
        data,
        pagination: PaginationMeta {
            page: 1,
            page_size: 100,
            total_items: total,
            total_pages: 1,
        },
    }))
}

#[utoipa::path(put, path = "/api/smoke/items/{id}", tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    request_body = UpdateItemRequest,
    responses(
        (status = 200, description = "Item updated", body = ItemResponse),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])))]
pub async fn update_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
    Json(req): Json<UpdateItemRequest>,
) -> Result<Json<ItemResponse>, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    let row = sqlx::query(
        "UPDATE items SET name = COALESCE($3, name), description = COALESCE($4, description), updated_at = now() WHERE id = $1 AND tenant_id = $2 RETURNING id, tenant_id, name, description, created_at, updated_at",
    )
    .bind(id)
    .bind(&tenant_id)
    .bind(&req.name)
    .bind(&req.description)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    .ok_or_else(|| ApiError::not_found("Item not found"))?;

    Ok(Json(row_to_item(&row)))
}

#[utoipa::path(delete, path = "/api/smoke/items/{id}", tag = "Items",
    params(("id" = Uuid, Path, description = "Item ID")),
    responses(
        (status = 204, description = "Item deleted"),
        (status = 404, description = "Not found", body = ApiError),
    ),
    security(("bearer" = [])))]
pub async fn delete_item(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = extract_tenant(&claims)?;

    let result = sqlx::query("DELETE FROM items WHERE id = $1 AND tenant_id = $2")
        .bind(id)
        .bind(&tenant_id)
        .execute(&state.pool)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("Item not found"));
    }

    Ok(StatusCode::NO_CONTENT)
}
