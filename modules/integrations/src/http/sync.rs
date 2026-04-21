//! HTTP handlers for the sync API.
//!
//! Permission matrix:
//!   POST /api/integrations/sync/authority              → integrations.sync.authority.flip
//!   POST /api/integrations/sync/conflicts/{id}/resolve → integrations.sync.conflict.resolve
//!   POST /api/integrations/sync/push/{entity_type}     → integrations.sync.push
//!   GET  /api/integrations/sync/authority              → integrations.sync.read
//!   GET  /api/integrations/sync/conflicts              → integrations.sync.read
//!   GET  /api/integrations/sync/dlq                    → integrations.sync.read
//!   GET  /api/integrations/sync/push-attempts          → integrations.sync.read
//!   GET  /api/integrations/sync/jobs                   → integrations.sync.read
//!   POST /api/integrations/sync/cdc/trigger            → integrations.oauth.admin (dev-local only)

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::domain::oauth::service as oauth_service;
use crate::domain::qbo::{cdc as qbo_cdc, client::QboClient, QboError, TokenProvider};
use crate::domain::sync::authority_repo::list_authority;
use crate::domain::sync::conflicts_repo::list_conflicts_paged;
use crate::domain::sync::resolve_service::{
    bulk_resolve_conflicts, resolve_conflict_transactional, BulkResolveError, BulkResolveItem,
    BulkResolveOutcome, PushError, ResolveConflictError, ResolveService,
};
use crate::domain::sync::{flip_authority as svc_flip_authority, FlipError};
use crate::domain::sync::health::{list_jobs as repo_list_jobs, SyncJobRow};
use crate::domain::sync::push_attempts::{list_attempts, ListAttemptsFilter, PushAttemptRow};
use crate::outbox::list_failed;
use crate::AppState;
use platform_sdk::extract_tenant;

// ============================================================================
// Helpers
// ============================================================================

fn correlation_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-correlation-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}

fn flip_error(e: FlipError) -> ApiError {
    match e {
        FlipError::InvalidSide(s) => ApiError::new(
            422,
            "invalid_authority_side",
            format!("Invalid authority side '{}': must be 'platform' or 'external'", s),
        ),
        FlipError::ConnectionNotFound(app_id, provider) => ApiError::not_found(format!(
            "No OAuth connection found for provider '{}' on tenant '{}'",
            provider, app_id
        )),
        FlipError::Database(e) => {
            tracing::error!(error = %e, "sync authority flip DB error");
            ApiError::internal("Internal database error")
        }
    }
}

// ============================================================================
// flip_authority
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct FlipAuthorityRequest {
    pub provider: String,
    pub entity_type: String,
    /// Must be "platform" or "external".
    pub new_side: String,
}

#[derive(Debug, Serialize)]
pub struct FlipAuthorityResponse {
    pub id: Uuid,
    pub app_id: String,
    pub provider: String,
    pub entity_type: String,
    pub previous_authority: String,
    pub new_authority: String,
    pub authority_version: i64,
    pub flipped_by: String,
    pub flipped_at: DateTime<Utc>,
}

pub async fn flip_authority(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<FlipAuthorityRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let flipped_by = match &claims {
        Some(Extension(c)) => c.user_id.to_string(),
        None => "unknown".to_string(),
    };
    let correlation_id = correlation_from_headers(&headers);

    match svc_flip_authority(
        &state.pool,
        &app_id,
        &req.provider,
        &req.entity_type,
        &req.new_side,
        &flipped_by,
        correlation_id,
    )
    .await
    {
        Ok(result) => {
            let resp = FlipAuthorityResponse {
                id: result.row.id,
                app_id: result.row.app_id,
                provider: result.row.provider,
                entity_type: result.row.entity_type,
                previous_authority: result.previous_side,
                new_authority: result.row.authoritative_side,
                authority_version: result.row.authority_version,
                flipped_by: result.row.last_flipped_by.unwrap_or_default(),
                flipped_at: result.row.last_flipped_at.unwrap_or_else(Utc::now),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => flip_error(e).into_response(),
    }
}

// ============================================================================
// get_authority_state
// ============================================================================

/// Response item for a single authority row.
#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorityItem {
    pub provider: String,
    pub entity_type: String,
    pub authoritative_side: String,
    pub authority_version: i64,
    pub last_flipped_by: Option<String>,
    pub last_flipped_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::sync::AuthorityRow> for AuthorityItem {
    fn from(r: crate::domain::sync::AuthorityRow) -> Self {
        Self {
            provider: r.provider,
            entity_type: r.entity_type,
            authoritative_side: r.authoritative_side,
            authority_version: r.authority_version,
            last_flipped_by: r.last_flipped_by,
            last_flipped_at: r.last_flipped_at,
        }
    }
}

/// GET /api/integrations/sync/authority
///
/// Returns all authority rows owned by the caller's tenant. Permission: integrations.sync.read.
pub async fn get_authority_state(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    match list_authority(&state.pool, &app_id).await {
        Ok(rows) => {
            let items: Vec<AuthorityItem> = rows.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(items)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "get_authority_state DB error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ============================================================================
// list_conflicts
// ============================================================================

const CONFLICT_VALID_STATUSES: &[&str] = &["pending", "resolved", "ignored", "unresolvable"];

#[derive(Debug, Deserialize)]
pub struct ConflictsQuery {
    pub provider: Option<String>,
    pub entity_type: Option<String>,
    pub status: Option<String>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

/// Response item for a single conflict row.
#[derive(Debug, Serialize, ToSchema)]
pub struct ConflictItem {
    pub id: Uuid,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    pub conflict_class: String,
    pub status: String,
    pub detected_by: String,
    pub detected_at: DateTime<Utc>,
    pub internal_value: Option<Value>,
    pub external_value: Option<Value>,
    pub internal_id: Option<String>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution_note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<crate::domain::sync::ConflictRow> for ConflictItem {
    fn from(r: crate::domain::sync::ConflictRow) -> Self {
        Self {
            id: r.id,
            provider: r.provider,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            conflict_class: r.conflict_class,
            status: r.status,
            detected_by: r.detected_by,
            detected_at: r.detected_at,
            internal_value: r.internal_value,
            external_value: r.external_value,
            internal_id: r.internal_id,
            resolved_by: r.resolved_by,
            resolved_at: r.resolved_at,
            resolution_note: r.resolution_note,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// GET /api/integrations/sync/conflicts
///
/// Returns app-scoped paginated conflict rows with optional filters.
pub async fn list_conflicts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<ConflictsQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if let Some(ref s) = q.status {
        if !CONFLICT_VALID_STATUSES.contains(&s.as_str()) {
            return ApiError::new(
                422,
                "invalid_status",
                format!(
                    "status must be one of: {}",
                    CONFLICT_VALID_STATUSES.join(", ")
                ),
            )
            .into_response();
        }
    }

    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 200);
    let offset = (page - 1) * page_size;

    match list_conflicts_paged(
        &state.pool,
        &app_id,
        q.provider.as_deref(),
        q.entity_type.as_deref(),
        q.status.as_deref(),
        page_size,
        offset,
    )
    .await
    {
        Ok((rows, total)) => {
            let items: Vec<ConflictItem> = rows.into_iter().map(Into::into).collect();
            Json(PaginatedResponse::new(items, page, page_size, total)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list_conflicts DB error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ============================================================================
// resolve_conflict
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct ResolveConflictBody {
    pub internal_id: String,
    pub resolution_note: Option<String>,
}

/// POST /api/integrations/sync/conflicts/{id}/resolve
pub async fn resolve_conflict(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<Uuid>,
    Json(body): Json<ResolveConflictBody>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let resolved_by = match &claims {
        Some(Extension(c)) => c.user_id.to_string(),
        None => "unknown".to_string(),
    };

    match resolve_conflict_transactional(
        &state.pool,
        &app_id,
        id,
        &resolved_by,
        &body.internal_id,
        body.resolution_note.as_deref(),
    )
    .await
    {
        Ok(row) => (StatusCode::OK, Json(ConflictItem::from(row))).into_response(),
        Err(ResolveConflictError::NotFound(_)) => {
            ApiError::not_found(format!("Conflict {} not found", id)).into_response()
        }
        Err(ResolveConflictError::InvalidTransition(from, to)) => ApiError::new(
            409,
            "invalid_transition",
            format!("Cannot transition conflict from '{}' to '{}'", from, to),
        )
        .into_response(),
        Err(ResolveConflictError::MissingInternalId) => ApiError::new(
            422,
            "missing_internal_id",
            "internal_id is required to resolve a conflict",
        )
        .into_response(),
        Err(ResolveConflictError::UnsupportedEntityType(et)) => ApiError::new(
            422,
            "unsupported_entity_type",
            format!("entity_type '{}' is not supported for conflict resolution", et),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, conflict_id = %id, "resolve_conflict error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ============================================================================
// bulk_resolve_conflicts
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct BulkResolveItemRequest {
    pub conflict_id: Uuid,
    /// "resolve" | "ignore" | "unresolvable"
    pub action: String,
    /// Caller's believed authority version — factored into the server deterministic key.
    pub authority_version: i64,
    pub internal_id: Option<String>,
    pub resolution_note: Option<String>,
    /// Caller-supplied alias.  Stored per-item but never drives server-side dedupe.
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkResolveBody {
    pub items: Vec<BulkResolveItemRequest>,
}

#[derive(Debug, Serialize)]
pub struct BulkResolveResponse {
    pub processed: usize,
    pub outcomes: Vec<BulkResolveOutcome>,
}

/// POST /api/integrations/sync/conflicts/bulk-resolve
///
/// Resolves, ignores, or marks-unresolvable up to 100 conflicts in best-effort
/// bulk fashion.  Each item is processed independently (not transactional across
/// items).  The server always computes the deterministic idempotency key from
/// `conflict_id + action + authority_version`; the caller-supplied `idempotency_key`
/// is stored as a tracking alias only.
pub async fn bulk_resolve_conflicts_handler(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<BulkResolveBody>,
) -> impl IntoResponse {
    if body.items.len() > 100 {
        return ApiError::new(
            422,
            "items_exceed_cap",
            format!("bulk-resolve accepts at most 100 items; got {}", body.items.len()),
        )
        .into_response();
    }

    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };
    let resolved_by = match &claims {
        Some(Extension(c)) => c.user_id.to_string(),
        None => "unknown".to_string(),
    };

    let items: Vec<BulkResolveItem> = body
        .items
        .into_iter()
        .map(|i| BulkResolveItem {
            conflict_id: i.conflict_id,
            action: i.action,
            authority_version: i.authority_version,
            internal_id: i.internal_id,
            resolution_note: i.resolution_note,
            caller_idempotency_key: i.idempotency_key,
        })
        .collect();

    match bulk_resolve_conflicts(&state.pool, &app_id, &resolved_by, items).await {
        Ok(outcomes) => {
            let processed = outcomes.len();
            (StatusCode::OK, Json(BulkResolveResponse { processed, outcomes })).into_response()
        }
        Err(BulkResolveError::ExceedsCapacity(n)) => ApiError::new(
            422,
            "items_exceed_cap",
            format!("bulk-resolve accepts at most 100 items; got {n}"),
        )
        .into_response(),
    }
}

// ============================================================================
// push_entity
// ============================================================================

/// DB-backed QBO token provider — defers refresh to the background worker.
struct DbTokenProvider {
    pool: sqlx::PgPool,
    app_id: String,
}

#[async_trait::async_trait]
impl TokenProvider for DbTokenProvider {
    async fn get_token(&self) -> Result<String, QboError> {
        oauth_service::get_access_token(&self.pool, &self.app_id, "quickbooks")
            .await
            .map_err(|e| QboError::TokenError(e.to_string()))
    }

    async fn refresh_token(&self) -> Result<String, QboError> {
        Err(QboError::AuthFailed)
    }
}

#[derive(Debug, Deserialize)]
pub struct PushEntityRequest {
    pub entity_id: String,
    pub operation: String,
    pub authority_version: i64,
    pub request_fingerprint: String,
    pub payload: Value,
}

/// POST /api/integrations/sync/push/{entity_type}
///
/// Supported entity types: customer, invoice, payment.
/// Returns the full push taxonomy variant as JSON with `"outcome"` discriminant.
pub async fn push_entity(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(entity_type): Path<String>,
    Json(req): Json<PushEntityRequest>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if !matches!(entity_type.as_str(), "customer" | "invoice" | "payment") {
        return ApiError::new(
            422,
            "invalid_entity_type",
            format!(
                "entity_type must be one of: customer, invoice, payment; got '{}'",
                entity_type
            ),
        )
        .into_response();
    }

    if !matches!(req.operation.as_str(), "create" | "update" | "delete") {
        return ApiError::new(
            422,
            "invalid_operation",
            format!(
                "operation must be one of: create, update, delete; got '{}'",
                req.operation
            ),
        )
        .into_response();
    }

    let connection =
        match oauth_service::get_connection_status(&state.pool, &app_id, "quickbooks").await {
            Ok(Some(c)) => c,
            Ok(None) => {
                return ApiError::not_found("No QuickBooks connection found for this tenant")
                    .into_response()
            }
            Err(e) => {
                tracing::error!(error = %e, "push_entity: OAuth lookup error");
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

    let base_url = crate::domain::qbo::cdc::qbo_base_url();
    let tokens: Arc<dyn TokenProvider> = Arc::new(DbTokenProvider {
        pool: state.pool.clone(),
        app_id: app_id.clone(),
    });
    let qbo = Arc::new(QboClient::new(&base_url, &connection.realm_id, tokens));
    let svc = ResolveService::new(qbo);

    let result = match entity_type.as_str() {
        "customer" => {
            svc.push_customer(
                &state.pool,
                &app_id,
                &req.entity_id,
                &req.operation,
                req.authority_version,
                &req.request_fingerprint,
                &req.payload,
            )
            .await
        }
        "invoice" => {
            svc.push_invoice(
                &state.pool,
                &app_id,
                &req.entity_id,
                &req.operation,
                req.authority_version,
                &req.request_fingerprint,
                &req.payload,
            )
            .await
        }
        "payment" => {
            svc.push_payment(
                &state.pool,
                &app_id,
                &req.entity_id,
                &req.operation,
                req.authority_version,
                &req.request_fingerprint,
                &req.payload,
            )
            .await
        }
        _ => unreachable!("entity_type validated above"),
    };

    match result {
        Ok(outcome) => (StatusCode::OK, Json(outcome)).into_response(),
        Err(PushError::DuplicateIntent) => ApiError::new(
            409,
            "duplicate_intent",
            "An equivalent push attempt is already pending for this entity",
        )
        .into_response(),
        Err(PushError::Database(e)) => {
            tracing::error!(error = %e, entity_type = %entity_type, "push_entity: DB error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ============================================================================
// list_dlq
// ============================================================================

const DLQ_VALID_REASONS: &[&str] = &[
    "bus_publish_failed",
    "retry_exhausted",
    "needs_reauth",
    "authority_superseded",
];

#[derive(Debug, Deserialize)]
pub struct DlqQuery {
    pub failure_reason: Option<String>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

/// GET /api/integrations/sync/dlq
pub async fn list_dlq(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<DlqQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    // Validate failure_reason if provided — avoid arbitrary string injection.
    if let Some(ref reason) = q.failure_reason {
        if !DLQ_VALID_REASONS.contains(&reason.as_str()) {
            return ApiError::new(
                422,
                "invalid_failure_reason",
                format!(
                    "failure_reason must be one of: {}",
                    DLQ_VALID_REASONS.join(", ")
                ),
            )
            .into_response();
        }
    }

    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 200);

    match list_failed(
        &state.pool,
        &app_id,
        q.failure_reason.as_deref(),
        page,
        page_size,
    )
    .await
    {
        Ok((rows, total)) => {
            Json(PaginatedResponse::new(rows, page, page_size, total)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list_dlq DB error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ============================================================================
// list_push_attempts
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct PushAttemptsQuery {
    pub provider: Option<String>,
    pub entity_type: Option<String>,
    pub status: Option<String>,
    pub request_fingerprint: Option<String>,
    pub started_after: Option<DateTime<Utc>>,
    pub started_before: Option<DateTime<Utc>>,
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

const PUSH_VALID_STATUSES: &[&str] = &[
    "accepted",
    "inflight",
    "succeeded",
    "failed",
    "unknown_failure",
    "superseded",
    "completed_under_stale_authority",
];

/// Response item for a push attempt — omits no sensitive fields (error_message
/// is operator-facing, not end-user-facing).
#[derive(Debug, Serialize, ToSchema)]
pub struct PushAttemptItem {
    pub id: Uuid,
    pub provider: String,
    pub entity_type: String,
    pub entity_id: String,
    pub operation: String,
    pub authority_version: i64,
    pub request_fingerprint: String,
    pub status: String,
    pub error_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<PushAttemptRow> for PushAttemptItem {
    fn from(r: PushAttemptRow) -> Self {
        Self {
            id: r.id,
            provider: r.provider,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            operation: r.operation,
            authority_version: r.authority_version,
            request_fingerprint: r.request_fingerprint,
            status: r.status,
            error_message: r.error_message,
            started_at: r.started_at,
            completed_at: r.completed_at,
            created_at: r.created_at,
        }
    }
}

/// GET /api/integrations/sync/push-attempts
pub async fn list_push_attempts(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<PushAttemptsQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    if let Some(ref s) = q.status {
        if !PUSH_VALID_STATUSES.contains(&s.as_str()) {
            return ApiError::new(
                422,
                "invalid_status",
                format!(
                    "status must be one of: {}",
                    PUSH_VALID_STATUSES.join(", ")
                ),
            )
            .into_response();
        }
    }

    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 200);

    let filter = ListAttemptsFilter {
        provider: q.provider.as_deref(),
        entity_type: q.entity_type.as_deref(),
        status: q.status.as_deref(),
        request_fingerprint: q.request_fingerprint.as_deref(),
        started_after: q.started_after,
        started_before: q.started_before,
    };

    match list_attempts(&state.pool, &app_id, &filter, page, page_size).await {
        Ok((rows, total)) => {
            let items: Vec<PushAttemptItem> = rows.into_iter().map(Into::into).collect();
            Json(PaginatedResponse::new(items, page, page_size, total)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list_push_attempts DB error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

fn default_page() -> i64 { 1 }
fn default_page_size() -> i64 { 50 }

// ============================================================================
// list_jobs
// ============================================================================

/// Response item for a sync job health row.
#[derive(Debug, Serialize, ToSchema)]
pub struct SyncJobItem {
    pub id: Uuid,
    pub provider: String,
    pub job_name: String,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    pub failure_streak: i32,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl From<SyncJobRow> for SyncJobItem {
    fn from(r: SyncJobRow) -> Self {
        Self {
            id: r.id,
            provider: r.provider,
            job_name: r.job_name,
            last_success_at: r.last_success_at,
            last_failure_at: r.last_failure_at,
            failure_streak: r.failure_streak,
            last_error: r.last_error,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct JobsQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
}

/// GET /api/integrations/sync/jobs
pub async fn list_jobs(
    State(state): State<Arc<AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(q): Query<JobsQuery>,
) -> impl IntoResponse {
    let app_id = match extract_tenant(&claims) {
        Ok(id) => id,
        Err(e) => return e.into_response(),
    };

    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 200);

    match repo_list_jobs(&state.pool, &app_id, page, page_size).await {
        Ok((rows, total)) => {
            let items: Vec<SyncJobItem> = rows.into_iter().map(Into::into).collect();
            Json(PaginatedResponse::new(items, page, page_size, total)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "list_jobs DB error");
            ApiError::internal("Internal database error").into_response()
        }
    }
}

// ============================================================================
// trigger_cdc — dev-local only, admin-guarded
// ============================================================================

/// POST /api/integrations/sync/cdc/trigger
///
/// Force one CDC poll cycle for the caller's QBO tenant. Returns the number of
/// observations processed and conflicts automatically opened by the detector.
///
/// **Dev-local only** — returns `403 Forbidden` when `APP_PROFILE != "dev-local"`.
/// Guards against accidental use in staging/production.
pub async fn trigger_cdc(
    State(state): State<Arc<AppState>>,
    Extension(claims): Extension<VerifiedClaims>,
) -> impl IntoResponse {
    let profile = std::env::var("APP_PROFILE").unwrap_or_default();
    if profile != "dev-local" {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "CDC trigger endpoint is only available in dev-local profile (APP_PROFILE=dev-local)"
            })),
        )
            .into_response();
    }

    let app_id = claims.tenant_id.to_string();

    // Snapshot conflict count before the tick so we can report new conflicts opened.
    let before: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_conflicts WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or((0,));

    let obs_count =
        match qbo_cdc::cdc_tick_for_tenant(&state.pool, &app_id).await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!(app_id = %app_id, error = %e, "CDC trigger failed");
                return ApiError::internal("CDC tick failed").into_response();
            }
        };

    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM integrations_sync_conflicts WHERE app_id = $1",
    )
    .bind(&app_id)
    .fetch_one(&state.pool)
    .await
    .unwrap_or((0,));

    let conflicts_opened = (after.0 - before.0).max(0) as u32;

    Json(serde_json::json!({
        "provider": "quickbooks",
        "observations_processed": obs_count,
        "conflicts_opened": conflicts_opened,
    }))
    .into_response()
}
