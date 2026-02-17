//! Tax quote HTTP routes (bd-29j)
//!
//! POST /api/ar/tax/quote — Request a tax quote for an invoice draft
//! GET  /api/ar/tax/quote  — Look up a cached tax quote by app_id + invoice_id

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::tax::{
    self, LocalTaxProvider, TaxAddress, TaxLineItem, TaxQuoteRequest,
};

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct QuoteTaxHttpRequest {
    pub app_id: String,
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
    pub app_id: String,
    pub invoice_id: String,
}

// ============================================================================
// Route builder
// ============================================================================

pub fn tax_router(db: PgPool) -> Router {
    Router::new()
        .route("/api/ar/tax/quote", post(quote_tax_handler).get(lookup_cached_quote))
        .with_state(db)
}

// ============================================================================
// POST /api/ar/tax/quote
// ============================================================================

async fn quote_tax_handler(
    State(pool): State<PgPool>,
    Json(body): Json<QuoteTaxHttpRequest>,
) -> impl IntoResponse {
    let correlation_id = body
        .correlation_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let req = TaxQuoteRequest {
        tenant_id: body.app_id.clone(),
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
    let cached_hit = tax::find_cached_quote(&pool, &body.app_id, &body.invoice_id, &request_hash)
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
    match tax::quote_tax_cached(&pool, &provider, &body.app_id, req).await {
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
            (status, Json(ErrorBody { error: e.to_string() })).into_response()
        }
    }
}

// ============================================================================
// GET /api/ar/tax/quote?app_id=...&invoice_id=...
// ============================================================================

async fn lookup_cached_quote(
    State(pool): State<PgPool>,
    axum::extract::Query(query): axum::extract::Query<LookupQuery>,
) -> impl IntoResponse {
    // Look up the most recent cached quote for this app_id + invoice_id
    let row = sqlx::query_as::<_, (
        uuid::Uuid,
        String,
        String,
        String,
        String,
        i64,
        serde_json::Value,
        chrono::DateTime<chrono::Utc>,
    )>(
        r#"
        SELECT id, provider, provider_quote_ref, request_hash, idempotency_key,
               total_tax_minor, tax_by_line, quoted_at
        FROM ar_tax_quote_cache
        WHERE app_id = $1 AND invoice_id = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(&query.app_id)
    .bind(&query.invoice_id)
    .fetch_optional(&pool)
    .await;

    match row {
        Ok(Some(r)) => {
            let tax_by_line: Vec<tax::TaxByLine> =
                serde_json::from_value(r.6).unwrap_or_default();

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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: format!("Database error: {}", e),
            }),
        )
            .into_response(),
    }
}
