use axum::{extract::State, Extension, Json};
use chrono::Utc;
use event_bus::TracingContext;
use platform_http_contracts::{ApiError, PaginatedResponse};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::{extract_actor, with_request_id};
use crate::{auth::PortalClaims, db::portal_repo, outbox::enqueue_portal_event};

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateStatusCardRequest {
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub title: String,
    pub status: String,
    #[serde(default)]
    pub details: serde_json::Value,
    pub source: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AcknowledgeRequest {
    pub document_id: Option<Uuid>,
    pub status_card_id: Option<Uuid>,
    pub ack_type: String,
    pub notes: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize, sqlx::FromRow, ToSchema)]
pub struct StatusCard {
    pub id: Uuid,
    pub entity_type: String,
    pub entity_id: Option<Uuid>,
    pub title: String,
    pub status: String,
    pub details: serde_json::Value,
    pub source: String,
    pub occurred_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatusFeedQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    post, path = "/portal/admin/status-cards", tag = "Admin",
    request_body = CreateStatusCardRequest,
    responses(
        (status = 200, description = "Status card created"),
        (status = 401, body = ApiError), (status = 403, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn create_status_card(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<CreateStatusCardRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor = extract_actor(&claims).map_err(|e| with_request_id(e, &ctx))?;
    if actor.tenant_id != req.tenant_id {
        return Err(with_request_id(ApiError::forbidden("forbidden"), &ctx));
    }

    let id = Uuid::new_v4();
    portal_repo::insert_status_card(
        &state.pool,
        id,
        req.tenant_id,
        req.party_id,
        &req.entity_type,
        req.entity_id,
        &req.title,
        &req.status,
        &req.details,
        &req.source,
        Utc::now(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(Json(serde_json::json!({"status_card_id": id})))
}

#[utoipa::path(
    get, path = "/portal/status/feed", tag = "Status",
    params(StatusFeedQuery),
    responses(
        (status = 200, description = "Paginated status cards", body = PaginatedResponse<StatusCard>),
        (status = 401, body = ApiError), (status = 403, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_status_cards(
    State(state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
    ctx: Option<Extension<TracingContext>>,
    axum::extract::Query(query): axum::extract::Query<StatusFeedQuery>,
) -> Result<Json<PaginatedResponse<StatusCard>>, ApiError> {
    if !claims.scopes.iter().any(|s| {
        s == platform_contracts::portal_identity::scopes::DOCUMENTS_READ
            || s == platform_contracts::portal_identity::scopes::ORDERS_READ
            || s == platform_contracts::portal_identity::scopes::INVOICES_READ
            || s == platform_contracts::portal_identity::scopes::SHIPMENTS_READ
            || s == platform_contracts::portal_identity::scopes::QUALITY_READ
    }) {
        return Err(with_request_id(ApiError::forbidden("forbidden"), &ctx));
    }

    let tenant_id = Uuid::parse_str(&claims.tenant_id)
        .map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;
    let party_id = Uuid::parse_str(&claims.party_id)
        .map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * page_size;

    let total: i64 = portal_repo::count_status_cards(&state.pool, tenant_id, party_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal status db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    let cards = portal_repo::list_status_cards(&state.pool, tenant_id, party_id, page_size, offset)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "portal status db error");
            with_request_id(ApiError::internal("Database error"), &ctx)
        })?;

    Ok(Json(PaginatedResponse::new(cards, page, page_size, total)))
}

#[utoipa::path(
    post, path = "/portal/acknowledgments", tag = "Status",
    request_body = AcknowledgeRequest,
    responses(
        (status = 200, description = "Acknowledgment recorded"),
        (status = 400, body = ApiError), (status = 401, body = ApiError),
        (status = 403, body = ApiError), (status = 404, body = ApiError),
        (status = 409, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn acknowledge(
    State(state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<AcknowledgeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !claims.scopes.iter().any(|s| {
        s == platform_contracts::portal_identity::scopes::ACKNOWLEDGMENTS_WRITE
            || s == platform_contracts::portal_identity::scopes::DOCUMENTS_ACKNOWLEDGE
    }) {
        return Err(with_request_id(ApiError::forbidden("forbidden"), &ctx));
    }

    if req.idempotency_key.trim().is_empty() {
        return Err(with_request_id(
            ApiError::bad_request("idempotency_key_required"),
            &ctx,
        ));
    }

    let tenant_id = Uuid::parse_str(&claims.tenant_id)
        .map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;
    let party_id = Uuid::parse_str(&claims.party_id)
        .map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;
    let portal_user_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;

    let existing =
        portal_repo::find_idempotency(&state.pool, tenant_id, "acknowledge", &req.idempotency_key)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "portal status db error");
                with_request_id(ApiError::internal("Database error"), &ctx)
            })?;

    if let Some(response) = existing {
        return Ok(Json(response));
    }

    if let Some(doc_id) = req.document_id {
        let linked: Option<(Uuid,)> =
            portal_repo::find_document_link(&state.pool, tenant_id, party_id, doc_id)
                .await
                .map_err(|e| {
                    tracing::error!(error = %e, "portal status db error");
                    with_request_id(ApiError::internal("Database error"), &ctx)
                })?;

        if linked.is_none() {
            return Err(with_request_id(ApiError::not_found("not_found"), &ctx));
        }
    }

    let ack_id = Uuid::new_v4();
    let mut tx = state.pool.begin().await.map_err(|e| {
        tracing::error!(error = %e, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    portal_repo::insert_acknowledgment_tx(
        &mut tx,
        ack_id,
        tenant_id,
        party_id,
        portal_user_id,
        req.document_id,
        req.status_card_id,
        &req.ack_type,
        req.notes.as_deref(),
        &req.idempotency_key,
    )
    .await
    .map_err(|err| {
        if let sqlx::Error::Database(db) = &err {
            if db.constraint()
                == Some("portal_acknowledgments_tenant_id_party_id_idempotency_key_key")
            {
                return with_request_id(ApiError::conflict("duplicate_acknowledgment"), &ctx);
            }
        }
        tracing::error!(error = %err, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    enqueue_portal_event(
        &mut tx,
        tenant_id,
        Some(portal_user_id),
        "portal.acknowledgment.recorded",
        serde_json::json!({
            "acknowledgment_id": ack_id,
            "tenant_id": tenant_id,
            "party_id": party_id,
            "portal_user_id": portal_user_id,
            "document_id": req.document_id,
            "status_card_id": req.status_card_id,
            "ack_type": req.ack_type,
        }),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    let response = serde_json::json!({
        "acknowledgment_id": ack_id,
        "document_id": req.document_id,
        "status_card_id": req.status_card_id,
        "ack_type": req.ack_type,
    });

    portal_repo::insert_idempotency_no_conflict_tx(
        &mut tx,
        tenant_id,
        "acknowledge",
        &req.idempotency_key,
        &response,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!(error = %e, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(Json(response))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LinkDocumentRequest {
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub document_id: Uuid,
    pub display_title: Option<String>,
}

#[utoipa::path(
    post, path = "/portal/admin/docs/link", tag = "Admin",
    request_body = LinkDocumentRequest,
    responses(
        (status = 200, description = "Document linked"),
        (status = 401, body = ApiError), (status = 403, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn link_document(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    ctx: Option<Extension<TracingContext>>,
    Json(req): Json<LinkDocumentRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let actor = extract_actor(&claims).map_err(|e| with_request_id(e, &ctx))?;
    if actor.tenant_id != req.tenant_id {
        return Err(with_request_id(ApiError::forbidden("forbidden"), &ctx));
    }

    let id = Uuid::new_v4();
    portal_repo::upsert_document_link(
        &state.pool,
        id,
        req.tenant_id,
        req.party_id,
        req.document_id,
        req.display_title.as_deref(),
        actor.user_id,
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal status db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    Ok(Json(serde_json::json!({"linked": true})))
}
