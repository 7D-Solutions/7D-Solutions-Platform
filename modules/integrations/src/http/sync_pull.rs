//! POST /api/integrations/sync/pull — prod-safe per-tenant manual CDC pull.
//!
//! Permission: integrations.sync.pull
//! Enforces one inflight pull per tenant via a partial unique index. Pulls all
//! entity types (entity_type logged as 'all'). Connection check happens before
//! the inflight INSERT to prevent phantom log rows.

use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::oauth::service as oauth_service;
use crate::domain::qbo::cdc as qbo_cdc;
use crate::AppState;
use platform_sdk::extract_tenant;

/// POST /api/integrations/sync/pull
pub async fn sync_pull(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let triggered_by = match &claims {
        Some(Extension(c)) => c.user_id.to_string(),
        None => "unknown".to_string(),
    };

    // Step 1: Check QBO connection before creating any log row.
    let connection =
        match oauth_service::get_connection_status(&state.pool, &app_id, "quickbooks").await {
            Ok(Some(c)) => c,
            Ok(None) => {
                return ApiError::new(412, "not_connected", "No QuickBooks connection found")
                    .into_response()
            }
            Err(e) => {
                tracing::error!(app_id = %app_id, error = %e, "sync_pull: OAuth lookup error");
                return ApiError::internal("Internal database error").into_response();
            }
        };

    if connection.connection_status != "connected" {
        return ApiError::new(
            412,
            "not_connected",
            format!(
                "QuickBooks connection is '{}' — reconnection required",
                connection.connection_status
            ),
        )
        .into_response();
    }

    // Step 2: INSERT inflight row. SQLSTATE 23505 = unique violation (already inflight).
    let pull_log_id: Uuid = match sqlx::query_scalar(
        r#"
        INSERT INTO integrations_sync_pull_log
            (app_id, entity_type, triggered_by, status)
        VALUES ($1, 'all', $2, 'inflight')
        RETURNING id
        "#,
    )
    .bind(&app_id)
    .bind(&triggered_by)
    .fetch_one(&state.pool)
    .await
    {
        Ok(id) => id,
        Err(sqlx::Error::Database(ref db_err))
            if db_err.code().as_deref() == Some("23505") =>
        {
            let mut headers = HeaderMap::new();
            headers.insert("Retry-After", HeaderValue::from_static("60"));
            return (
                StatusCode::CONFLICT,
                headers,
                Json(serde_json::json!({ "error": "inflight", "retry_after": 60 })),
            )
                .into_response();
        }
        Err(e) => {
            tracing::error!(app_id = %app_id, error = %e, "sync_pull: inflight INSERT failed");
            return ApiError::internal("Internal database error").into_response();
        }
    };

    // Step 3: RAII guard — flips the row to 'failed' on drop unless defused.
    // Uses tokio::spawn so the update runs even when the handler returns early.
    let guard_pool = state.pool.clone();
    let guard_log_id = pull_log_id;
    let guard = PullLogGuard {
        pool: guard_pool,
        log_id: guard_log_id,
        defused: false,
    };

    // Step 4: Run CDC tick.
    match qbo_cdc::cdc_tick_for_tenant(&state.pool, &app_id).await {
        Ok(_) => {
            // Step 5: Mark complete.
            if let Err(e) = sqlx::query(
                "UPDATE integrations_sync_pull_log \
                 SET status = 'complete', completed_at = now() \
                 WHERE id = $1",
            )
            .bind(pull_log_id)
            .execute(&state.pool)
            .await
            {
                tracing::error!(app_id = %app_id, pull_log_id = %pull_log_id, error = %e,
                    "sync_pull: failed to mark log row complete");
            }
            guard.defuse();
            Json(serde_json::json!({
                "pull_log_id": pull_log_id,
                "status": "complete"
            }))
            .into_response()
        }
        Err(e) => {
            let msg = e.to_string();
            tracing::error!(app_id = %app_id, pull_log_id = %pull_log_id, error = %e,
                "sync_pull: CDC tick failed");
            if let Err(ue) = sqlx::query(
                "UPDATE integrations_sync_pull_log \
                 SET status = 'failed', error = $1 \
                 WHERE id = $2",
            )
            .bind(&msg)
            .bind(pull_log_id)
            .execute(&state.pool)
            .await
            {
                tracing::error!(error = %ue, "sync_pull: failed to mark log row failed");
            }
            guard.defuse();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "pull_failed",
                    "message": "internal error"
                })),
            )
                .into_response()
        }
    }
}

/// RAII guard that marks an inflight pull log row as 'failed' on drop.
///
/// Defuse before returning on the happy path so the final UPDATE isn't
/// duplicated. Uses tokio::spawn in Drop so the async update fires while
/// the runtime is still live.
struct PullLogGuard {
    pool: sqlx::PgPool,
    log_id: Uuid,
    defused: bool,
}

impl PullLogGuard {
    fn defuse(mut self) {
        self.defused = true;
    }
}

impl Drop for PullLogGuard {
    fn drop(&mut self) {
        if self.defused {
            return;
        }
        let pool = self.pool.clone();
        let log_id = self.log_id;
        tokio::spawn(async move {
            let _ = sqlx::query(
                "UPDATE integrations_sync_pull_log \
                 SET status = 'failed', error = 'handler exited without status update' \
                 WHERE id = $1 AND status = 'inflight'",
            )
            .bind(log_id)
            .execute(&pool)
            .await;
        });
    }
}
