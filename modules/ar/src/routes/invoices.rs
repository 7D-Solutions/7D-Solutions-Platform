use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use sqlx::PgPool;

use crate::models::{
    CreateInvoiceRequest, Customer, ErrorResponse, FinalizeInvoiceRequest, Invoice,
    ListInvoicesQuery, PaymentCollectionRequestedPayload, Subscription, UpdateInvoiceRequest,
    GlPostingLine, GlPostingRequestPayload,
};

/// POST /api/ar/invoices - Create a new invoice
pub async fn create_invoice(
    State(db): State<PgPool>,
    Json(req): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<Invoice>), (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Validate required fields
    if req.amount_cents < 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "amount_cents must be non-negative",
            )),
        ));
    }

    let status = req.status.unwrap_or_else(|| "draft".to_string());
    let valid_statuses = ["draft", "open", "paid", "void", "uncollectible"];
    if !valid_statuses.contains(&status.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!("status must be one of: {}", valid_statuses.join(", ")),
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
    .bind(app_id)
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

    // Verify subscription exists if provided
    if let Some(subscription_id) = req.subscription_id {
        let _subscription = sqlx::query_as::<_, Subscription>(
            r#"
            SELECT
                s.id, s.app_id, s.ar_customer_id, s.tilled_subscription_id,
                s.plan_id, s.plan_name, s.price_cents, s.status, s.interval_unit, s.interval_count,
                s.billing_cycle_anchor, s.current_period_start, s.current_period_end,
                s.cancel_at_period_end, s.cancel_at, s.canceled_at, s.ended_at,
                s.payment_method_id, s.payment_method_type, s.metadata,
                s.update_source, s.updated_by, s.created_at, s.updated_at
            FROM ar_subscriptions s
            WHERE s.id = $1 AND s.app_id = $2 AND s.ar_customer_id = $3
            "#,
        )
        .bind(subscription_id)
        .bind(app_id)
        .bind(req.ar_customer_id)
        .fetch_optional(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching subscription: {:?}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    format!("Failed to fetch subscription: {}", e),
                )),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse::new(
                    "not_found",
                    format!(
                        "Subscription {} not found for customer {}",
                        subscription_id, req.ar_customer_id
                    ),
                )),
            )
        })?;
    }

    // Generate unique Tilled invoice ID
    let tilled_invoice_id = format!("in_{}_{}", app_id, uuid::Uuid::new_v4());
    let currency = req.currency.unwrap_or_else(|| "usd".to_string());

    // Create invoice
    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, party_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW(), NOW())
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, party_id, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(&tilled_invoice_id)
    .bind(req.ar_customer_id)
    .bind(req.subscription_id)
    .bind(&status)
    .bind(req.amount_cents)
    .bind(&currency)
    .bind(req.due_at)
    .bind(req.metadata)
    .bind(req.billing_period_start)
    .bind(req.billing_period_end)
    .bind(req.line_item_details)
    .bind(req.compliance_codes)
    .bind(req.correlation_id)
    .bind(req.party_id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to create invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to create invoice: {}", e),
            )),
        )
    })?;

    tracing::info!(
        "Created invoice {} for customer {} (amount: {})",
        invoice.id,
        req.ar_customer_id,
        req.amount_cents
    );

    Ok((StatusCode::CREATED, Json(invoice)))
}

/// GET /api/ar/invoices/:id - Get invoice by ID
pub async fn get_invoice(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
) -> Result<Json<Invoice>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch invoice: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Invoice {} not found", id),
            )),
        )
    })?;

    Ok(Json(invoice))
}

/// GET /api/ar/invoices - List invoices (with optional filtering)
pub async fn list_invoices(
    State(db): State<PgPool>,
    Query(query): Query<ListInvoicesQuery>,
) -> Result<Json<Vec<Invoice>>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    // Build query based on filters
    let invoices = match (query.customer_id, query.subscription_id, query.status) {
        (Some(customer_id), _, Some(ref status)) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.correlation_id, i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1 AND i.ar_customer_id = $2 AND i.status = $3
                ORDER BY i.created_at DESC
                LIMIT $4 OFFSET $5
                "#,
            )
            .bind(app_id)
            .bind(customer_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (Some(customer_id), _, None) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.correlation_id, i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1 AND i.ar_customer_id = $2
                ORDER BY i.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
            .bind(customer_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, Some(subscription_id), _) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.correlation_id, i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1 AND i.subscription_id = $2
                ORDER BY i.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
            .bind(subscription_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, None, Some(ref status)) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.correlation_id, i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1 AND i.status = $2
                ORDER BY i.created_at DESC
                LIMIT $3 OFFSET $4
                "#,
            )
            .bind(app_id)
            .bind(status)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
        (None, None, None) => {
            sqlx::query_as::<_, Invoice>(
                r#"
                SELECT
                    i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
                    i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
                    i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
                    i.correlation_id, i.created_at, i.updated_at
                FROM ar_invoices i
                INNER JOIN ar_customers c ON i.ar_customer_id = c.id
                WHERE c.app_id = $1
                ORDER BY i.created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(app_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&db)
            .await
        }
    }
    .map_err(|e| {
        tracing::error!("Database error listing invoices: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to list invoices: {}", e),
            )),
        )
    })?;

    Ok(Json(invoices))
}

/// PUT /api/ar/invoices/:id - Update invoice
pub async fn update_invoice(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<UpdateInvoiceRequest>,
) -> Result<Json<Invoice>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Verify invoice exists and belongs to app
    let existing = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch invoice: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Invoice {} not found", id),
            )),
        )
    })?;

    // Validate at least one field is being updated
    if req.status.is_none()
        && req.amount_cents.is_none()
        && req.due_at.is_none()
        && req.metadata.is_none()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                "No valid fields to update",
            )),
        ));
    }

    // Build update based on provided fields
    let status = req.status.unwrap_or(existing.status);
    let amount_cents = req.amount_cents.unwrap_or(existing.amount_cents);
    let due_at = req.due_at.or(existing.due_at);
    let metadata = req.metadata.or(existing.metadata);

    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        UPDATE ar_invoices
        SET status = $1, amount_cents = $2, due_at = $3, metadata = $4, updated_at = NOW()
        WHERE id = $5
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, created_at, updated_at
        "#,
    )
    .bind(&status)
    .bind(amount_cents)
    .bind(due_at)
    .bind(metadata)
    .bind(id)
    .fetch_one(&db)
    .await
    .map_err(|e| {
        tracing::error!("Failed to update invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to update invoice: {}", e),
            )),
        )
    })?;

    tracing::info!("Updated invoice {}", id);

    Ok(Json(invoice))
}

/// POST /api/ar/invoices/:id/finalize - Mark invoice as finalized (open or paid)
pub async fn finalize_invoice(
    State(db): State<PgPool>,
    Path(id): Path<i32>,
    Json(req): Json<FinalizeInvoiceRequest>,
) -> Result<Json<Invoice>, (StatusCode, Json<ErrorResponse>)> {
    // TODO: Extract app_id from auth middleware
    let app_id = "test-app"; // Placeholder

    // Verify invoice exists and belongs to app
    let existing = sqlx::query_as::<_, Invoice>(
        r#"
        SELECT
            i.id, i.app_id, i.tilled_invoice_id, i.ar_customer_id, i.subscription_id,
            i.status, i.amount_cents, i.currency, i.due_at, i.paid_at, i.hosted_url, i.metadata,
            i.billing_period_start, i.billing_period_end, i.line_item_details, i.compliance_codes,
            i.correlation_id, i.created_at, i.updated_at
        FROM ar_invoices i
        INNER JOIN ar_customers c ON i.ar_customer_id = c.id
        WHERE i.id = $1 AND c.app_id = $2
        "#,
    )
    .bind(id)
    .bind(app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch invoice: {}", e),
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Invoice {} not found", id),
            )),
        )
    })?;

    // Only draft invoices can be finalized to open
    if existing.status != "draft" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!("Cannot finalize invoice with status {}", existing.status),
            )),
        ));
    }

    let paid_at = req.paid_at.or_else(|| Some(chrono::Utc::now().naive_utc()));

    // Begin transaction for atomicity (bd-umnu: invoice mutation + outbox must commit together)
    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to begin transaction: {}", e),
            )),
        )
    })?;


    let invoice = sqlx::query_as::<_, Invoice>(
        r#"
        UPDATE ar_invoices
        SET status = 'open', paid_at = $1, updated_at = NOW()
        WHERE id = $2
        RETURNING
            id, app_id, tilled_invoice_id, ar_customer_id, subscription_id,
            status, amount_cents, currency, due_at, paid_at, hosted_url, metadata,
            billing_period_start, billing_period_end, line_item_details, compliance_codes,
            correlation_id, created_at, updated_at
        "#,
    )
    .bind(paid_at)
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to finalize invoice: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to finalize invoice: {}", e),
            )),
        )
    })?;

    tracing::info!("Finalized invoice {}", id);

    // Fetch customer's default payment method
    let customer_payment_method: Option<String> = sqlx::query_scalar(
        "SELECT default_payment_method_id FROM ar_customers WHERE id = $1"
    )
    .bind(invoice.ar_customer_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("Failed to fetch customer payment method: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                format!("Failed to fetch customer payment method: {}", e),
            )),
        )
    })?
    .flatten();

    // Emit ar.payment.collection.requested event to outbox
    let event_payload = PaymentCollectionRequestedPayload {
        invoice_id: invoice.id.to_string(),
        customer_id: invoice.ar_customer_id.to_string(),
        amount_minor: invoice.amount_cents,
        currency: invoice.currency.to_uppercase(),
        payment_method_id: customer_payment_method,
    };

    let event_envelope = crate::events::envelope::create_ar_envelope(
        uuid::Uuid::new_v4(),
        app_id.to_string(),
        "payment.collection.requested".to_string(),
        uuid::Uuid::new_v4().to_string(),
        None,
        "DATA_MUTATION".to_string(), // Phase 16: Requests payment collection (financially significant)
        event_payload,
    );

    crate::events::outbox::enqueue_event_tx(
        &mut tx,
        "payment.collection.requested",
        "invoice",
        &invoice.id.to_string(),
        &event_envelope,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to enqueue payment.collection.requested event: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("outbox_error", format!("Failed to enqueue event: {}", e))),
        )
    })?;


    // Emit gl.posting.requested event to GL module
    let gl_payload = GlPostingRequestPayload {
        posting_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        currency: invoice.currency.to_uppercase(),
        source_doc_type: "AR_INVOICE".to_string(),
        source_doc_id: invoice.id.to_string(),
        description: format!("Invoice {} for customer {}", invoice.id, invoice.ar_customer_id),
        lines: vec![
            // Debit: Accounts Receivable
            GlPostingLine {
                account_ref: "1100".to_string(),
                debit: invoice.amount_cents,
                credit: 0,
                memo: Some(format!("AR Invoice {}", invoice.id)),
            },
            // Credit: Revenue
            GlPostingLine {
                account_ref: "4000".to_string(),
                debit: 0,
                credit: invoice.amount_cents,
                memo: Some(format!("Revenue from Invoice {}", invoice.id)),
            },
        ],
    };

    let gl_envelope = crate::events::envelope::create_ar_envelope(
        uuid::Uuid::new_v4(),
        app_id.to_string(),
        "gl.posting.requested".to_string(),
        uuid::Uuid::new_v4().to_string(),
        None,
        "DATA_MUTATION".to_string(), // Phase 16: Requests GL journal entry (financially significant)
        gl_payload,
    );

    crate::events::outbox::enqueue_event_tx(
        &mut tx,
        "gl.posting.requested",
        "invoice",
        &invoice.id.to_string(),
        &gl_envelope,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to enqueue gl.posting.requested event: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("outbox_error", format!("Failed to enqueue event: {}", e))),
        )
    })?;


    // Commit transaction atomically
    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit transaction: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("transaction_error", format!("Failed to commit transaction: {}", e))),
        )
    })?;

    Ok(Json(invoice))
}
