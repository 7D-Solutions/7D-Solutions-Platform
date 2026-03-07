//! Tax HTTP routes (bd-29j + bd-3fy)
//!
//! POST /api/ar/tax/quote  — Request a tax quote for an invoice draft
//! GET  /api/ar/tax/quote   — Look up a cached tax quote by tenant + invoice_id
//! POST /api/ar/tax/commit  — Commit tax when invoice is finalized
//! POST /api/ar/tax/void    — Void committed tax on refund/cancellation

use axum::{
    extract::State, http::StatusCode, response::IntoResponse, routing::post, Extension, Json,
    Router,
};
use security::VerifiedClaims;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::tax::{self, LocalTaxProvider, TaxAddress, TaxLineItem, TaxQuoteRequest};

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
        .with_state(db)
}

// ============================================================================
// POST /api/ar/tax/quote
// ============================================================================

async fn quote_tax_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<QuoteTaxHttpRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err((status, Json(err))) => {
            return (status, Json(ErrorBody { error: err.message })).into_response();
        }
    };

    let correlation_id = body
        .correlation_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let req = TaxQuoteRequest {
        tenant_id: tenant_id.clone(),
        invoice_id: body.invoice_id.clone(),
        customer_id: body.customer_id,
        ship_to: body.ship_to,
        ship_from: body.ship_from,
        line_items: body.line_items,
        currency: body.currency,
        invoice_date: body.invoice_date,
        correlation_id,
    };

    // Check if we already have a cached quote with the same request hash
    let request_hash = tax::compute_request_hash(&req);
    let cached_hit = tax::find_cached_quote(&pool, &tenant_id, &body.invoice_id, &request_hash)
        .await
        .ok()
        .flatten();

    if let Some(cached) = cached_hit {
        let tax_by_line: Vec<tax::TaxByLine> = match serde_json::from_value(cached.tax_by_line) {
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

        let resp = QuoteTaxHttpResponse {
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
        };

        return (StatusCode::OK, Json(resp)).into_response();
    }

    // Cache miss — call local provider
    let provider = LocalTaxProvider;
    match tax::quote_tax_cached(&pool, &provider, &tenant_id, req).await {
        Ok(response) => {
            let resp = QuoteTaxHttpResponse {
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
                provider: "local".to_string(),
                cached: false,
                quoted_at: response.quoted_at,
            };
            (StatusCode::OK, Json(resp)).into_response()
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

async fn lookup_cached_quote(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    axum::extract::Query(query): axum::extract::Query<LookupQuery>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err((status, Json(err))) => {
            return (status, Json(ErrorBody { error: err.message })).into_response();
        }
    };

    // Look up the most recent cached quote for this tenant + invoice_id
    let row = sqlx::query_as::<
        _,
        (
            uuid::Uuid,
            String,
            String,
            String,
            String,
            i64,
            serde_json::Value,
            chrono::DateTime<chrono::Utc>,
        ),
    >(
        r#"
        SELECT id, provider, provider_quote_ref, request_hash, idempotency_key,
               total_tax_minor, tax_by_line, quoted_at
        FROM ar_tax_quote_cache
        WHERE app_id = $1 AND invoice_id = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&tenant_id)
    .bind(&query.invoice_id)
    .fetch_optional(&pool)
    .await;

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
            tracing::error!("Database error loading tax quote: {}", e);
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

async fn commit_tax_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<CommitTaxHttpRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err((status, Json(err))) => {
            return (status, Json(ErrorBody { error: err.message })).into_response();
        }
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

async fn void_tax_handler(
    State(pool): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(body): Json<VoidTaxHttpRequest>,
) -> impl IntoResponse {
    let tenant_id = match super::tenant::extract_tenant(&claims) {
        Ok(id) => id,
        Err((status, Json(err))) => {
            return (status, Json(ErrorBody { error: err.message })).into_response();
        }
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
