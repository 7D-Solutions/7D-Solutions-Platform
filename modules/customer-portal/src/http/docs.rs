use axum::{extract::State, Extension, Json};
use event_bus::TracingContext;
use platform_client_doc_mgmt::DistributionsClient;
use platform_http_contracts::{ApiError, PaginatedResponse};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::tenant::with_request_id;
use crate::auth::PortalClaims;

#[derive(Debug, Serialize, ToSchema)]
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

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DocsQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[utoipa::path(
    get, path = "/portal/docs", tag = "Documents",
    params(DocsQuery),
    responses(
        (status = 200, description = "Paginated documents", body = PaginatedResponse<PortalDocumentView>),
        (status = 401, body = ApiError), (status = 403, body = ApiError),
    ),
    security(("bearer" = [])),
)]
pub async fn list_documents(
    State(state): State<Arc<crate::AppState>>,
    PortalClaims(claims): PortalClaims,
    ctx: Option<Extension<TracingContext>>,
    Extension(doc_mgmt_client): Extension<Arc<DistributionsClient>>,
    axum::extract::Query(query): axum::extract::Query<DocsQuery>,
) -> Result<Json<PaginatedResponse<PortalDocumentView>>, ApiError> {
    if !claims
        .scopes
        .iter()
        .any(|s| s == platform_contracts::portal_identity::scopes::DOCUMENTS_READ)
    {
        return Err(with_request_id(ApiError::forbidden("forbidden"), &ctx));
    }

    let tenant_id =
        Uuid::parse_str(&claims.tenant_id).map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;
    let party_id =
        Uuid::parse_str(&claims.party_id).map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;
    let user_id =
        Uuid::parse_str(&claims.sub).map_err(|_| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;

    let user_email = sqlx::query_as::<_, PortalUserEmailRow>(
        "SELECT email FROM portal_users WHERE id = $1 AND tenant_id = $2 AND party_id = $3",
    )
    .bind(user_id)
    .bind(tenant_id)
    .bind(party_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal docs db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?
    .ok_or_else(|| with_request_id(ApiError::unauthorized("unauthorized"), &ctx))?;

    let links = sqlx::query_as::<_, PortalDocLinkRow>(
        "SELECT document_id, display_title FROM portal_document_links WHERE tenant_id = $1 AND party_id = $2 ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .bind(party_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "portal docs db error");
        with_request_id(ApiError::internal("Database error"), &ctx)
    })?;

    let mut visible = Vec::new();
    for link in links {
        if let Some(dist) =
            fetch_authorized_distribution(&doc_mgmt_client, &ctx, tenant_id, link.document_id, &user_email.email)
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

    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    let total = visible.len() as i64;
    let start = ((page - 1) * page_size) as usize;
    let page_items: Vec<_> = visible.into_iter().skip(start).take(page_size as usize).collect();

    Ok(Json(PaginatedResponse::new(page_items, page, page_size, total)))
}

async fn fetch_authorized_distribution(
    client: &DistributionsClient,
    ctx: &Option<Extension<TracingContext>>,
    tenant_id: Uuid,
    document_id: Uuid,
    user_email: &str,
) -> Result<Option<platform_client_doc_mgmt::DocumentDistribution>, ApiError> {
    let claims = platform_sdk::PlatformClient::service_claims(tenant_id);
    let payload = match client.list_distributions(&claims, document_id).await {
        Ok(resp) => resp,
        Err(platform_sdk::ClientError::Api { .. } | platform_sdk::ClientError::Unexpected { .. }) => {
            return Ok(None);
        }
        Err(e) => {
            tracing::error!(error = %e, "portal docs fetch failed");
            return Err(with_request_id(
                ApiError::new(503, "service_unavailable", "doc_mgmt_unavailable"),
                ctx,
            ));
        }
    };

    let authorized = payload
        .distributions
        .into_iter()
        .find(|d| d.recipient_ref.eq_ignore_ascii_case(user_email));

    Ok(authorized)
}
