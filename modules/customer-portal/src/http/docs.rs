use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::PortalClaims;

#[derive(Debug, Deserialize)]
struct DocMgmtDistributionList {
    distributions: Vec<DocMgmtDistribution>,
}

#[derive(Debug, Deserialize)]
struct DocMgmtDistribution {
    id: Uuid,
    recipient_ref: String,
    status: String,
}

#[derive(Debug, Serialize)]
pub struct PortalDocumentView {
    pub document_id: Uuid,
    pub distribution_id: Uuid,
    pub display_title: Option<String>,
    pub status: String,
}

#[derive(Debug, sqlx::FromRow)]
struct PortalDocLinkRow {
    document_id: Uuid,
    display_title: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct PortalUserEmailRow {
    email: String,
}

pub async fn list_documents(
    State(state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    if !claims
        .scopes
        .iter()
        .any(|s| s == platform_contracts::portal_identity::scopes::DOCUMENTS_READ)
    {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "forbidden"})),
        ));
    }

    let tenant_id = Uuid::parse_str(&claims.tenant_id).map_err(|_| unauthorized())?;
    let party_id = Uuid::parse_str(&claims.party_id).map_err(|_| unauthorized())?;
    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| unauthorized())?;

    let user_email = sqlx::query_as::<_, PortalUserEmailRow>(
        "SELECT email FROM portal_users WHERE id = $1 AND tenant_id = $2 AND party_id = $3",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal_err)?
    .ok_or_else(unauthorized)?;

    let links = sqlx::query_as::<_, PortalDocLinkRow>(
        "SELECT document_id, display_title FROM portal_document_links WHERE tenant_id = $1 AND party_id = $2 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .bind(party_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal_err)?;

    let mut visible = Vec::new();
    for link in links {
        if let Some(dist) = fetch_authorized_distribution(&state, link.document_id, &user_email.email)
            .await?
        {
            visible.push(PortalDocumentView {
                document_id: link.document_id,
                distribution_id: dist.id,
                display_title: link.display_title,
                status: dist.status,
            });
        }
    }

    Ok(Json(serde_json::json!({"documents": visible})))
}

async fn fetch_authorized_distribution(
    state: &crate::AppState,
    document_id: Uuid,
    user_email: &str,
) -> Result<Option<DocMgmtDistribution>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let url = format!(
        "{}/api/documents/{}/distributions",
        state.config.doc_mgmt_base_url.trim_end_matches('/'),
        document_id
    );

    let client = reqwest::Client::new();
    let mut req = client.get(url);

    if let Some(token) = state.config.doc_mgmt_bearer_token.as_ref() {
        req = req.bearer_auth(token);
    }

    let response = req.send().await.map_err(|e| {
        tracing::error!("portal docs fetch failed: {e}");
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "doc_mgmt_unavailable"})),
        )
    })?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let payload: DocMgmtDistributionList = response.json().await.map_err(|e| {
        tracing::error!("portal docs decode failed: {e}");
        (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "doc_mgmt_unavailable"})),
        )
    })?;

    let authorized = payload
        .distributions
        .into_iter()
        .find(|d| d.recipient_ref.eq_ignore_ascii_case(user_email));

    Ok(authorized)
}

fn unauthorized() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({"error": "unauthorized"})),
    )
}

fn internal_err(err: sqlx::Error) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    tracing::error!("portal docs db error: {err}");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "internal_error"})),
    )
}
