//! Admin HTTP router for managing feature flags.
//!
//! Mount this router under a guarded `/admin` prefix.  The caller is
//! responsible for authentication and authorization; these endpoints must
//! not be exposed without a platform-admin guard.
//!
//! # Routes
//!
//! | Method | Path                                        | Description                           |
//! |--------|---------------------------------------------|---------------------------------------|
//! | GET    | `/feature-flags`                            | List all flags                        |
//! | GET    | `/feature-flags/{flag_name}`                | Get a single flag                     |
//! | PUT    | `/feature-flags/{flag_name}`                | Set enabled/disabled for a flag       |
//! | DELETE | `/feature-flags/{flag_name}`                | Remove a flag row                     |

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::flags::{delete_flag, is_enabled, set_flag, FlagError};

/// State threaded through admin handler functions.
#[derive(Clone)]
pub struct AdminState {
    pub pool: PgPool,
}

/// Request body for `PUT /feature-flags/{flag_name}`.
#[derive(Debug, Deserialize)]
pub struct SetFlagRequest {
    /// `null` sets the global default; a UUID sets a per-tenant override.
    pub tenant_id: Option<Uuid>,
    pub enabled: bool,
}

/// Response body for flag reads.
#[derive(Debug, Serialize)]
pub struct FlagResponse {
    pub flag_name: String,
    pub tenant_id: Option<Uuid>,
    pub enabled: bool,
}

/// Response body for flag lists.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct FlagRow {
    pub flag_name: String,
    pub tenant_id: Option<Uuid>,
    pub enabled: bool,
}

fn flag_error_response(e: FlagError) -> impl IntoResponse {
    tracing::warn!(error = %e, "feature flag admin error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": e.to_string() })),
    )
}

/// GET `/feature-flags` — list all flag rows.
async fn list_flags(State(state): State<AdminState>) -> impl IntoResponse {
    let rows: Result<Vec<FlagRow>, sqlx::Error> = sqlx::query_as(
        "SELECT flag_name, tenant_id, enabled FROM feature_flags ORDER BY flag_name, tenant_id",
    )
    .fetch_all(&state.pool)
    .await;

    match rows {
        Ok(r) => (StatusCode::OK, Json(r)).into_response(),
        Err(e) => flag_error_response(FlagError::Db(e)).into_response(),
    }
}

/// GET `/feature-flags/{flag_name}?tenant_id=<uuid>` — resolve a single flag.
async fn get_flag(
    Path(flag_name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let tenant_id = params
        .get("tenant_id")
        .and_then(|v| Uuid::parse_str(v).ok());

    match is_enabled(&state.pool, &flag_name, tenant_id).await {
        Ok(enabled) => (
            StatusCode::OK,
            Json(FlagResponse {
                flag_name,
                tenant_id,
                enabled,
            }),
        )
            .into_response(),
        Err(e) => flag_error_response(e).into_response(),
    }
}

/// PUT `/feature-flags/{flag_name}` — create or update a flag.
async fn put_flag(
    Path(flag_name): Path<String>,
    State(state): State<AdminState>,
    Json(body): Json<SetFlagRequest>,
) -> impl IntoResponse {
    match set_flag(&state.pool, &flag_name, body.tenant_id, body.enabled).await {
        Ok(()) => (
            StatusCode::OK,
            Json(FlagResponse {
                flag_name,
                tenant_id: body.tenant_id,
                enabled: body.enabled,
            }),
        )
            .into_response(),
        Err(e) => flag_error_response(e).into_response(),
    }
}

/// DELETE `/feature-flags/{flag_name}?tenant_id=<uuid>` — remove a flag row.
async fn remove_flag(
    Path(flag_name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let tenant_id = params
        .get("tenant_id")
        .and_then(|v| Uuid::parse_str(v).ok());

    match delete_flag(&state.pool, &flag_name, tenant_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => flag_error_response(e).into_response(),
    }
}

/// Build the admin router for feature flag management.
///
/// Mount at `/admin` (or similar) with appropriate auth middleware:
/// ```rust,ignore
/// let app = Router::new()
///     .nest("/admin", feature_flags::admin_router(pool.clone()))
///     .layer(require_platform_admin);
/// ```
pub fn admin_router(pool: PgPool) -> Router {
    Router::new()
        .route("/feature-flags", get(list_flags))
        .route("/feature-flags/{flag_name}", get(get_flag))
        .route("/feature-flags/{flag_name}", put(put_flag))
        .route("/feature-flags/{flag_name}", delete(remove_flag))
        .with_state(AdminState { pool })
}
