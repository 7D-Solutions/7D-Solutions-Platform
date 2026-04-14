//! GDPR erasure handler
//!
//! This endpoint intentionally shares the same implementation as the audited
//! tombstone path. The public route name is clearer for compliance workflows.

use axum::{extract::{Path, State}, Json};
use std::sync::Arc;
use uuid::Uuid;

use crate::models::{ErrorBody, TombstoneResponse};
use crate::state::AppState;

/// POST /api/control/tenants/:tenant_id/gdpr-erasure
///
/// Delegates to the existing tombstone handler so the same validation,
/// idempotency, and outbox semantics apply.
pub async fn gdpr_erasure(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<Uuid>,
) -> Result<Json<TombstoneResponse>, (axum::http::StatusCode, Json<ErrorBody>)> {
    super::retention::tombstone_tenant(State(state), Path(tenant_id)).await
}
