/// POST /api/control/tenants handler
///
/// Creates a new tenant using the Guard → Mutation → Outbox pattern:
/// 1. Guard: check idempotency key; reject duplicate if key already used.
/// 2. Mutation: insert tenant row (status=pending) atomically.
/// 3. Outbox: write tenant.provisioning_started event in the same transaction.
///
/// Returns 202 Accepted with the tenant_id and current status.
/// Returns 200 OK if idempotency key was already used (replays the result).
/// Returns 422 Unprocessable Entity for validation errors.
/// Returns 409 Conflict if tenant_id is explicitly supplied and already exists.

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde_json::json;
use sqlx::Acquire;
use std::sync::Arc;
use uuid::Uuid;

use tenant_registry::event_types;

use crate::models::{CreateTenantRequest, CreateTenantResponse, ErrorBody};
use crate::state::AppState;

/// POST /api/control/tenants
pub async fn create_tenant(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<(StatusCode, Json<CreateTenantResponse>), (StatusCode, Json<ErrorBody>)> {
    // Validate idempotency key
    if req.idempotency_key.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ErrorBody {
                error: "idempotency_key must not be empty".to_string(),
            }),
        ));
    }

    let mut conn = state.pool.acquire().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database connection error: {e}"),
            }),
        )
    })?;

    // --- GUARD: check idempotency key ---
    let existing: Option<(Uuid,)> = sqlx::query_as(
        "SELECT tenant_id FROM provisioning_requests WHERE idempotency_key = $1",
    )
    .bind(&req.idempotency_key)
    .fetch_optional(&mut *conn)
    .await
    .map_err(|e| db_error(e))?;

    if let Some((existing_tenant_id,)) = existing {
        // Idempotency replay: return existing result
        let status: String = sqlx::query_scalar(
            "SELECT status FROM tenants WHERE tenant_id = $1",
        )
        .bind(existing_tenant_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| db_error(e))?;

        return Ok((
            StatusCode::OK,
            Json(CreateTenantResponse {
                tenant_id: existing_tenant_id,
                status,
                idempotency_key: req.idempotency_key,
            }),
        ));
    }

    // Resolve tenant ID
    let tenant_id = req.tenant_id.unwrap_or_else(Uuid::new_v4);

    // --- GUARD: check tenant_id uniqueness if explicitly supplied ---
    if req.tenant_id.is_some() {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM tenants WHERE tenant_id = $1)",
        )
        .bind(tenant_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| db_error(e))?;

        if exists {
            return Err((
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    error: format!("Tenant {} already exists", tenant_id),
                }),
            ));
        }
    }

    let environment = req.environment.as_str();
    let now = Utc::now();

    // --- MUTATION + OUTBOX: atomic transaction ---
    let mut tx = conn.begin().await.map_err(|e| db_error(e))?;

    // 1. Insert tenant record (status=pending)
    sqlx::query(
        r#"
        INSERT INTO tenants (tenant_id, status, environment, module_schema_versions, created_at, updated_at)
        VALUES ($1, 'pending', $2, '{}'::jsonb, $3, $3)
        "#,
    )
    .bind(tenant_id)
    .bind(environment)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| db_error(e))?;

    // 2. Record idempotency key
    sqlx::query(
        "INSERT INTO provisioning_requests (idempotency_key, tenant_id, environment, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(&req.idempotency_key)
    .bind(tenant_id)
    .bind(environment)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| db_error(e))?;

    // 3. Write provisioning_started event to outbox
    let payload = json!({
        "tenant_id": tenant_id,
        "environment": environment,
        "idempotency_key": req.idempotency_key,
        "occurred_at": now,
    });

    sqlx::query(
        r#"
        INSERT INTO provisioning_outbox (tenant_id, event_type, payload, created_at)
        VALUES ($1, $2, $3, $4)
        "#,
    )
    .bind(tenant_id)
    .bind(event_types::TENANT_PROVISIONING_STARTED)
    .bind(&payload)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| db_error(e))?;

    // Commit atomically
    tx.commit().await.map_err(|e| db_error(e))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(CreateTenantResponse {
            tenant_id,
            status: "pending".to_string(),
            idempotency_key: req.idempotency_key,
        }),
    ))
}

fn db_error(e: sqlx::Error) -> (StatusCode, Json<ErrorBody>) {
    tracing::error!("Database error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: format!("Database error: {e}"),
        }),
    )
}
