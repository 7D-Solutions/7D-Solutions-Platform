use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, ListPaymentMethodsQuery, PaginatedResponse, PaymentMethod};

/// GET /api/ar/payment-methods/:id - Get payment method by ID
#[utoipa::path(get, path = "/api/ar/payment-methods/{id}", tag = "Payment Methods",
    params(("id" = i32, Path, description = "Payment method ID")),
    responses(
        (status = 200, description = "Payment method found", body = PaymentMethod),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn get_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<PaymentMethod>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let payment_method = sqlx::query_as::<_, PaymentMethod>(
        r#"
        SELECT
            pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
            pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
            pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
            pm.deleted_at, pm.created_at, pm.updated_at
        FROM ar_payment_methods pm
        INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
        WHERE pm.id = $1 AND c.app_id = $2 AND pm.deleted_at IS NULL
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching payment method: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Payment method {} not found", id))
    })?;

    Ok(Json(payment_method))
}

/// GET /api/ar/payment-methods - List payment methods (with optional filtering)
#[utoipa::path(get, path = "/api/ar/payment-methods", tag = "Payment Methods",
    params(ListPaymentMethodsQuery),
    responses(
        (status = 200, description = "Paginated payment methods", body = PaginatedResponse<PaymentMethod>),
    ),
    security(("bearer" = [])))]
pub async fn list_payment_methods(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListPaymentMethodsQuery>,
) -> Result<Json<PaginatedResponse<PaymentMethod>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Count total matching rows
    let mut count_sql = String::from(
        "SELECT COUNT(*) FROM ar_payment_methods pm \
         INNER JOIN ar_customers c ON pm.ar_customer_id = c.id \
         WHERE c.app_id = $1 AND pm.deleted_at IS NULL",
    );
    let mut bind_idx = 2;
    if query.customer_id.is_some() {
        count_sql.push_str(&format!(" AND pm.ar_customer_id = ${bind_idx}"));
        bind_idx += 1;
    }
    if query.status.is_some() {
        count_sql.push_str(&format!(" AND pm.status = ${bind_idx}"));
    }

    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        count_q = count_q.bind(cid);
    }
    if let Some(ref st) = query.status {
        count_q = count_q.bind(st);
    }
    let total_items = count_q.fetch_one(&db).await.map_err(|e| {
        tracing::error!("Database error counting payment methods: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    // Fetch data
    let mut data_sql = String::from(
        r#"SELECT
            pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
            pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
            pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
            pm.deleted_at, pm.created_at, pm.updated_at
        FROM ar_payment_methods pm
        INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
        WHERE c.app_id = $1 AND pm.deleted_at IS NULL"#,
    );
    let mut data_idx = 2;
    if query.customer_id.is_some() {
        data_sql.push_str(&format!(" AND pm.ar_customer_id = ${data_idx}"));
        data_idx += 1;
    }
    if query.status.is_some() {
        data_sql.push_str(&format!(" AND pm.status = ${data_idx}"));
        data_idx += 1;
    }
    data_sql.push_str(&format!(
        " ORDER BY pm.is_default DESC, pm.created_at DESC LIMIT ${data_idx} OFFSET ${}",
        data_idx + 1
    ));

    let mut data_q = sqlx::query_as::<_, PaymentMethod>(&data_sql).bind(&app_id);
    if let Some(cid) = query.customer_id {
        data_q = data_q.bind(cid);
    }
    if let Some(ref st) = query.status {
        data_q = data_q.bind(st);
    }
    let payment_methods = data_q
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing payment methods: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(Json(PaginatedResponse::new(
        payment_methods,
        page,
        limit as i64,
        total_items,
    )))
}
