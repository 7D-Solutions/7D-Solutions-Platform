use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, Invoice, ListInvoicesQuery, PaginatedResponse};

/// GET /api/ar/invoices/:id - Get invoice by ID
#[utoipa::path(get, path = "/api/ar/invoices/{id}", tag = "Invoices",
    params(("id" = i32, Path, description = "Invoice ID")),
    responses(
        (status = 200, description = "Invoice found", body = Invoice),
        (status = 404, description = "Invoice not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_invoice(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Invoice>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.party_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Invoice {} not found", id))
    })?;

    Ok(Json(invoice))
}

/// GET /api/ar/invoices - List invoices (with optional filtering)
#[utoipa::path(get, path = "/api/ar/invoices", tag = "Invoices",
    params(ListInvoicesQuery),
    responses(
        (status = 200, description = "Paginated invoices", body = PaginatedResponse<Invoice>),
    ),
    security(("bearer" = [])))]
pub async fn list_invoices(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<PaginatedResponse<Invoice>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Count total matching rows
    let mut count_sql = String::from(
        "SELECT COUNT(*) FROM ar_invoices i \
         INNER JOIN ar_customers c ON i.ar_customer_id = c.id \
         WHERE c.app_id = $1",
    );
    let mut bind_idx = 2;
    if query.customer_id.is_some() {
        count_sql.push_str(&format!(" AND i.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.subscription_id.is_some() {
        count_sql.push_str(&format!(" AND i.subscription_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.status.is_some() {
        count_sql.push_str(&format!(" AND i.status = ${bind_idx}"));
    }

    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        count_q = count_q.bind(cid);
    }
    if let Some(sid) = query.subscription_id {
        count_q = count_q.bind(sid);
    }
    if let Some(ref st) = query.status {
        count_q = count_q.bind(st);
    }
    let total_items = count_q.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Database error counting invoices: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    // Fetch data
    let mut data_sql = String::from(
        r#"SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.party_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE c.app_id = $1"#,
    );
    let mut data_idx = 2;
    if query.customer_id.is_some() {
        data_sql.push_str(&format!(" AND i.ar_customer_id = ${data_idx}"));
        data_idx += 1;
    }
    if query.subscription_id.is_some() {
        data_sql.push_str(&format!(" AND i.subscription_id = ${data_idx}"));
        data_idx += 1;
    }
    if query.status.is_some() {
        data_sql.push_str(&format!(" AND i.status = ${data_idx}"));
        data_idx += 1;
    }
    data_sql.push_str(&format!(
        " ORDER BY i.created_at DESC LIMIT ${data_idx} OFFSET ${}",
        data_idx + 1
    ));

    let mut data_q = sqlx::query_as::<_, Invoice>(&data_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        data_q = data_q.bind(cid);
    }
    if let Some(sid) = query.subscription_id {
        data_q = data_q.bind(sid);
    }
    if let Some(ref st) = query.status {
        data_q = data_q.bind(st);
    }
    let invoices = data_q
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing invoices: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        invoices,
        page,
        limit as i64,
        total_items,
    )))
}
