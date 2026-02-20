use axum::{extract::State, http::StatusCode, Json};
use health::{build_ready_response, db_check, ready_response_to_axum, ReadyResponse};
use std::sync::Arc;
use std::time::Instant;

/// GET /api/ready — readiness probe (verifies DB connectivity)
pub async fn ready(
    State(state): State<Arc<crate::AppState>>,
) -> Result<Json<ReadyResponse>, (StatusCode, Json<ReadyResponse>)> {
    let start = Instant::now();
    let db_err = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .err()
        .map(|e| e.to_string());
    let latency = start.elapsed().as_millis() as u64;

    let resp = build_ready_response(
        "timekeeping",
        env!("CARGO_PKG_VERSION"),
        vec![db_check(latency, db_err)],
    );
    ready_response_to_axum(resp)
}
