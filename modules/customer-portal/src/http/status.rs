use axum::{extract::State, Extension, Json};
use chrono::Utc;
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{auth::PortalClaims, outbox::enqueue_portal_event};

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct AcknowledgeRequest {
    pub document_id: Option<Uuid>,
    pub status_card_id: Option<Uuid>,
    pub ack_type: String,
    pub notes: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
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

pub async fn create_status_card(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<CreateStatusCardRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let Extension(actor) = claims.ok_or_else(crate::auth::unauthorized)?;
    if actor.tenant_id != req.tenant_id {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ));
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO portal_status_feed (id, tenant_id, party_id, entity_type, entity_id, title, status, details, source, occurred_at) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(id)
    .bind(req.tenant_id)
    .bind(req.party_id)
    .bind(req.entity_type)
    .bind(req.entity_id)
    .bind(req.title)
    .bind(req.status)
    .bind(req.details)
    .bind(req.source)
    .bind(Utc::now())
    .execute(&state.pool)
    .await
    .map_err(internal_err)?;

    Ok(Json(serde_json::json!({"status_card_id": id})))
}

pub async fn list_status_cards(
    State(state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    if !claims
        .scopes
        .iter()
        .any(|s| {
            s == platform_contracts::portal_identity::scopes::DOCUMENTS_READ
                || s == platform_contracts::portal_identity::scopes::ORDERS_READ
                || s == platform_contracts::portal_identity::scopes::INVOICES_READ
                || s == platform_contracts::portal_identity::scopes::SHIPMENTS_READ
                || s == platform_contracts::portal_identity::scopes::QUALITY_READ
        })
    {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ));
    }

    let tenant_id = Uuid::parse_str(&claims.tenant_id).map_err(|_| unauthorized())?;
    let party_id = Uuid::parse_str(&claims.party_id).map_err(|_| unauthorized())?;

    let cards = sqlx::query_as::<_, StatusCard>(
        "SELECT id, entity_type, entity_id, title, status, details, source, occurred_at \
         FROM portal_status_feed WHERE tenant_id = $1 AND party_id = $2 ORDER BY occurred_at DESC LIMIT 100",
    )
    .bind(tenant_id)
    .bind(party_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_err)?;

    Ok(Json(serde_json::json!({"status_cards": cards})))
}

pub async fn acknowledge(
    State(state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
    Json(req): Json<AcknowledgeRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    if !claims.scopes.iter().any(|s| {
        s == platform_contracts::portal_identity::scopes::ACKNOWLEDGMENTS_WRITE
            || s == platform_contracts::portal_identity::scopes::DOCUMENTS_ACKNOWLEDGE
    }) {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ));
    }

    if req.idempotency_key.trim().is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "idempotency_key_required"})),
        ));
    }

    let tenant_id = Uuid::parse_str(&claims.tenant_id).map_err(|_| unauthorized())?;
    let party_id = Uuid::parse_str(&claims.party_id).map_err(|_| unauthorized())?;
    let portal_user_id = Uuid::parse_str(&claims.sub).map_err(|_| unauthorized())?;

    let existing = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT response FROM portal_idempotency WHERE tenant_id = $1 AND operation = 'acknowledge' AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(&req.idempotency_key)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_err)?;

    if let Some(response) = existing {
        return Ok(Json(response));
    }

    if let Some(doc_id) = req.document_id {
        let linked: Option<(Uuid,)> = sqlx::query_as(
            "SELECT document_id FROM portal_document_links WHERE tenant_id = $1 AND party_id = $2 AND document_id = $3",
        )
        .bind(tenant_id)
        .bind(party_id)
        .bind(doc_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal_err)?;

        if linked.is_none() {
            return Err((
                axum::http::StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "not_found"})),
            ));
        }
    }

    let ack_id = Uuid::new_v4();
    let mut tx = state.pool.begin().await.map_err(internal_err)?;

    sqlx::query(
        "INSERT INTO portal_acknowledgments \
         (id, tenant_id, party_id, portal_user_id, document_id, status_card_id, ack_type, notes, idempotency_key) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
    )
    .bind(ack_id)
    .bind(tenant_id)
    .bind(party_id)
    .bind(portal_user_id)
    .bind(req.document_id)
    .bind(req.status_card_id)
    .bind(&req.ack_type)
    .bind(&req.notes)
    .bind(&req.idempotency_key)
    .execute(&mut *tx)
    .await
    .map_err(|err| {
        if let sqlx::Error::Database(db) = &err {
            if db.constraint() == Some("portal_acknowledgments_tenant_id_party_id_idempotency_key_key")
            {
                return (
                    axum::http::StatusCode::CONFLICT,
                    Json(serde_json::json!({"error": "duplicate_acknowledgment"})),
                );
            }
        }
        internal_err(err)
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
    .map_err(internal_err)?;

    let response = serde_json::json!({
        "acknowledgment_id": ack_id,
        "document_id": req.document_id,
        "status_card_id": req.status_card_id,
        "ack_type": req.ack_type,
    });

    sqlx::query(
        "INSERT INTO portal_idempotency (tenant_id, operation, idempotency_key, response) VALUES ($1,'acknowledge',$2,$3)",
    )
    .bind(tenant_id)
    .bind(&req.idempotency_key)
    .bind(&response)
    .execute(&mut *tx)
    .await
    .map_err(internal_err)?;

    tx.commit().await.map_err(internal_err)?;

    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
pub struct LinkDocumentRequest {
    pub tenant_id: Uuid,
    pub party_id: Uuid,
    pub document_id: Uuid,
    pub display_title: Option<String>,
}

pub async fn link_document(
    State(state): State<Arc<crate::AppState>>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<LinkDocumentRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let Extension(actor) = claims.ok_or_else(crate::auth::unauthorized)?;
    if actor.tenant_id != req.tenant_id {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ));
    }

    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO portal_document_links (id, tenant_id, party_id, document_id, display_title, created_by) \
         VALUES ($1,$2,$3,$4,$5,$6) \
         ON CONFLICT (tenant_id, party_id, document_id) DO NOTHING",
    )
    .bind(id)
    .bind(req.tenant_id)
    .bind(req.party_id)
    .bind(req.document_id)
    .bind(req.display_title)
    .bind(actor.user_id)
    .execute(&state.pool)
    .await
    .map_err(internal_err)?;

    Ok(Json(serde_json::json!({"linked": true})))
}

fn unauthorized() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized"})),
    )
}

fn internal_err(err: sqlx::Error) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    tracing::error!("portal status db error: {err}");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal_error"})),
    )
}
