//! Internal HTTP endpoints — not exposed to public clients.
//!
//! Routes:
//!   GET /api/integrations/internal/carrier-credentials/{connector_type}
//!
//! These endpoints are called by other platform modules (e.g. shipping-receiving)
//! over the internal network. They are NOT behind `RequirePermissionsLayer` —
//! network isolation is the access control boundary.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use platform_http_contracts::ApiError;
use std::sync::Arc;

use crate::domain::connectors::repo;
use crate::domain::webhooks::secret_store;
use crate::AppState;

/// GET /api/integrations/internal/carrier-credentials/{connector_type}
///
/// Returns carrier credentials JSON for the tenant identified by `X-App-Id`.
/// Lookup order:
///   1. integrations_carrier_credentials (admin API — encrypted)
///   2. integrations_connector_configs (CI-seeded sandbox creds — plaintext)
/// Returns 404 if neither source has a row.
pub async fn get_carrier_credentials(
    State(state): State<Arc<AppState>>,
    Path(connector_type): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let app_id = match headers.get("x-app-id").and_then(|v| v.to_str().ok()) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            return ApiError::new(400, "missing_app_id", "X-App-Id header is required")
                .into_response();
        }
    };

    // 1. Try admin-managed encrypted credentials first.
    match secret_store::get_carrier_creds(
        &state.pool,
        &app_id,
        &connector_type,
        &state.webhooks_key,
    )
    .await
    {
        Ok(Some(json_str)) => {
            let value: serde_json::Value = match serde_json::from_str(&json_str) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(error = %e, "Corrupt carrier credentials JSON");
                    return ApiError::internal("Corrupt credentials data").into_response();
                }
            };
            return (StatusCode::OK, Json(value)).into_response();
        }
        Ok(None) => {} // fall through to legacy table
        Err(e) => {
            tracing::error!(
                app_id = %app_id,
                connector_type = %connector_type,
                error = %e,
                "Error reading encrypted carrier credentials"
            );
            return ApiError::internal("Internal database error").into_response();
        }
    }

    // 2. Fall back to CI-seeded connector_configs (sandbox creds).
    match repo::get_config_by_type(&state.pool, &app_id, &connector_type).await {
        Ok(Some(config)) => (StatusCode::OK, Json(config.config)).into_response(),
        Ok(None) => ApiError::not_found(format!(
            "No enabled connector config found for connector_type={}",
            connector_type
        ))
        .into_response(),
        Err(e) => {
            tracing::error!(
                app_id = %app_id,
                connector_type = %connector_type,
                error = %e,
                "DB error fetching carrier credentials"
            );
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, routing::get, Router};
    use event_bus::InMemoryBus;
    use serial_test::serial;
    use tower::ServiceExt;

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                .to_string()
        })
    }

    async fn test_pool() -> sqlx::PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to integrations test DB");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
        sqlx::query("DELETE FROM integrations_connector_configs WHERE app_id = $1")
            .bind(app_id)
            .execute(pool)
            .await
            .ok();
    }

    fn build_router(pool: sqlx::PgPool) -> Router {
        let metrics =
            Arc::new(crate::metrics::IntegrationsMetrics::new().expect("metrics init failed"));
        let bus: Arc<dyn event_bus::EventBus> = Arc::new(InMemoryBus::new());
        let state = Arc::new(AppState {
            pool,
            metrics,
            bus,
            webhooks_key: [0u8; 32],
        });

        Router::new()
            .route(
                "/api/integrations/internal/carrier-credentials/{connector_type}",
                get(get_carrier_credentials),
            )
            .with_state(state)
    }

    #[tokio::test]
    #[serial]
    async fn get_carrier_credentials_returns_config_json() {
        let pool = test_pool().await;
        let app_id = "test-app-carrier-creds-001";
        let connector_type = "stub_carrier";
        cleanup(&pool, app_id).await;

        // Insert an enabled connector config
        sqlx::query(
            r#"
            INSERT INTO integrations_connector_configs
                (app_id, connector_type, name, config, enabled, created_at, updated_at)
            VALUES ($1, $2, $3, $4, TRUE, NOW(), NOW())
            "#,
        )
        .bind(app_id)
        .bind(connector_type)
        .bind("Test Stub Carrier")
        .bind(serde_json::json!({"api_key": "test-creds-abc123"}))
        .execute(&pool)
        .await
        .expect("insert failed");

        let router = build_router(pool.clone());
        let req = Request::builder()
            .uri(format!(
                "/api/integrations/internal/carrier-credentials/{}",
                connector_type
            ))
            .header("x-app-id", app_id)
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot request");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "expected 200 for known connector config"
        );

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("parse json");
        assert_eq!(body["api_key"], "test-creds-abc123");

        cleanup(&pool, app_id).await;
    }

    #[tokio::test]
    #[serial]
    async fn get_carrier_credentials_returns_404_for_unknown_connector() {
        let pool = test_pool().await;
        let router = build_router(pool);

        let req = Request::builder()
            .uri("/api/integrations/internal/carrier-credentials/nonexistent-carrier-type")
            .header("x-app-id", "no-such-app")
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot request");
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "expected 404 for unknown connector type"
        );
    }

    #[tokio::test]
    #[serial]
    async fn get_carrier_credentials_returns_400_without_app_id_header() {
        let pool = test_pool().await;
        let router = build_router(pool);

        let req = Request::builder()
            .uri("/api/integrations/internal/carrier-credentials/stub_carrier")
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot request");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[serial]
    async fn get_carrier_credentials_returns_404_for_disabled_connector() {
        let pool = test_pool().await;
        let app_id = "test-app-carrier-creds-disabled";
        let connector_type = "disabled_carrier_type";
        cleanup(&pool, app_id).await;

        sqlx::query(
            r#"
            INSERT INTO integrations_connector_configs
                (app_id, connector_type, name, config, enabled, created_at, updated_at)
            VALUES ($1, $2, $3, $4, FALSE, NOW(), NOW())
            "#,
        )
        .bind(app_id)
        .bind(connector_type)
        .bind("Disabled Carrier")
        .bind(serde_json::json!({"api_key": "disabled-key"}))
        .execute(&pool)
        .await
        .expect("insert disabled connector failed");

        let router = build_router(pool.clone());
        let req = Request::builder()
            .uri(format!(
                "/api/integrations/internal/carrier-credentials/{}",
                connector_type
            ))
            .header("x-app-id", app_id)
            .body(Body::empty())
            .expect("build request");

        let resp = router.oneshot(req).await.expect("oneshot request");
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "disabled connector should return 404, not expose credentials"
        );

        cleanup(&pool, app_id).await;
    }
}
