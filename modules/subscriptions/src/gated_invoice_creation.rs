//! Gated Invoice Creation
//!
//! Phase 15 bd-184: Wraps AR invoice creation with cycle gating to ensure
//! exactly-once invoice per subscription cycle.
//!
//! # Flow
//! ```
//! Gate → Lock → Check → Execute → Record
//! ```

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::cycle_gating::{
    acquire_cycle_lock, calculate_cycle_boundaries, cycle_attempt_exists, generate_cycle_key,
    mark_attempt_failed, mark_attempt_succeeded, record_cycle_attempt, CycleGatingError,
};
use crate::models::{CreateInvoiceRequest, FinalizeInvoiceRequest, Invoice};

#[derive(Debug)]
pub enum InvoiceCreationError {
    /// Duplicate invoice for this cycle (idempotent - not an error)
    DuplicateCycle {
        subscription_id: Uuid,
        cycle_key: String,
    },
    /// AR API error (invoice creation failed)
    ArApiError { status: u16, message: String },
    /// AR API communication error
    ArApiCommunicationError { message: String },
    /// Database error
    DatabaseError { message: String },
    /// Cycle gating error
    GatingError { message: String },
}

impl std::fmt::Display for InvoiceCreationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateCycle {
                subscription_id,
                cycle_key,
            } => write!(
                f,
                "Duplicate invoice for subscription {} cycle {}",
                subscription_id, cycle_key
            ),
            Self::ArApiError { status, message } => {
                write!(f, "AR API error ({}): {}", status, message)
            }
            Self::ArApiCommunicationError { message } => {
                write!(f, "AR API communication error: {}", message)
            }
            Self::DatabaseError { message } => write!(f, "Database error: {}", message),
            Self::GatingError { message } => write!(f, "Gating error: {}", message),
        }
    }
}

impl std::error::Error for InvoiceCreationError {}

impl From<sqlx::Error> for InvoiceCreationError {
    fn from(e: sqlx::Error) -> Self {
        InvoiceCreationError::DatabaseError {
            message: e.to_string(),
        }
    }
}

impl From<CycleGatingError> for InvoiceCreationError {
    fn from(e: CycleGatingError) -> Self {
        InvoiceCreationError::GatingError {
            message: e.to_string(),
        }
    }
}

pub struct InvoiceCreationResult {
    pub invoice_id: i32,
    pub ar_invoice_id: i32,
    pub cycle_key: String,
}

/// Create invoice for subscription with cycle gating.
///
/// **Pattern:** Gate → Lock → Check → Execute → Record
///
/// **Idempotency:** Returns `DuplicateCycle` error if invoice already created for this cycle
///
/// **Concurrency:** Advisory lock prevents concurrent attempts for same cycle
///
/// **Exactly-Once:** UNIQUE constraint on (tenant_id, subscription_id, cycle_key)
pub async fn create_gated_invoice(
    pool: &PgPool,
    tenant_id: &str,
    subscription_id: Uuid,
    ar_customer_id: i32,
    price_minor: i64,
    billing_date: NaiveDate,
    ar_base_url: &str,
) -> Result<InvoiceCreationResult, InvoiceCreationError> {
    // Generate cycle key and boundaries
    let cycle_key = generate_cycle_key(billing_date);
    let (cycle_start, cycle_end) = calculate_cycle_boundaries(billing_date);

    tracing::info!(
        tenant_id = tenant_id,
        subscription_id = %subscription_id,
        cycle_key = &cycle_key,
        "Starting gated invoice creation"
    );

    // Begin transaction
    let mut tx = pool.begin().await?;

    // Step 1: Acquire advisory lock (transaction-scoped)
    acquire_cycle_lock(&mut tx, tenant_id, subscription_id, &cycle_key).await?;

    // Step 2: Check if attempt already exists (idempotency)
    if cycle_attempt_exists(&mut tx, tenant_id, subscription_id, &cycle_key).await? {
        tracing::info!(
            tenant_id = tenant_id,
            subscription_id = %subscription_id,
            cycle_key = &cycle_key,
            "Invoice already created for this cycle (idempotent)"
        );
        tx.rollback().await?;
        return Err(InvoiceCreationError::DuplicateCycle {
            subscription_id,
            cycle_key: cycle_key.clone(),
        });
    }

    // Step 3: Record attempt (status: 'attempting')
    let attempt_id = record_cycle_attempt(
        &mut tx,
        tenant_id,
        subscription_id,
        &cycle_key,
        cycle_start,
        cycle_end,
        None,
    )
    .await?;

    tracing::debug!(
        attempt_id = %attempt_id,
        "Recorded cycle attempt"
    );

    // Commit transaction (releases advisory lock)
    tx.commit().await?;

    // Step 4: Execute invoice creation via AR API (outside transaction)
    //
    // Timeout Budget (per DOMAIN-OWNERSHIP-REGISTRY.md):
    // - 15s per HTTP call (create + finalize)
    // - 30s total for invoice creation operation
    // - NO automatic retry (cycle gating enforces exactly-once)
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| InvoiceCreationError::ArApiCommunicationError {
            message: format!("Failed to build HTTP client: {}", e),
        })?;

    // Create invoice
    let create_invoice_req = CreateInvoiceRequest {
        ar_customer_id,
        amount_cents: price_minor as i32,
    };

    let create_result = client
        .post(format!("{}/api/ar/invoices", ar_base_url))
        .json(&create_invoice_req)
        .send()
        .await
        .map_err(|e| InvoiceCreationError::ArApiCommunicationError {
            message: e.to_string(),
        })?;

    if !create_result.status().is_success() {
        let status = create_result.status().as_u16();
        let message = format!("Failed to create invoice: status {}", status);

        // Mark attempt as failed
        let mut tx = pool.begin().await?;
        mark_attempt_failed(&mut tx, attempt_id, "AR_CREATE_FAILED", &message).await?;
        tx.commit().await?;

        return Err(InvoiceCreationError::ArApiError { status, message });
    }

    let invoice: Invoice =
        create_result
            .json()
            .await
            .map_err(|e| InvoiceCreationError::ArApiCommunicationError {
                message: format!("Failed to parse invoice response: {}", e),
            })?;

    tracing::info!(
        invoice_id = invoice.id,
        subscription_id = %subscription_id,
        "Created AR invoice"
    );

    // Finalize invoice
    let finalize_req = FinalizeInvoiceRequest {
        auto_advance: Some(true),
    };

    let finalize_result = client
        .post(format!(
            "{}/api/ar/invoices/{}/finalize",
            ar_base_url, invoice.id
        ))
        .json(&finalize_req)
        .send()
        .await
        .map_err(|e| InvoiceCreationError::ArApiCommunicationError {
            message: e.to_string(),
        })?;

    if !finalize_result.status().is_success() {
        let status = finalize_result.status().as_u16();
        let message = format!(
            "Failed to finalize invoice {}: status {}",
            invoice.id, status
        );

        // Mark attempt as failed
        let mut tx = pool.begin().await?;
        mark_attempt_failed(&mut tx, attempt_id, "AR_FINALIZE_FAILED", &message).await?;
        tx.commit().await?;

        return Err(InvoiceCreationError::ArApiError { status, message });
    }

    tracing::info!(
        invoice_id = invoice.id,
        subscription_id = %subscription_id,
        "Finalized AR invoice"
    );

    // Step 5: Mark attempt as succeeded
    let mut tx = pool.begin().await?;
    mark_attempt_succeeded(&mut tx, attempt_id, invoice.id).await?;
    tx.commit().await?;

    tracing::info!(
        attempt_id = %attempt_id,
        invoice_id = invoice.id,
        cycle_key = &cycle_key,
        "Gated invoice creation succeeded"
    );

    Ok(InvoiceCreationResult {
        invoice_id: invoice.id,
        ar_invoice_id: invoice.id,
        cycle_key,
    })
}
