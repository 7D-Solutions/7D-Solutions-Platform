use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Datelike;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{
    AddPaymentMethodRequest, ApiError, Customer, PaymentMethod, UpdatePaymentMethodRequest,
};
use crate::tilled::TilledClient;

/// POST /api/ar/payment-methods - Add a new payment method
#[utoipa::path(post, path = "/api/ar/payment-methods", tag = "Payment Methods",
    request_body = AddPaymentMethodRequest,
    responses(
        (status = 201, description = "Payment method added", body = PaymentMethod),
        (status = 400, description = "Validation error", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn add_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<AddPaymentMethodRequest>,
) -> Result<(StatusCode, Json<PaymentMethod>), ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    if req.tilled_payment_method_id.is_empty() {
        return Err(ApiError::bad_request("tilled_payment_method_id is required"));
    }

    let _customer = sqlx::query_as::<_, Customer>(
        r#"
        SELECT
            id, app_id, external_customer_id, tilled_customer_id, status,
            email, name, default_payment_method_id, payment_method_type,
            metadata, update_source, updated_by, delinquent_since,
            grace_period_end, next_retry_at, retry_attempt_count,
            created_at, updated_at
        FROM ar_customers
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(req.ar_customer_id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching customer: {:?}", e);
        ApiError::internal("Internal database error")
    })?
    .ok_or_else(|| {
        ApiError::not_found(format!("Customer {} not found", req.ar_customer_id))
    })?;

    let existing = sqlx::query_as::<_, PaymentMethod>(
        r#"
        SELECT
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        FROM ar_payment_methods
        WHERE tilled_payment_method_id = $1
        "#,
    )
    .bind(&req.tilled_payment_method_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking payment method: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let payment_method = if let Some(_pm) = existing {
        sqlx::query_as::<_, PaymentMethod>(
            r#"
            UPDATE ar_payment_methods
            SET app_id = $1, ar_customer_id = $2, status = 'pending_sync',
                deleted_at = NULL, updated_at = NOW()
            WHERE tilled_payment_method_id = $3
            RETURNING
                id, app_id, ar_customer_id, tilled_payment_method_id,
                status, type, brand, last4, exp_month, exp_year,
                bank_name, bank_last4, is_default, metadata,
                deleted_at, created_at, updated_at
            "#,
        )
        .bind(&app_id)
        .bind(req.ar_customer_id)
        .bind(&req.tilled_payment_method_id)
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update payment method: {:?}", e);
            ApiError::internal("Internal database error")
        })?
    } else {
        sqlx::query_as::<_, PaymentMethod>(
            r#"
            INSERT INTO ar_payment_methods (
                app_id, ar_customer_id, tilled_payment_method_id,
                type, status, is_default, metadata, created_at, updated_at
            )
            VALUES ($1, $2, $3, 'card', 'pending_sync', FALSE, '{}', NOW(), NOW())
            RETURNING
                id, app_id, ar_customer_id, tilled_payment_method_id,
                status, type, brand, last4, exp_month, exp_year,
                bank_name, bank_last4, is_default, metadata,
                deleted_at, created_at, updated_at
            "#,
        )
        .bind(&app_id)
        .bind(req.ar_customer_id)
        .bind(&req.tilled_payment_method_id)
        .fetch_one(&db)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create payment method: {:?}", e);
            ApiError::internal("Internal database error")
        })?
    };

    tracing::info!(
        "Added payment method {} for customer {}",
        payment_method.id,
        req.ar_customer_id
    );

    Ok((StatusCode::CREATED, Json(payment_method)))
}

/// PUT /api/ar/payment-methods/:id - Update payment method
#[utoipa::path(put, path = "/api/ar/payment-methods/{id}", tag = "Payment Methods",
    params(("id" = i32, Path, description = "Payment method ID")),
    request_body = UpdatePaymentMethodRequest,
    responses(
        (status = 200, description = "Payment method updated", body = PaymentMethod),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn update_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<UpdatePaymentMethodRequest>,
) -> Result<Json<PaymentMethod>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let existing = sqlx::query_as::<_, PaymentMethod>(
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

    if req.metadata.is_none() {
        return Err(ApiError::bad_request("No valid fields to update"));
    }

    let metadata = req.metadata.or(existing.metadata);

    let payment_method = sqlx::query_as::<_, PaymentMethod>(
        r#"
        UPDATE ar_payment_methods
        SET metadata = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(metadata)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update payment method: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tracing::info!("Updated payment method {}", id);

    Ok(Json(payment_method))
}

/// DELETE /api/ar/payment-methods/:id - Delete payment method (soft delete)
#[utoipa::path(delete, path = "/api/ar/payment-methods/{id}", tag = "Payment Methods",
    params(("id" = i32, Path, description = "Payment method ID")),
    responses(
        (status = 204, description = "Payment method deleted"),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
        (status = 409, description = "Pending charges exist", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn delete_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<StatusCode, ApiError> {
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

    let blocking_charge_count: Option<i64> = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM ar_charges
        WHERE ar_customer_id = $1 AND status IN ('pending', 'authorized')
        "#,
    )
    .bind(payment_method.ar_customer_id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error checking blocking charges: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    if blocking_charge_count.unwrap_or(0) > 0 {
        return Err(ApiError::conflict(format!(
            "Cannot detach payment method: {} pending/authorized charge(s) exist for this customer",
            blocking_charge_count.unwrap_or(0)
        )));
    }

    if !payment_method.tilled_payment_method_id.is_empty() {
        let client = TilledClient::from_env(&app_id).map_err(|e| {
            tracing::error!("Failed to create Tilled client: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

        if let Err(e) = client
            .detach_payment_method(&payment_method.tilled_payment_method_id)
            .await
        {
            tracing::error!("Tilled detach failed for payment method {}: {:?}", id, e);
            return Err(ApiError::new(
                502,
                "provider_error",
                format!("Payment provider detach failed: {}", e),
            ));
        }
    }

    sqlx::query(
        r#"
        UPDATE ar_payment_methods
        SET deleted_at = NOW(), is_default = FALSE, updated_at = NOW()
        WHERE id = $1
        "#,
    )
    .bind(id)
    .execute(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to delete payment method: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    if payment_method.is_default {
        sqlx::query(
            r#"
            UPDATE ar_customers
            SET default_payment_method_id = NULL, payment_method_type = NULL, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(payment_method.ar_customer_id)
        .execute(&db)
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to clear default payment method from customer: {:?}",
                e
            );
            ApiError::internal("Internal database error")
        })?;
    }

    tracing::info!("Deleted payment method {}", id);

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/ar/payment-methods/:id/set-default - Set payment method as default
#[utoipa::path(post, path = "/api/ar/payment-methods/{id}/set-default", tag = "Payment Methods",
    params(("id" = i32, Path, description = "Payment method ID")),
    responses(
        (status = 200, description = "Default payment method set", body = PaymentMethod),
        (status = 400, description = "Invalid payment method state", body = platform_http_contracts::ApiError),
        (status = 404, description = "Not found", body = platform_http_contracts::ApiError),
    ),
    security(("bearer" = [])))]
pub async fn set_default_payment_method(
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

    if payment_method.status != "active" {
        return Err(ApiError::bad_request(format!(
                    "Cannot set payment method with status {} as default",
                    payment_method.status
                )));
    }

    if payment_method.payment_type == "card" {
        if let (Some(exp_month), Some(exp_year)) =
            (payment_method.exp_month, payment_method.exp_year)
        {
            let now = chrono::Utc::now();
            let current_year = now.year();
            let current_month = now.month() as i32;

            if exp_year < current_year || (exp_year == current_year && exp_month < current_month) {
                return Err(ApiError::bad_request("Card is expired and cannot be set as default"));
            }
        }
    }

    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    sqlx::query(
        r#"
        UPDATE ar_payment_methods
        SET is_default = FALSE, updated_at = NOW()
        WHERE ar_customer_id = $1 AND app_id = $2
        "#,
    )
    .bind(payment_method.ar_customer_id)
    .bind(&app_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to clear default flags: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let updated_pm = sqlx::query_as::<_, PaymentMethod>(
        r#"
        UPDATE ar_payment_methods
        SET is_default = TRUE, updated_at = NOW()
        WHERE id = $1
        RETURNING
            id, app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            bank_name, bank_last4, is_default, metadata,
            deleted_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to set default flag: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    sqlx::query(
        r#"
        UPDATE ar_customers
        SET default_payment_method_id = $1, payment_method_type = $2, updated_at = NOW()
        WHERE id = $3
        "#,
    )
    .bind(&payment_method.tilled_payment_method_id)
    .bind(&payment_method.payment_type)
    .bind(payment_method.ar_customer_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update customer default: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit transaction: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tracing::info!(
        "Set payment method {} as default for customer {}",
        id,
        payment_method.ar_customer_id
    );

    Ok(Json(updated_pm))
}
