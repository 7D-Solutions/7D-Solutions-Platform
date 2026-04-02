//! Invoice service — business logic, validation, and event orchestration.
//!
//! Handlers call into this layer. SQL goes through [`super::repo`].
//! All writes follow Guard → Mutation → Outbox atomicity.

use event_bus::TracingContext;
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::events::contracts::{
    build_invoice_opened_envelope, InvoiceLifecyclePayload, EVENT_TYPE_INVOICE_OPENED,
};
use crate::models::{
    ApiError, CreateInvoiceRequest, FinalizeInvoiceRequest, GlPostingLine,
    GlPostingRequestPayload, Invoice, ListInvoicesQuery, PaginatedResponse,
    PaymentCollectionRequestedPayload, UpdateInvoiceRequest,
};

use super::repo;

// ============================================================================
// Reads
// ============================================================================

/// Get a single invoice by ID with tenant isolation.
pub async fn get_invoice(
    db: &PgPool,
    app_id: &str,
    id: i32,
) -> Result<Invoice, ApiError> {
    repo::fetch_invoice_with_tenant(db, id, app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching invoice: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Invoice {} not found", id)))
}

/// List invoices with optional filtering and pagination.
pub async fn list_invoices(
    db: &PgPool,
    app_id: &str,
    query: ListInvoicesQuery,
) -> Result<PaginatedResponse<Invoice>, ApiError> {
    let limit = query.limit.unwrap_or(50).min(100);
    let offset = query.offset.unwrap_or(0).max(0);

    let total_items = repo::count_invoices(db, app_id, &query)
        .await
        .map_err(|e| {
            tracing::error!("Database error counting invoices: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let invoices = repo::fetch_invoices_page(db, app_id, &query, limit, offset)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing invoices: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    let page = (offset as i64 / limit as i64) + 1;
    Ok(PaginatedResponse::new(invoices, page, limit as i64, total_items))
}

// ============================================================================
// Writes
// ============================================================================

/// Create a new invoice with Guard → Mutation → Outbox atomicity.
pub async fn create_invoice(
    db: &PgPool,
    app_id: &str,
    claims: Option<&VerifiedClaims>,
    tracing_ctx: TracingContext,
    req: CreateInvoiceRequest,
) -> Result<Invoice, ApiError> {
    // Validate required fields
    if req.amount_cents < 0 {
        return Err(ApiError::bad_request("amount_cents must be non-negative"));
    }

    let status = req.status.unwrap_or_else(|| "draft".to_string());
    let valid_statuses = ["draft", "open", "paid", "void", "uncollectible"];
    if !valid_statuses.contains(&status.as_str()) {
        return Err(ApiError::bad_request(format!(
            "status must be one of: {}",
            valid_statuses.join(", ")
        )));
    }

    // Guard: customer must exist and belong to app
    repo::fetch_customer(db, req.ar_customer_id, app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching customer: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| {
            ApiError::not_found(format!("Customer {} not found", req.ar_customer_id))
        })?;

    // Guard: subscription must exist if provided
    if let Some(subscription_id) = req.subscription_id {
        repo::fetch_subscription(db, subscription_id, app_id, req.ar_customer_id)
            .await
            .map_err(|e| {
                tracing::error!("Database error fetching subscription: {:?}", e);
                ApiError::internal("Internal database error")
            })?
            .ok_or_else(|| {
                ApiError::not_found(format!(
                    "Subscription {} not found for customer {}",
                    subscription_id, req.ar_customer_id
                ))
            })?;
    }

    // Guard: party_id must exist in Party Master if provided
    if let Some(pid) = req.party_id {
        let url = crate::integrations::party_client::party_master_url();
        let verified = claims
            .ok_or_else(|| ApiError::unauthorized("Missing authentication"))?;
        crate::integrations::party_client::verify_party(&url, pid, app_id, verified)
            .await
            .map_err(|e| {
                use crate::integrations::party_client::PartyClientError;
                tracing::warn!("Party validation failed for invoice create: {}", e);
                match &e {
                    PartyClientError::ServiceUnavailable(_) => {
                        ApiError::new(503, "party_service_unavailable", e.to_string())
                    }
                    _ => ApiError::new(422, "party_not_found", e.to_string()),
                }
            })?;
    }

    // Mutation + Outbox (atomic transaction)
    let tilled_invoice_id = format!("in_{}_{}", app_id, uuid::Uuid::new_v4());
    let currency = req.currency.unwrap_or_else(|| "usd".to_string());

    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let invoice = repo::insert_invoice(
        &mut *tx,
        app_id,
        &tilled_invoice_id,
        req.ar_customer_id,
        req.subscription_id,
        &status,
        req.amount_cents,
        &currency,
        req.due_at,
        req.metadata,
        req.billing_period_start,
        req.billing_period_end,
        req.line_item_details
            .map(|items| serde_json::to_value(items).expect("Vec<InvoiceLineItem> always serializes")),
        req.compliance_codes,
        req.correlation_id,
        req.party_id,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to create invoice: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    // Emit ar.invoice_opened event
    let idem_key = format!("ar.events.ar.invoice_opened:{}", invoice.id);
    let event_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, idem_key.as_bytes());
    let event_payload = InvoiceLifecyclePayload {
        invoice_id: invoice.id.to_string(),
        customer_id: invoice.ar_customer_id.to_string(),
        app_id: app_id.to_string(),
        amount_cents: invoice.amount_cents,
        currency: invoice.currency.clone(),
        created_at: invoice.created_at,
        due_at: invoice.due_at,
        paid_at: invoice.paid_at,
    };
    let envelope = build_invoice_opened_envelope(
        event_id,
        app_id.to_string(),
        uuid::Uuid::new_v4().to_string(),
        None,
        event_payload,
    )
    .with_tracing_context(&tracing_ctx);
    crate::events::outbox::enqueue_event_tx_idempotent(
        &mut tx,
        EVENT_TYPE_INVOICE_OPENED,
        "invoice",
        &invoice.id.to_string(),
        &envelope,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to enqueue invoice_opened event: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit create_invoice transaction: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    tracing::info!(
        "Created invoice {} for customer {} (amount: {}), invoice_opened event enqueued",
        invoice.id,
        req.ar_customer_id,
        req.amount_cents
    );

    Ok(invoice)
}

/// Update invoice fields.
pub async fn update_invoice(
    db: &PgPool,
    app_id: &str,
    id: i32,
    req: UpdateInvoiceRequest,
) -> Result<Invoice, ApiError> {
    // Guard: invoice must exist and belong to app
    let existing = repo::fetch_invoice_for_mutation(db, id, app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching invoice: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Invoice {} not found", id)))?;

    // Validate at least one field is being updated
    if req.status.is_none()
        && req.amount_cents.is_none()
        && req.due_at.is_none()
        && req.metadata.is_none()
    {
        return Err(ApiError::bad_request("No valid fields to update"));
    }

    let status = req.status.unwrap_or(existing.status);
    let amount_cents = req.amount_cents.unwrap_or(existing.amount_cents);
    let due_at = req.due_at.or(existing.due_at);
    let metadata = req.metadata.or(existing.metadata);

    let invoice = repo::update_invoice_fields(db, id, &status, amount_cents, due_at, metadata)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update invoice: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    tracing::info!("Updated invoice {}", id);

    Ok(invoice)
}

/// Finalize a draft invoice: set status to 'open', emit payment collection
/// and GL posting events atomically.
pub async fn finalize_invoice(
    db: &PgPool,
    app_id: &str,
    tracing_ctx: TracingContext,
    id: i32,
    req: FinalizeInvoiceRequest,
) -> Result<Invoice, ApiError> {
    // Guard: invoice must exist, belong to app, and be in draft status
    let existing = repo::fetch_invoice_for_mutation(db, id, app_id)
        .await
        .map_err(|e| {
            tracing::error!("Database error fetching invoice: {:?}", e);
            ApiError::internal("Internal database error")
        })?
        .ok_or_else(|| ApiError::not_found(format!("Invoice {} not found", id)))?;

    if existing.status != "draft" {
        return Err(ApiError::bad_request(format!(
            "Cannot finalize invoice with status {}",
            existing.status
        )));
    }

    let paid_at = req.paid_at.or_else(|| Some(chrono::Utc::now().naive_utc()));

    // Mutation + Outbox (atomic transaction)
    let mut tx = db.begin().await.map_err(|e| {
        tracing::error!("Failed to begin transaction: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    let invoice = repo::set_invoice_finalized(&mut *tx, id, paid_at)
        .await
        .map_err(|e| {
            tracing::error!("Failed to finalize invoice: {:?}", e);
            ApiError::internal("Internal database error")
        })?;

    tracing::info!("Finalized invoice {}", id);

    let customer_payment_method =
        repo::fetch_customer_default_payment_method(&mut *tx, invoice.ar_customer_id)
            .await
            .map_err(|e| {
                tracing::error!("Failed to fetch customer payment method: {:?}", e);
                ApiError::internal("Internal database error")
            })?;

    // Emit payment.collection.requested event
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
        "DATA_MUTATION".to_string(),
        event_payload,
    )
    .with_tracing_context(&tracing_ctx);
    crate::events::outbox::enqueue_event_tx(
        &mut tx,
        "payment.collection.requested",
        "invoice",
        &invoice.id.to_string(),
        &event_envelope,
    )
    .await
    .map_err(|e| {
        tracing::error!(
            "Failed to enqueue payment.collection.requested event: {:?}",
            e
        );
        ApiError::internal("Internal database error")
    })?;

    // Emit gl.posting.requested event
    let gl_payload = GlPostingRequestPayload {
        posting_date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        currency: invoice.currency.to_uppercase(),
        source_doc_type: "AR_INVOICE".to_string(),
        source_doc_id: invoice.id.to_string(),
        description: format!(
            "Invoice {} for customer {}",
            invoice.id, invoice.ar_customer_id
        ),
        lines: vec![
            GlPostingLine {
                account_ref: "1100".to_string(),
                debit: invoice.amount_cents,
                credit: 0,
                memo: Some(format!("AR Invoice {}", invoice.id)),
            },
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
        "DATA_MUTATION".to_string(),
        gl_payload,
    )
    .with_tracing_context(&tracing_ctx);
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
        ApiError::internal("Internal database error")
    })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("Failed to commit transaction: {:?}", e);
        ApiError::internal("Internal database error")
    })?;

    Ok(invoice)
}
