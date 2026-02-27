use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Datelike;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{
    AddPaymentMethodRequest, Customer, ErrorResponse, ListPaymentMethodsQuery, PaymentMethod,
    UpdatePaymentMethodRequest,
};
use crate::tilled::TilledClient;

/// POST /api/ar/payment-methods - Add a new payment method
pub async fn add_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Json(req): Json<AddPaymentMethodRequest>,
) -> Result<(StatusCode, Json<PaymentMethod>), (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Validate required fields
    if req.tilled_payment_method_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "tilled_payment_method_id is required",
            )),
        ));
    }

    // Verify customer exists and belongs to app
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch customer: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Customer {} not found", req.ar_customer_id),
            )),
        )
    })?;

    // Check if payment method already exists (upsert pattern)
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to check payment method: {}", e),
            )),
        )
    })?;

    // Local-first, low-code pattern: status starts as 'pending_sync'.
    // The provider webhook (payment_method.attached) will set card details
    // (brand, last4, etc.) and transition status to 'active'.
    let payment_method = if let Some(_pm) = existing {
        // Update existing record (reactivate if deleted)
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to update payment method: {}", e),
                )),
            )
        })?
    } else {
        // Create new payment method record
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to create payment method: {}", e),
                )),
            )
        })?
    };

    tracing::info!(
        "Added payment method {} for customer {}",
        payment_method.id,
        req.ar_customer_id
    );

    Ok((StatusCode::CREATED, Json(payment_method)))
}

/// GET /api/ar/payment-methods/:id - Get payment method by ID
pub async fn get_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<PaymentMethod>, (StatusCode, Json<ErrorResponse>)> {
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    Ok(Json(payment_method))
}

/// GET /api/ar/payment-methods - List payment methods (with optional filtering)
pub async fn list_payment_methods(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListPaymentMethodsQuery>,
) -> Result<Json<Vec<PaymentMethod>>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let payment_methods = match (query.customer_id, query.status) {
        (Some(customer_id), Some(ref status)) => {
            // Filter by both customer and status
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.ar_customer_id = $2 AND pm.status = $3
                    AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
                LIMIT $4 OFFSET $5
                "#,
            )
            .bind(&app_id)
            .bind(customer_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (Some(customer_id), None) => {
            // Filter by customer only (most common case)
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.ar_customer_id = $2 AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(&app_id)
            .bind(customer_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, Some(ref status)) => {
            // Filter by status only (unusual case)
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.status = $2 AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(&app_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, None) => {
            // List all for app (rare case)
            sqlx::query_as::<_, PaymentMethod>(
                r#"
                SELECT
                    pm.id, pm.app_id, pm.ar_customer_id, pm.tilled_payment_method_id,
                    pm.status, pm.type, pm.brand, pm.last4, pm.exp_month, pm.exp_year,
                    pm.bank_name, pm.bank_last4, pm.is_default, pm.metadata,
                    pm.deleted_at, pm.created_at, pm.updated_at
                FROM ar_payment_methods pm
                INNER JOIN ar_customers c ON pm.ar_customer_id = c.id
                WHERE c.app_id = $1 AND pm.deleted_at IS NULL
                ORDER BY pm.is_default DESC, pm.created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(&app_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
    }
    .map_err(|e| {
        tracing::error!("Database error listing payment methods: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list payment methods: {}", e),
            )),
        )
    })?;

    Ok(Json(payment_methods))
}

/// PUT /api/ar/payment-methods/:id - Update payment method
pub async fn update_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<UpdatePaymentMethodRequest>,
) -> Result<Json<PaymentMethod>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Verify payment method exists and belongs to app
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    // Validate at least one field is being updated
    if req.metadata.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "No valid fields to update",
            )),
        ));
    }

    // Update metadata only (other fields come from Tilled)
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update payment method: {}", e),
            )),
        )
    })?;

    tracing::info!("Updated payment method {}", id);

    Ok(Json(payment_method))
}

/// DELETE /api/ar/payment-methods/:id - Delete payment method (soft delete)
pub async fn delete_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Verify payment method exists and belongs to app
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    // Guard: block detach if there are pending or authorized charges using this PM
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to check blocking charges: {}", e),
            )),
        )
    })?;

    if blocking_charge_count.unwrap_or(0) > 0 {
        return Err((
            StatusCode::CONFLICT,
            Json(ErrorResponse::new(
                "conflict",
                format!(
                    "Cannot detach payment method: {} pending/authorized charge(s) exist for this customer",
                    blocking_charge_count.unwrap_or(0)
                ),
            )),
        ));
    }

    // Detach from Tilled if we have a provider ID
    if !payment_method.tilled_payment_method_id.is_empty() {
        let client = TilledClient::from_env(&app_id).map_err(|e| {
            tracing::error!("Failed to create Tilled client: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "provider_config_error",
                    format!("Failed to initialize payment provider: {}", e),
                )),
            )
        })?;

        if let Err(e) = client
            .detach_payment_method(&payment_method.tilled_payment_method_id)
            .await
        {
            tracing::error!(
                "Tilled detach failed for payment method {}: {:?}",
                id,
                e
            );
            return Err((
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse::new(
                    "provider_error",
                    format!("Payment provider detach failed: {}", e),
                )),
            ));
        }
    }

    // Soft delete the payment method
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to delete payment method: {}", e),
            )),
        )
    })?;

    // If this was the default, clear customer fast-path
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to update customer: {}", e),
                )),
            )
        })?;
    }

    tracing::info!("Deleted payment method {}", id);

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/ar/payment-methods/:id/set-default - Set payment method as default
pub async fn set_default_payment_method(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<PaymentMethod>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Verify payment method exists, belongs to app, and is active
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch payment method: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Payment method {} not found", id),
            )),
        )
    })?;

    // Verify payment method is active
    if payment_method.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!(
                    "Cannot set payment method with status {} as default",
                    payment_method.status
                ),
            )),
        ));
    }

    // Validate card expiration if it's a card
    if payment_method.payment_type == "card" {
        if let (Some(exp_month), Some(exp_year)) =
            (payment_method.exp_month, payment_method.exp_year)
        {
            let now = chrono::Utc::now();
            let current_year = now.year();
            let current_month = now.month() as i32;

            if exp_year < current_year || (exp_year == current_year && exp_month < current_month) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse::new(
                        "validation_error",
                        "Card is expired and cannot be set as default",
                    )),
                ));
            }
        }
    }

    // Use a transaction to atomically update defaults
    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to begin transaction",
            )),
        )
    })?;

    // Clear all other defaults for this customer
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to clear default flags",
            )),
        )
    })?;

    // Set this payment method as default
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to set default flag",
            )),
        )
    })?;

    // Update customer fast-path
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to update customer default",
            )),
        )
    })?;

    // Commit transaction
    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to commit transaction",
            )),
        )
    })?;

    tracing::info!(
        "Set payment method {} as default for customer {}",
        id,
        payment_method.ar_customer_id
    );

    Ok(Json(updated_pm))
}
