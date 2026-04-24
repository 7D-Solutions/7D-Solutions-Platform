//! Tax HTTP routes (bd-29j + bd-3fy)
//!
//! POST /api/ar/tax/quote  — Request a tax quote for an invoice draft
//! GET  /api/ar/tax/quote   — Look up a cached tax quote by tenant + invoice_id
//! POST /api/ar/tax/commit  — Commit tax when invoice is finalized
//! POST /api/ar/tax/void    — Void committed tax on refund/cancellation

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::domain::tax_config as tax_config_repo;
use crate::tax::{
    self, AvalaraConfig, AvalaraProvider, LocalTaxProvider, TaxAddress, TaxLineItem,
    TaxQuoteRequest, ZeroTaxProvider,
};

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct QuoteTaxHttpRequest {
    pub invoice_id: String,
    pub customer_id: String,
    pub ship_to: TaxAddress,
    pub ship_from: TaxAddress,
    pub line_items: Vec<TaxLineItem>,
    pub currency: String,
    pub invoice_date: chrono::DateTime<chrono::Utc>,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct QuoteTaxHttpResponse {
    pub total_tax_minor: i64,
    pub tax_by_line: Vec<TaxByLineHttp>,
    pub provider_quote_ref: String,
    pub provider: String,
    pub cached: bool,
    pub quoted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
pub struct TaxByLineHttp {
    pub line_id: String,
    pub tax_minor: i64,
    pub rate: f64,
    pub jurisdiction: String,
    pub tax_type: String,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Deserialize)]
pub struct LookupQuery {
    pub invoice_id: String,
}

// ============================================================================
// Commit/Void request/response types (bd-3fy)
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct CommitTaxHttpRequest {
    pub invoice_id: String,
    pub customer_id: String,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CommitTaxHttpResponse {
    pub provider_commit_ref: String,
    pub provider_quote_ref: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub already_committed: bool,
}

#[derive(Debug, Deserialize)]
pub struct VoidTaxHttpRequest {
    pub invoice_id: String,
    pub void_reason: String,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VoidTaxHttpResponse {
    pub provider_commit_ref: String,
    pub total_tax_minor: i64,
    pub already_voided: bool,
}

// ============================================================================
// Route builder
// ============================================================================

pub fn tax_router(db: PgPool) -> Router {
    Router::new()
        .route(
            "/api/ar/tax/quote",
            post(quote_tax_handler).get(lookup_cached_quote),
        )
        .route("/api/ar/tax/commit", post(commit_tax_handler))
        .route("/api/ar/tax/void", post(void_tax_handler))
        .route(
            "/api/ar/tax/tenant-config",
            get(super::tax_tenant_config::get_tax_tenant_config)
                .put(super::tax_tenant_config::put_tax_tenant_config),
        )
        .with_state(db)
}

// ============================================================================
// POST /api/ar/tax/quote
// ============================================================================

#[utoipa::path(post, path = "/api/ar/tax/quote", tag = "Tax",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Tax quote", body = serde_json::Value),
        (status = 400, description = "Invalid request", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
pub async fn quote_tax_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<QuoteTaxHttpRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    let tenant_uuid = match uuid::Uuid::parse_str(&tenant_id) {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: "invalid tenant_id in JWT".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Fetch per-tenant config; return default (external_accounting_software) if no row.
    let tenant_cfg = match crate::tax::tenant_config::get(&pool, tenant_uuid).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(error = %e, "Failed to load tenant tax config");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "tax config unavailable".to_string(),
                }),
            )
                .into_response();
        }
    };

    let correlation_id = body
        .correlation_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let invoice_id = body.invoice_id.clone();

    let req = TaxQuoteRequest {
        tenant_id: tenant_id.clone(),
        invoice_id: invoice_id.clone(),
        customer_id: body.customer_id,
        ship_to: body.ship_to,
        ship_from: body.ship_from,
        line_items: body.line_items,
        currency: body.currency,
        invoice_date: body.invoice_date,
        correlation_id,
    };

    // External source: return zero — the external accounting software (QBO/AST) computes tax.
    // No provider call, no cache write; zero is the AR platform's contribution.
    if !tenant_cfg.is_platform_source() {
        let tax_by_line: Vec<TaxByLineHttp> = req
            .line_items
            .iter()
            .map(|l| TaxByLineHttp {
                line_id: l.line_id.clone(),
                tax_minor: 0,
                rate: 0.0,
                jurisdiction: "external".to_string(),
                tax_type: "none".to_string(),
            })
            .collect();

        return (
            StatusCode::OK,
            Json(QuoteTaxHttpResponse {
                total_tax_minor: 0,
                tax_by_line,
                provider_quote_ref: format!("external-{}", uuid::Uuid::new_v4()),
                provider: "external_accounting_software".to_string(),
                cached: false,
                quoted_at: chrono::Utc::now(),
            }),
        )
            .into_response();
    }

    // Platform source: cache keyed by (tenant, invoice, content_hash, config_version).
    // Including config_version ensures cache misses whenever the tenant's config changes,
    // preventing stale provider quotes from being returned after a source/provider flip.
    let request_hash = tax::compute_request_hash(&req);
    let idempotency_key = format!(
        "{}:{}:{}:cv{}",
        tenant_id, invoice_id, request_hash, tenant_cfg.config_version
    );

    let cached_hit =
        tax::find_cached_quote_by_idempotency_key(&pool, &tenant_id, &invoice_id, &idempotency_key)
            .await
            .ok()
            .flatten();

    if let Some(cached) = cached_hit {
        let tax_by_line: Vec<tax::TaxByLine> =
            match serde_json::from_value(cached.tax_by_line) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorBody {
                            error: format!("cached data corrupt: {}", e),
                        }),
                    )
                        .into_response();
                }
            };

        return (
            StatusCode::OK,
            Json(QuoteTaxHttpResponse {
                total_tax_minor: cached.total_tax_minor,
                tax_by_line: tax_by_line
                    .into_iter()
                    .map(|l| TaxByLineHttp {
                        line_id: l.line_id,
                        tax_minor: l.tax_minor,
                        rate: l.rate,
                        jurisdiction: l.jurisdiction,
                        tax_type: l.tax_type,
                    })
                    .collect(),
                provider_quote_ref: cached.provider_quote_ref,
                provider: cached.provider,
                cached: true,
                quoted_at: cached.quoted_at,
            }),
        )
            .into_response();
    }

    // Cache miss — call the configured provider then store with the config_version-aware key.
    // We call the provider directly (not via quote_tax_cached) so the cache entry uses
    // `idempotency_key = "{tenant}:{invoice}:{hash}:cv{version}"`.  The old quote_tax_cached
    // would store with a key that lacks the config_version suffix, creating a split-brain
    // where the pre-check never finds the stored entry.
    use tax::TaxProvider;

    let (provider_name, quote_result): (&str, Result<_, tax::TaxProviderError>) =
        match tenant_cfg.provider_name.as_str() {
            "avalara" => {
                let cfg = match AvalaraConfig::from_env() {
                    Ok(c) => c,
                    Err(_) => {
                        return (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(ErrorBody {
                                error: "Avalara provider not configured for this deployment"
                                    .to_string(),
                            }),
                        )
                            .into_response();
                    }
                };
                ("avalara", AvalaraProvider::new(cfg).quote_tax(req).await)
            }
            "zero" => ("zero", ZeroTaxProvider.quote_tax(req).await),
            _ => ("local", LocalTaxProvider.quote_tax(req).await),
        };

    match quote_result {
        Ok(response) => {
            // Persist with config_version-aware idempotency_key so subsequent requests
            // by the same tenant/invoice/config_version get a cache hit.
            let _ = tax::store_quote_cache(
                &pool,
                &tenant_id,
                &invoice_id,
                &idempotency_key,
                &request_hash,
                provider_name,
                &response,
            )
            .await;

            (
                StatusCode::OK,
                Json(QuoteTaxHttpResponse {
                    total_tax_minor: response.total_tax_minor,
                    tax_by_line: response
                        .tax_by_line
                        .into_iter()
                        .map(|l| TaxByLineHttp {
                            line_id: l.line_id,
                            tax_minor: l.tax_minor,
                            rate: l.rate,
                            jurisdiction: l.jurisdiction,
                            tax_type: l.tax_type,
                        })
                        .collect(),
                    provider_quote_ref: response.provider_quote_ref,
                    provider: provider_name.to_string(),
                    cached: false,
                    quoted_at: response.quoted_at,
                }),
            )
                .into_response()
        }
        Err(e) => {
            let status = match &e {
                tax::TaxProviderError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorBody {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// GET /api/ar/tax/quote?invoice_id=...
// ============================================================================

#[utoipa::path(get, path = "/api/ar/tax/quote", tag = "Tax",
    params(("invoice_id" = String, Query, description = "Invoice ID to look up cached quote")),
    responses(
        (status = 200, description = "Cached tax quote", body = serde_json::Value),
        (status = 404, description = "No cached quote found", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
pub async fn lookup_cached_quote(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    axum::extract::Query(query): axum::extract::Query<LookupQuery>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    // Look up the most recent cached quote for this tenant + invoice_id
    let row = tax_config_repo::lookup_cached_quote(&pool, &tenant_id, &query.invoice_id).await;

    match row {
        Ok(Some(r)) => {
            let tax_by_line: Vec<tax::TaxByLine> = serde_json::from_value(r.6).unwrap_or_default();

            let resp = QuoteTaxHttpResponse {
                total_tax_minor: r.5,
                tax_by_line: tax_by_line
                    .into_iter()
                    .map(|l| TaxByLineHttp {
                        line_id: l.line_id,
                        tax_minor: l.tax_minor,
                        rate: l.rate,
                        jurisdiction: l.jurisdiction,
                        tax_type: l.tax_type,
                    })
                    .collect(),
                provider_quote_ref: r.2,
                provider: r.1,
                cached: true,
                quoted_at: r.7,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody {
                error: "No cached tax quote found".to_string(),
            }),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Database error loading tax quote");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorBody {
                    error: "Internal database error".to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// POST /api/ar/tax/commit (bd-3fy)
// ============================================================================

#[utoipa::path(post, path = "/api/ar/tax/commit", tag = "Tax",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Tax committed", body = serde_json::Value),
        (status = 404, description = "No quote found", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
pub async fn commit_tax_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<CommitTaxHttpRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    let correlation_id = body
        .correlation_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let provider = LocalTaxProvider;
    match crate::finalization::commit_tax_for_invoice(
        &pool,
        &provider,
        &tenant_id,
        &body.invoice_id,
        &body.customer_id,
        &correlation_id,
    )
    .await
    {
        Ok(result) => {
            let resp = CommitTaxHttpResponse {
                provider_commit_ref: result.provider_commit_ref,
                provider_quote_ref: result.provider_quote_ref,
                total_tax_minor: result.total_tax_minor,
                currency: result.currency,
                already_committed: result.was_already_committed,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            use crate::finalization::TaxCommitError;
            let status = match &e {
                TaxCommitError::NoQuote { .. } => StatusCode::NOT_FOUND,
                TaxCommitError::AlreadyVoided { .. } => StatusCode::CONFLICT,
                TaxCommitError::AlreadyCommitted { .. } => StatusCode::OK,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorBody {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}

// ============================================================================
// POST /api/ar/tax/void (bd-3fy)
// ============================================================================

#[utoipa::path(post, path = "/api/ar/tax/void", tag = "Tax",
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Tax voided", body = serde_json::Value),
    ),
    security(("bearer" = [])))]
pub async fn void_tax_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<VoidTaxHttpRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };

    let correlation_id = body
        .correlation_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let provider = LocalTaxProvider;
    match crate::finalization::void_tax_for_invoice(
        &pool,
        &provider,
        &tenant_id,
        &body.invoice_id,
        &body.void_reason,
        &correlation_id,
    )
    .await
    {
        Ok(result) => {
            let resp = VoidTaxHttpResponse {
                provider_commit_ref: result.provider_commit_ref,
                total_tax_minor: result.total_tax_minor,
                already_voided: result.was_already_voided,
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            use crate::finalization::TaxCommitError;
            let status = match &e {
                TaxCommitError::NotCommitted { .. } => StatusCode::NOT_FOUND,
                TaxCommitError::AlreadyVoided { .. } => StatusCode::OK,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(ErrorBody {
                    error: e.to_string(),
                }),
            )
                .into_response()
        }
    }
}
