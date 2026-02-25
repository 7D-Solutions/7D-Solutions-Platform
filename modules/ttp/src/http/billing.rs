/// HTTP handler: POST /api/ttp/billing-runs
///
/// Triggers a billing run for a tenant + billing period.
///
/// # Idempotency
///
/// Calling with the same (tenant_id, billing_period) again is a no-op — the
/// endpoint returns the existing run summary with `was_noop: true`.
///
/// # Request
///
/// Tenant is derived from the JWT `VerifiedClaims`.
///
/// ```json
/// {
///   "billing_period": "2026-02",
///   "idempotency_key": "caller-generated-key"
/// }
/// ```
///
/// # Response — 200 OK
///
/// ```json
/// {
///   "run_id": "uuid",
///   "tenant_id": "uuid",
///   "billing_period": "2026-02",
///   "parties_billed": 3,
///   "total_amount_minor": 30000,
///   "currency": "usd",
///   "was_noop": false
/// }
/// ```

use axum::{extract::State, http::StatusCode, Extension, Json};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::AppState;
use crate::clients::ar::ArClient;
use crate::clients::tenant_registry::{TenantRegistryClient, TenantRegistryError};
use crate::domain::billing::{run_billing, BillingError};
use crate::events::{
    create_ttp_envelope, BillingRunCompleted, BillingRunFailed, BILLING_RUN_COMPLETED,
    BILLING_RUN_FAILED,
};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct BillingRunRequest {
    pub billing_period: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct BillingRunResponse {
    pub run_id: Uuid,
    pub tenant_id: Uuid,
    pub billing_period: String,
    pub parties_billed: u32,
    pub total_amount_minor: i64,
    pub currency: String,
    pub was_noop: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    error: String,
    code: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// POST /api/ttp/billing-runs
pub async fn create_billing_run(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<BillingRunRequest>,
) -> Result<Json<BillingRunResponse>, (StatusCode, Json<ErrorBody>)>
{
    let tenant_id = claims
        .map(|Extension(c)| c.tenant_id)
        .ok_or_else(|| (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "Missing or invalid authentication".to_string(),
                code: "unauthorized".to_string(),
            }),
        ))?;
    // Build clients from env — base URLs are resolved at request time from env so
    // they can be overridden in test environments.
    let registry_url = std::env::var("TENANT_REGISTRY_URL")
        .unwrap_or_else(|_| "http://localhost:8092".to_string());
    let ar_url = std::env::var("AR_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8086".to_string());

    let registry = TenantRegistryClient::new(registry_url);
    let ar = ArClient::new(ar_url);

    // Validate billing_period format ("YYYY-MM")
    if !is_valid_billing_period(&req.billing_period) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "billing_period must be in YYYY-MM format".to_string(),
                code: "validation_error".to_string(),
            }),
        ));
    }

    // Validate idempotency_key is non-empty
    if req.idempotency_key.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "idempotency_key must not be empty".to_string(),
                code: "validation_error".to_string(),
            }),
        ));
    }

    let correlation_id = req.idempotency_key.clone();

    match run_billing(
        &state.pool,
        &registry,
        &ar,
        tenant_id,
        &req.billing_period,
        &req.idempotency_key,
    )
    .await
    {
        Ok(summary) => {
            // Publish BillingRunCompleted event (best-effort; do not fail the HTTP response)
            let payload = BillingRunCompleted {
                run_id: summary.run_id,
                tenant_id,
                billing_period: req.billing_period.clone(),
                parties_billed: summary.parties_billed,
                total_amount_minor: summary.total_amount_minor,
                currency: summary.currency.clone(),
            };

            if !summary.was_noop {
                let _env = create_ttp_envelope(
                    tenant_id,
                    BILLING_RUN_COMPLETED,
                    &correlation_id,
                    "billing",
                    payload,
                );
                // NOTE: Event bus publishing is wired in the full service start; for now
                // the envelope is created (and merchant_context validated) but not published.
                // bd-2hdr (E2E proof) will verify end-to-end event delivery.
            }

            Ok(Json(BillingRunResponse {
                run_id: summary.run_id,
                tenant_id,
                billing_period: req.billing_period,
                parties_billed: summary.parties_billed,
                total_amount_minor: summary.total_amount_minor,
                currency: summary.currency,
                was_noop: summary.was_noop,
            }))
        }
        Err(BillingError::Registry(TenantRegistryError::TenantNotFound(tid))) => {
            tracing::warn!("Billing run failed: tenant {} not found", tid);
            Err((
                StatusCode::NOT_FOUND,
                Json(ErrorBody {
                    error: format!("Tenant {} not found in registry", tid),
                    code: "tenant_not_found".to_string(),
                }),
            ))
        }
        Err(BillingError::Registry(TenantRegistryError::NoAppId(tid))) => {
            tracing::warn!("Billing run failed: tenant {} has no app_id", tid);
            Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorBody {
                    error: format!("Tenant {} has no app_id assigned", tid),
                    code: "no_app_id".to_string(),
                }),
            ))
        }
        Err(e) => {
            tracing::error!("Billing run error: {:?}", e);

            // Publish BillingRunFailed event (best-effort)
            let _fail_env = create_ttp_envelope(
                tenant_id,
                BILLING_RUN_FAILED,
                &correlation_id,
                "billing",
                BillingRunFailed {
                    run_id: Uuid::nil(),
                    tenant_id,
                    billing_period: req.billing_period.clone(),
                    reason: e.to_string(),
                },
            );

            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: e.to_string(),
                    code: "billing_run_failed".to_string(),
                }),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate billing_period format: "YYYY-MM"
fn is_valid_billing_period(period: &str) -> bool {
    if period.len() != 7 {
        return false;
    }
    let parts: Vec<&str> = period.splitn(2, '-').collect();
    if parts.len() != 2 {
        return false;
    }
    let year = parts[0].parse::<u16>();
    let month = parts[1].parse::<u8>();
    match (year, month) {
        (Ok(y), Ok(m)) => y >= 2020 && m >= 1 && m <= 12,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn billing_period_validation_accepts_valid_periods() {
        assert!(is_valid_billing_period("2026-01"));
        assert!(is_valid_billing_period("2026-12"));
        assert!(is_valid_billing_period("2030-06"));
    }

    #[test]
    fn billing_period_validation_rejects_invalid_formats() {
        assert!(!is_valid_billing_period("202601"));
        assert!(!is_valid_billing_period("2026-1"));
        assert!(!is_valid_billing_period("26-01"));
        assert!(!is_valid_billing_period("2026-00"));
        assert!(!is_valid_billing_period("2026-13"));
        assert!(!is_valid_billing_period(""));
        assert!(!is_valid_billing_period("2026-02-01"));
    }

    // ---------------------------------------------------------------------------
    // Integration tests against real DB + running services
    // ---------------------------------------------------------------------------

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use security::ActorType;
    use tower::ServiceExt;

    /// Build a lazy pool that defers the actual TCP connection until first use.
    /// Validation-only tests (400 responses) never touch the DB so no connection is made.
    fn lazy_pool() -> sqlx::PgPool {
        let url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5450/ttp_default_db".to_string());
        sqlx::PgPool::connect_lazy(&url).expect("build lazy pool")
    }

    /// Build fake VerifiedClaims for a given tenant.
    fn fake_claims(tenant_id: Uuid) -> VerifiedClaims {
        let now = Utc::now();
        VerifiedClaims {
            user_id: Uuid::new_v4(),
            tenant_id,
            app_id: None,
            roles: vec!["admin".into()],
            perms: vec!["ttp.mutate".into()],
            actor_type: ActorType::User,
            issued_at: now,
            expires_at: now + chrono::Duration::minutes(15),
            token_id: Uuid::new_v4(),
            version: "1".into(),
        }
    }

    /// Build the HTTP app for testing, injecting VerifiedClaims via Extension.
    fn build_app(pool: sqlx::PgPool, claims: VerifiedClaims) -> axum::Router {
        use crate::metrics::TtpMetrics;
        let metrics = Arc::new(TtpMetrics::new().unwrap());
        let state = Arc::new(crate::AppState { pool, metrics });
        axum::Router::new()
            .route("/api/ttp/billing-runs", axum::routing::post(create_billing_run))
            .layer(Extension(claims))
            .with_state(state)
    }

    #[tokio::test]
    async fn missing_claims_returns_401() {
        use crate::metrics::TtpMetrics;
        let metrics = Arc::new(TtpMetrics::new().unwrap());
        let state = Arc::new(crate::AppState { pool: lazy_pool(), metrics });
        let app = axum::Router::new()
            .route("/api/ttp/billing-runs", axum::routing::post(create_billing_run))
            .with_state(state);

        let body = serde_json::json!({
            "billing_period": "2026-02",
            "idempotency_key": "test-key"
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ttp/billing-runs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn bad_billing_period_returns_400() {
        let tenant_id = Uuid::new_v4();
        let app = build_app(lazy_pool(), fake_claims(tenant_id));

        let body = serde_json::json!({
            "billing_period": "202602",
            "idempotency_key": "test-key"
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ttp/billing-runs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn empty_idempotency_key_returns_400() {
        let tenant_id = Uuid::new_v4();
        let app = build_app(lazy_pool(), fake_claims(tenant_id));

        let body = serde_json::json!({
            "billing_period": "2026-02",
            "idempotency_key": ""
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ttp/billing-runs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// Integration: unknown tenant returns 404.
    ///
    /// Requires TENANT_REGISTRY_URL pointing at a running tenant-registry and
    /// DATABASE_URL pointing at the TTP postgres.
    #[tokio::test]
    #[ignore]
    async fn integration_unknown_tenant_returns_404() {
        let tenant_id = Uuid::new_v4();
        let app = build_app(lazy_pool(), fake_claims(tenant_id));

        let body = serde_json::json!({
            "billing_period": "2026-02",
            "idempotency_key": "int-test-key-1"
        });

        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ttp/billing-runs")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
