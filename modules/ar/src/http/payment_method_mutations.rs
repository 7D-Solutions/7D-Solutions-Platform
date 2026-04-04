use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Datelike;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::domain::{customers, payment_methods};
use crate::models::{
    AddPaymentMethodRequest, ApiError, PaymentMethod, UpdatePaymentMethodRequest,
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

    let _customer = customers::fetch_customer(&db, req.ar_customer_id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching customer: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| {
            ApiError::not_found(format!("Customer {} not found", req.ar_customer_id))
        })?;

    let existing = payment_methods::find_by_tilled_id(&db, &req.tilled_payment_method_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error checking payment method: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let payment_method = if existing.is_some() {
        payment_methods::reattach(&db, &app_id, req.ar_customer_id, &req.tilled_payment_method_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to update payment method: {:?}", e);
                ApiError::internal("Internal database error")
            })?
    } else {
        payment_methods::insert_pending(&db, &app_id, req.ar_customer_id, &req.tilled_payment_method_id)
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

    let existing = payment_methods::fetch_with_tenant(&db, id, &app_id)
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

    let payment_method = payment_methods::update_metadata(&db, id, metadata)
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

    let payment_method = payment_methods::fetch_with_tenant(&db, id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching payment method: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| {
            ApiError::not_found(format!("Payment method {} not found", id))
        })?;

    let blocking_charge_count = payment_methods::count_blocking_charges(&db, payment_method.ar_customer_id)
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

    payment_methods::soft_delete(&db, id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete payment method: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    if payment_method.is_default {
        customers::clear_default_payment_method(&db, payment_method.ar_customer_id)
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

    let payment_method = payment_methods::fetch_with_tenant(&db, id, &app_id)
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

    payment_methods::clear_default_flags(&mut *tx, payment_method.ar_customer_id, &app_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to clear default flags: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let updated_pm = payment_methods::set_default_flag(&mut *tx, id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set default flag: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    customers::set_default_payment_method(
        &mut *tx,
        payment_method.ar_customer_id,
        &payment_method.tilled_payment_method_id,
        &payment_method.payment_type,
    )
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
