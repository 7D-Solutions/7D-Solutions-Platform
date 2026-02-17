//! Payment allocation engine (Phase 22, bd-14f)
//!
//! Allocates a payment to one or more open invoices using FIFO (oldest-due-first).
//! All allocations + outbox event are persisted atomically in a single transaction.
//!
//! Idempotency: the caller supplies an `idempotency_key`. If the key already exists
//! in `ar_payment_allocations`, the original allocation result is returned without
//! re-executing.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::events::contracts::{
    build_payment_allocated_envelope, AllocationLine, PaymentAllocatedPayload,
    EVENT_TYPE_PAYMENT_ALLOCATED,
};
use crate::events::outbox::enqueue_event_tx;

// ============================================================================
// Request / Response types
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct AllocatePaymentRequest {
    pub payment_id: String,
    pub customer_id: i32,
    pub amount_cents: i32,
    pub currency: String,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct AllocationResult {
    pub payment_id: String,
    pub allocated_amount_cents: i32,
    pub unallocated_amount_cents: i32,
    pub strategy: String,
    pub allocations: Vec<AllocationRow>,
}

#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct AllocationRow {
    pub id: i32,
    pub invoice_id: i32,
    pub amount_cents: i32,
    pub strategy: String,
}

// ============================================================================
// Core FIFO allocation
// ============================================================================

/// Allocate a payment to open invoices using FIFO (oldest due_at first).
///
/// Guarantees:
/// - Atomic: allocation rows + outbox event in one transaction
/// - Idempotent: duplicate idempotency_key returns original result
/// - Deterministic: FIFO ordering by (due_at ASC NULLS LAST, id ASC)
/// - Partial: allocates as much as possible, returns unallocated remainder
pub async fn allocate_payment_fifo(
    db: &PgPool,
    app_id: &str,
    req: &AllocatePaymentRequest,
) -> Result<AllocationResult, AllocationError> {
    // Guard: check for duplicate idempotency_key (return cached result).
    // Per-line keys are stored as "{parent_key}:inv-{invoice_id}", so we
    // match the prefix to detect a prior allocation with the same parent key.
    let idem_prefix = format!("{}:inv-", req.idempotency_key);
    let existing = sqlx::query_as::<_, AllocationRow>(
        r#"
        SELECT id, invoice_id, amount_cents, strategy
        FROM ar_payment_allocations
        WHERE idempotency_key LIKE $1 || '%'
        "#,
    )
    .bind(&idem_prefix)
    .fetch_all(db)
    .await
    .map_err(AllocationError::Database)?;

    if !existing.is_empty() {
        let allocated: i32 = existing.iter().map(|r| r.amount_cents).sum();
        return Ok(AllocationResult {
            payment_id: req.payment_id.clone(),
            allocated_amount_cents: allocated,
            unallocated_amount_cents: req.amount_cents - allocated,
            strategy: "fifo".to_string(),
            allocations: existing,
        });
    }

    // Guard: validate inputs
    if req.amount_cents <= 0 {
        return Err(AllocationError::Validation(
            "amount_cents must be positive".to_string(),
        ));
    }

    // Begin atomic transaction
    let mut tx = db.begin().await.map_err(AllocationError::Database)?;

    // Fetch open invoices for this customer ordered by FIFO (oldest due first).
    // Lock rows to prevent concurrent allocation races.
    let invoices = sqlx::query(
        r#"
        SELECT i.id, i.amount_cents,
               COALESCE(
                   (SELECT SUM(a.amount_cents) FROM ar_payment_allocations a WHERE a.invoice_id = i.id),
                   0
               )::INTEGER AS already_allocated
        FROM ar_invoices i
        WHERE i.app_id = $1
          AND i.ar_customer_id = $2
          AND i.status IN ('open', 'past_due')
        ORDER BY i.due_at ASC NULLS LAST, i.id ASC
        FOR UPDATE OF i SKIP LOCKED
        "#,
    )
    .bind(app_id)
    .bind(req.customer_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(AllocationError::Database)?;

    let mut remaining_payment = req.amount_cents;
    let mut allocation_rows: Vec<AllocationRow> = Vec::new();
    let mut event_lines: Vec<AllocationLine> = Vec::new();

    for row in &invoices {
        if remaining_payment <= 0 {
            break;
        }

        let invoice_id: i32 = row.get("id");
        let invoice_amount: i32 = row.get("amount_cents");
        let already_allocated: i32 = row.get("already_allocated");
        let invoice_remaining = invoice_amount - already_allocated;

        if invoice_remaining <= 0 {
            continue;
        }

        let alloc_amount = remaining_payment.min(invoice_remaining);
        let remaining_after = invoice_remaining - alloc_amount;

        // Per-line idempotency key: deterministic from parent key + invoice_id
        let line_idem_key = format!("{}:inv-{}", req.idempotency_key, invoice_id);

        let inserted = sqlx::query_as::<_, AllocationRow>(
            r#"
            INSERT INTO ar_payment_allocations (
                app_id, payment_id, invoice_id, amount_cents,
                allocated_at, strategy, idempotency_key
            )
            VALUES ($1, $2, $3, $4, NOW(), 'fifo', $5)
            ON CONFLICT (idempotency_key) DO NOTHING
            RETURNING id, invoice_id, amount_cents, strategy
            "#,
        )
        .bind(app_id)
        .bind(&req.payment_id)
        .bind(invoice_id)
        .bind(alloc_amount)
        .bind(&line_idem_key)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AllocationError::Database)?;

        if let Some(alloc_row) = inserted {
            remaining_payment -= alloc_amount;
            allocation_rows.push(alloc_row);
            event_lines.push(AllocationLine {
                invoice_id: invoice_id.to_string(),
                allocated_minor: alloc_amount as i64,
                remaining_after_minor: remaining_after as i64,
            });
        }
    }

    let allocated_total: i32 = allocation_rows.iter().map(|r| r.amount_cents).sum();
    let unallocated = req.amount_cents - allocated_total;

    // Emit ar.payment_allocated outbox event atomically
    if !allocation_rows.is_empty() {
        let event_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("alloc:{}:{}", req.payment_id, req.idempotency_key).as_bytes(),
        );

        let payload = PaymentAllocatedPayload {
            tenant_id: app_id.to_string(),
            payment_id: req.payment_id.clone(),
            customer_id: req.customer_id.to_string(),
            payment_amount_minor: req.amount_cents as i64,
            allocated_amount_minor: allocated_total as i64,
            unallocated_amount_minor: unallocated as i64,
            currency: req.currency.clone(),
            allocation_strategy: "fifo".to_string(),
            allocations: event_lines,
            allocated_at: Utc::now(),
        };

        let envelope = build_payment_allocated_envelope(
            event_id,
            app_id.to_string(),
            req.idempotency_key.clone(), // correlation_id
            None,
            payload,
        );

        enqueue_event_tx(
            &mut tx,
            EVENT_TYPE_PAYMENT_ALLOCATED,
            "payment",
            &req.payment_id,
            &envelope,
        )
        .await
        .map_err(AllocationError::Database)?;
    }

    tx.commit().await.map_err(AllocationError::Database)?;

    Ok(AllocationResult {
        payment_id: req.payment_id.clone(),
        allocated_amount_cents: allocated_total,
        unallocated_amount_cents: unallocated,
        strategy: "fifo".to_string(),
        allocations: allocation_rows,
    })
}

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum AllocationError {
    Database(sqlx::Error),
    Validation(String),
}

impl std::fmt::Display for AllocationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocationError::Database(e) => write!(f, "database error: {}", e),
            AllocationError::Validation(msg) => write!(f, "validation error: {}", msg),
        }
    }
}
