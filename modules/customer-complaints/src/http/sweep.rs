//! Overdue sweep endpoint — idempotent, admin-gated.
//!
//! Finds complaints where due_date < now() AND status NOT IN ('closed','cancelled')
//! AND overdue_emitted_at IS NULL. For each, emits complaint_overdue once and
//! sets overdue_emitted_at in the same transaction (atomicity guarantee).

use axum::{extract::State, response::IntoResponse, Extension, Json};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::ApiError;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::events::produced::{self as ev, ComplaintOverduePayload};
use crate::http::tenant::with_request_id;
use crate::outbox;
use crate::AppState;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SweepOverdueResponse {
    pub swept_count: i64,
}

#[derive(Debug, sqlx::FromRow)]
struct OverdueCandidate {
    id: Uuid,
    tenant_id: String,
    assigned_to: Option<String>,
    due_date: chrono::DateTime<Utc>,
    severity: Option<String>,
}

/// Core sweep logic — call this from HTTP handler and from integration tests.
pub async fn sweep_overdue_complaints(pool: &PgPool) -> Result<i64, sqlx::Error> {
    let candidates: Vec<OverdueCandidate> = sqlx::query_as(
        r#"
        SELECT id, tenant_id, assigned_to, due_date, severity
        FROM complaints
        WHERE due_date < now()
          AND status NOT IN ('closed', 'cancelled')
          AND overdue_emitted_at IS NULL
        ORDER BY due_date ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    let mut swept = 0i64;

    for c in candidates {
        let days_overdue = (Utc::now() - c.due_date).num_days().max(0);
        let event_id = Uuid::new_v4();
        let corr = event_id.to_string();
        let payload = ComplaintOverduePayload {
            complaint_id: c.id,
            tenant_id: c.tenant_id.clone(),
            assigned_to: c.assigned_to,
            due_date: c.due_date,
            days_overdue,
            severity: c.severity,
        };

        let mut tx = pool.begin().await?;

        // Re-check inside transaction (FOR UPDATE) to prevent double-emit under concurrent sweep runs.
        // fetch_optional returns Option<Option<T>> for nullable columns: outer None = no row,
        // inner None = column is NULL (not yet emitted), inner Some = already emitted.
        let already_emitted: Option<Option<chrono::DateTime<Utc>>> = sqlx::query_scalar(
            "SELECT overdue_emitted_at FROM complaints WHERE id = $1 FOR UPDATE",
        )
        .bind(c.id)
        .fetch_optional(&mut *tx)
        .await?;

        if already_emitted.flatten().is_some() {
            tx.rollback().await?;
            continue;
        }

        outbox::enqueue_event_tx(
            &mut tx,
            event_id,
            ev::EVENT_COMPLAINT_OVERDUE,
            c.id,
            &c.tenant_id,
            Some(&corr),
            None,
            &payload,
        )
        .await?;

        sqlx::query(
            "UPDATE complaints SET overdue_emitted_at = now() WHERE id = $1",
        )
        .bind(c.id)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        swept += 1;
    }

    Ok(swept)
}

#[utoipa::path(
    post, path = "/api/customer-complaints/admin/sweep-overdue", tag = "Admin",
    responses(
        (status = 200, description = "Sweep complete", body = SweepOverdueResponse),
        (status = 500, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn sweep_overdue(
    State(state): State<Arc<AppState>>,
    _claims: Option<Extension<VerifiedClaims>>,
    tracing_ctx: Option<Extension<TracingContext>>,
) -> impl IntoResponse {
    match sweep_overdue_complaints(&state.pool).await {
        Ok(swept_count) => Json(SweepOverdueResponse { swept_count }).into_response(),
        Err(e) => {
            tracing::error!("sweep_overdue failed: {}", e);
            with_request_id(ApiError::internal(e.to_string()), &tracing_ctx).into_response()
        }
    }
}
