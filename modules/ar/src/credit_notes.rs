//! AR Credit Note Issuance (bd-1gt)
//!
//! Credit notes are formal financial artifacts that compensate for overbilling
//! or adjustments against a finalized invoice. Semantics:
//!
//! - **Append-only**: once issued, a credit note is never updated or deleted.
//!   Further corrections require a new credit note.
//! - **Compensating entry**: credit notes reduce the outstanding balance on an
//!   invoice without voiding or modifying the original invoice record.
//! - **Atomic**: INSERT into ar_credit_notes + INSERT into events_outbox in
//!   a single BEGIN/COMMIT transaction. If either fails, both roll back.
//! - **Idempotent**: caller supplies a deterministic `credit_note_id` (UUID).
//!   Duplicate credit_note_id returns `AlreadyProcessed` (no-op, no error).
//!
//! ## Transaction Pattern
//! ```text
//! BEGIN
//!   INSERT INTO ar_credit_notes (credit_note_id = $caller_id) ON CONFLICT DO NOTHING
//!   INSERT INTO events_outbox (event_type = 'ar.credit_note_issued')
//! COMMIT
//! ```
//!
//! ## Usage
//! ```rust,ignore
//! let result = issue_credit_note(&pool, IssueCreditNoteRequest {
//!     credit_note_id: Uuid::new_v4(),  // deterministic from business key
//!     app_id: "tenant-1".to_string(),
//!     customer_id: "cust-42".to_string(),
//!     invoice_id: 7,
//!     amount_minor: 5000,
//!     currency: "usd".to_string(),
//!     reason: "billing_error".to_string(),
//!     reference_id: None,
//!     issued_by: Some("admin@example.com".to_string()),
//!     correlation_id: "corr-xyz".to_string(),
//!     causation_id: Some("inv-7-finalize".to_string()),
//! }).await?;
//! ```

use crate::events::{
    build_credit_memo_approved_envelope, build_credit_memo_created_envelope,
    build_credit_note_issued_envelope, CreditNoteIssuedPayload, EVENT_TYPE_CREDIT_NOTE_ISSUED,
    CreditMemoApprovedPayload, CreditMemoCreatedPayload, EVENT_TYPE_CREDIT_MEMO_APPROVED,
    EVENT_TYPE_CREDIT_MEMO_CREATED,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

/// Request to issue a credit note against an invoice
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueCreditNoteRequest {
    /// Stable business key — deterministic from caller's business input.
    /// Duplicate IDs return `AlreadyProcessed` (idempotency anchor).
    pub credit_note_id: Uuid,
    /// Tenant identifier (app_id in AR schema)
    pub app_id: String,
    /// Customer the credit applies to
    pub customer_id: String,
    /// Internal invoice ID (ar_invoices.id)
    pub invoice_id: i32,
    /// Credit amount in minor currency units (must be > 0)
    pub amount_minor: i64,
    /// ISO 4217 currency code (lowercase, e.g. "usd")
    pub currency: String,
    /// Human-readable reason (e.g. "billing_error", "service_credit")
    pub reason: String,
    /// Optional reference to a line item, usage record, or external ID
    pub reference_id: Option<String>,
    /// Who authorized this credit (service name, user email, etc.)
    pub issued_by: Option<String>,
    /// Distributed trace correlation ID (propagated from upstream caller)
    pub correlation_id: String,
    /// Causation ID — what event/action triggered this credit note
    pub causation_id: Option<String>,
}

/// Request to create a draft credit memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCreditMemoRequest {
    pub credit_note_id: Uuid,
    pub app_id: String,
    pub customer_id: String,
    pub invoice_id: i32,
    pub amount_minor: i64,
    pub currency: String,
    pub reason: String,
    pub reference_id: Option<String>,
    pub created_by: Option<String>,
    pub create_idempotency_key: Uuid,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

/// Request to approve a draft credit memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApproveCreditMemoRequest {
    pub app_id: String,
    pub credit_note_id: Uuid,
    pub approved_by: Option<String>,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

/// Request to issue an approved credit memo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueCreditMemoRequest {
    pub app_id: String,
    pub credit_note_id: Uuid,
    pub issued_by: Option<String>,
    pub issue_idempotency_key: Uuid,
    pub correlation_id: String,
    pub causation_id: Option<String>,
}

/// Result of issuing a credit note
#[derive(Debug, Clone)]
pub enum IssueCreditNoteResult {
    /// New credit note created and outbox event enqueued
    Issued {
        /// Internal row ID assigned by the database
        credit_note_row_id: i32,
        /// The stable UUID that was stored (echoes request.credit_note_id)
        credit_note_id: Uuid,
        /// Timestamp of issuance
        issued_at: DateTime<Utc>,
    },
    /// Credit note with this ID was already issued (deterministic no-op)
    AlreadyProcessed {
        /// The existing credit note's row ID
        existing_row_id: i32,
        /// The credit_note_id that already existed
        credit_note_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub enum CreateCreditMemoResult {
    Created {
        credit_note_row_id: i32,
        credit_note_id: Uuid,
        created_at: DateTime<Utc>,
    },
    AlreadyProcessed {
        existing_row_id: i32,
        credit_note_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub enum ApproveCreditMemoResult {
    Approved {
        credit_note_row_id: i32,
        credit_note_id: Uuid,
        approved_at: DateTime<Utc>,
    },
    AlreadyApproved {
        credit_note_row_id: i32,
        credit_note_id: Uuid,
    },
}

#[derive(Debug, Clone)]
pub enum IssueCreditMemoResult {
    Issued {
        credit_note_row_id: i32,
        credit_note_id: Uuid,
        issued_at: DateTime<Utc>,
    },
    AlreadyProcessed {
        existing_row_id: i32,
        credit_note_id: Uuid,
    },
}

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum CreditNoteError {
    /// Invoice not found or does not belong to this tenant
    InvoiceNotFound { invoice_id: i32, app_id: String },
    /// Credit amount must be positive
    InvalidAmount(i64),
    /// Currency must be non-empty
    InvalidCurrency,
    /// Credit memo lifecycle guard failed
    InvalidStatusTransition {
        credit_note_id: Uuid,
        expected: &'static str,
        actual: String,
    },
    /// Credit memo not found in tenant
    CreditMemoNotFound { credit_note_id: Uuid, app_id: String },
    /// Total credits (existing + new) would exceed the invoice amount — financial integrity guard
    OverCreditBalance {
        invoice_id: i32,
        invoice_amount_cents: i64,
        existing_credits: i64,
        requested: i64,
    },
    /// Database error
    DatabaseError(String),
}

impl fmt::Display for CreditNoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvoiceNotFound { invoice_id, app_id } => {
                write!(f, "Invoice {} not found for tenant {}", invoice_id, app_id)
            }
            Self::InvalidAmount(n) => write!(f, "Amount must be > 0, got {}", n),
            Self::InvalidCurrency => write!(f, "Currency must not be empty"),
            Self::InvalidStatusTransition {
                credit_note_id,
                expected,
                actual,
            } => write!(
                f,
                "Credit memo {} expected status '{}' but was '{}'",
                credit_note_id, expected, actual
            ),
            Self::CreditMemoNotFound {
                credit_note_id,
                app_id,
            } => {
                write!(
                    f,
                    "Credit memo {} not found for tenant {}",
                    credit_note_id, app_id
                )
            }
            Self::OverCreditBalance {
                invoice_id,
                invoice_amount_cents,
                existing_credits,
                requested,
            } => {
                write!(
                    f,
                    "Credit of {} exceeds remaining balance ({} - {} = {}) on invoice {}",
                    requested,
                    invoice_amount_cents,
                    existing_credits,
                    invoice_amount_cents - existing_credits,
                    invoice_id
                )
            }
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for CreditNoteError {}

impl From<sqlx::Error> for CreditNoteError {
    fn from(e: sqlx::Error) -> Self {
        tracing::error!("Credit note sqlx error: {:?}", e);
        Self::DatabaseError("internal database error".to_string())
    }
}

// ============================================================================
// Core function
// ============================================================================

pub async fn create_credit_memo(
    pool: &PgPool,
    req: CreateCreditMemoRequest,
) -> Result<CreateCreditMemoResult, CreditNoteError> {
    if req.amount_minor <= 0 {
        return Err(CreditNoteError::InvalidAmount(req.amount_minor));
    }
    if req.currency.trim().is_empty() {
        return Err(CreditNoteError::InvalidCurrency);
    }

    let mut tx = pool.begin().await?;

    let existing_for_key: Option<(i32, Uuid)> = sqlx::query_as(
        "SELECT id, credit_note_id FROM ar_credit_notes WHERE app_id = $1 AND create_idempotency_key = $2",
    )
    .bind(&req.app_id)
    .bind(req.create_idempotency_key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((row_id, credit_note_id)) = existing_for_key {
        tx.rollback().await?;
        return Ok(CreateCreditMemoResult::AlreadyProcessed {
            existing_row_id: row_id,
            credit_note_id,
        });
    }

    let invoice_amount_cents: Option<i64> = sqlx::query_scalar(
        "SELECT amount_cents::BIGINT FROM ar_invoices WHERE id = $1 AND app_id = $2",
    )
    .bind(req.invoice_id)
    .bind(&req.app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let invoice_amount_cents = match invoice_amount_cents {
        Some(v) => v,
        None => {
            tx.rollback().await?;
            return Err(CreditNoteError::InvoiceNotFound {
                invoice_id: req.invoice_id,
                app_id: req.app_id,
            });
        }
    };

    let existing_issued_credits: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT
         FROM ar_credit_notes
         WHERE invoice_id = $1 AND app_id = $2 AND status = 'issued'",
    )
    .bind(req.invoice_id)
    .bind(&req.app_id)
    .fetch_one(&mut *tx)
    .await?;
    if existing_issued_credits + req.amount_minor > invoice_amount_cents {
        tx.rollback().await?;
        return Err(CreditNoteError::OverCreditBalance {
            invoice_id: req.invoice_id,
            invoice_amount_cents,
            existing_credits: existing_issued_credits,
            requested: req.amount_minor,
        });
    }

    let now = Utc::now();
    let credit_note_row_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_credit_notes (
            credit_note_id, app_id, customer_id, invoice_id, amount_minor, currency, reason,
            reference_id, status, issued_by, create_idempotency_key, created_at, updated_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'draft', $9, $10, $11, $11)
        RETURNING id
        "#,
    )
    .bind(req.credit_note_id)
    .bind(&req.app_id)
    .bind(&req.customer_id)
    .bind(req.invoice_id)
    .bind(req.amount_minor)
    .bind(&req.currency)
    .bind(&req.reason)
    .bind(&req.reference_id)
    .bind(&req.created_by)
    .bind(req.create_idempotency_key)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;

    let outbox_event_id = Uuid::new_v4();
    let envelope = build_credit_memo_created_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        CreditMemoCreatedPayload {
            credit_note_id: req.credit_note_id,
            tenant_id: req.app_id.clone(),
            customer_id: req.customer_id.clone(),
            invoice_id: req.invoice_id.to_string(),
            amount_minor: req.amount_minor,
            currency: req.currency.clone(),
            reason: req.reason.clone(),
            reference_id: req.reference_id.clone(),
            created_at: now,
        },
    );
    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| CreditNoteError::DatabaseError(e.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'credit_memo', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_CREDIT_MEMO_CREATED)
    .bind(req.credit_note_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(CreateCreditMemoResult::Created {
        credit_note_row_id,
        credit_note_id: req.credit_note_id,
        created_at: now,
    })
}

pub async fn approve_credit_memo(
    pool: &PgPool,
    req: ApproveCreditMemoRequest,
) -> Result<ApproveCreditMemoResult, CreditNoteError> {
    let mut tx = pool.begin().await?;
    let row: Option<(i32, i32, String)> = sqlx::query_as(
        "SELECT id, invoice_id, status FROM ar_credit_notes WHERE app_id = $1 AND credit_note_id = $2 FOR UPDATE",
    )
    .bind(&req.app_id)
    .bind(req.credit_note_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (credit_note_row_id, invoice_id, status) = match row {
        Some(v) => v,
        None => {
            tx.rollback().await?;
            return Err(CreditNoteError::CreditMemoNotFound {
                credit_note_id: req.credit_note_id,
                app_id: req.app_id,
            });
        }
    };
    if status == "approved" || status == "issued" {
        tx.rollback().await?;
        return Ok(ApproveCreditMemoResult::AlreadyApproved {
            credit_note_row_id,
            credit_note_id: req.credit_note_id,
        });
    }
    if status != "draft" {
        tx.rollback().await?;
        return Err(CreditNoteError::InvalidStatusTransition {
            credit_note_id: req.credit_note_id,
            expected: "draft",
            actual: status,
        });
    }

    let now = Utc::now();
    sqlx::query(
        "UPDATE ar_credit_notes
         SET status = 'approved', approved_by = $1, approved_at = $2, updated_at = $2
         WHERE id = $3",
    )
    .bind(&req.approved_by)
    .bind(now)
    .bind(credit_note_row_id)
    .execute(&mut *tx)
    .await?;

    let outbox_event_id = Uuid::new_v4();
    let envelope = build_credit_memo_approved_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        CreditMemoApprovedPayload {
            credit_note_id: req.credit_note_id,
            tenant_id: req.app_id.clone(),
            invoice_id: invoice_id.to_string(),
            approved_by: req.approved_by.clone(),
            approved_at: now,
        },
    );
    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| CreditNoteError::DatabaseError(e.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'credit_memo', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_CREDIT_MEMO_APPROVED)
    .bind(req.credit_note_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(ApproveCreditMemoResult::Approved {
        credit_note_row_id,
        credit_note_id: req.credit_note_id,
        approved_at: now,
    })
}

pub async fn issue_credit_memo(
    pool: &PgPool,
    req: IssueCreditMemoRequest,
) -> Result<IssueCreditMemoResult, CreditNoteError> {
    let mut tx = pool.begin().await?;

    let existing_for_key: Option<(i32, Uuid)> = sqlx::query_as(
        "SELECT id, credit_note_id FROM ar_credit_notes WHERE app_id = $1 AND issue_idempotency_key = $2",
    )
    .bind(&req.app_id)
    .bind(req.issue_idempotency_key)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((row_id, credit_note_id)) = existing_for_key {
        tx.rollback().await?;
        return Ok(IssueCreditMemoResult::AlreadyProcessed {
            existing_row_id: row_id,
            credit_note_id,
        });
    }

    let row: Option<(i32, String, String, i32, i64, String, String, Option<String>)> =
        sqlx::query_as(
            "SELECT id, status, customer_id, invoice_id, amount_minor, currency, reason, reference_id
             FROM ar_credit_notes WHERE app_id = $1 AND credit_note_id = $2 FOR UPDATE",
        )
        .bind(&req.app_id)
        .bind(req.credit_note_id)
        .fetch_optional(&mut *tx)
        .await?;
    let (credit_note_row_id, status, customer_id, invoice_id, amount_minor, currency, reason, reference_id) =
        match row {
            Some(v) => v,
            None => {
                tx.rollback().await?;
                return Err(CreditNoteError::CreditMemoNotFound {
                    credit_note_id: req.credit_note_id,
                    app_id: req.app_id,
                });
            }
        };

    if status == "issued" {
        tx.rollback().await?;
        return Ok(IssueCreditMemoResult::AlreadyProcessed {
            existing_row_id: credit_note_row_id,
            credit_note_id: req.credit_note_id,
        });
    }
    if status != "approved" {
        tx.rollback().await?;
        return Err(CreditNoteError::InvalidStatusTransition {
            credit_note_id: req.credit_note_id,
            expected: "approved",
            actual: status,
        });
    }

    let invoice_amount_cents: Option<i64> = sqlx::query_scalar(
        "SELECT amount_cents::BIGINT FROM ar_invoices WHERE id = $1 AND app_id = $2",
    )
    .bind(invoice_id)
    .bind(&req.app_id)
    .fetch_optional(&mut *tx)
    .await?;
    let invoice_amount_cents = match invoice_amount_cents {
        Some(v) => v,
        None => {
            tx.rollback().await?;
            return Err(CreditNoteError::InvoiceNotFound {
                invoice_id,
                app_id: req.app_id,
            });
        }
    };
    let existing_issued_credits: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_minor), 0)::BIGINT
         FROM ar_credit_notes
         WHERE invoice_id = $1 AND app_id = $2 AND status = 'issued'",
    )
    .bind(invoice_id)
    .bind(&req.app_id)
    .fetch_one(&mut *tx)
    .await?;
    if existing_issued_credits + amount_minor > invoice_amount_cents {
        tx.rollback().await?;
        return Err(CreditNoteError::OverCreditBalance {
            invoice_id,
            invoice_amount_cents,
            existing_credits: existing_issued_credits,
            requested: amount_minor,
        });
    }

    let now = Utc::now();
    sqlx::query(
        "UPDATE ar_credit_notes
         SET status = 'issued', issued_at = $1, issued_by = $2, issue_idempotency_key = $3, updated_at = $1
         WHERE id = $4",
    )
    .bind(now)
    .bind(&req.issued_by)
    .bind(req.issue_idempotency_key)
    .bind(credit_note_row_id)
    .execute(&mut *tx)
    .await?;

    let outbox_event_id = Uuid::new_v4();
    let envelope = build_credit_note_issued_envelope(
        outbox_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        CreditNoteIssuedPayload {
            credit_note_id: req.credit_note_id,
            tenant_id: req.app_id.clone(),
            customer_id,
            invoice_id: invoice_id.to_string(),
            amount_minor,
            currency: currency.clone(),
            reason: reason.clone(),
            reference_id: reference_id.clone(),
            issued_at: now,
        },
    );
    let payload_json = serde_json::to_value(&envelope)
        .map_err(|e| CreditNoteError::DatabaseError(e.to_string()))?;
    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'credit_note', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
        "#,
    )
    .bind(outbox_event_id)
    .bind(EVENT_TYPE_CREDIT_NOTE_ISSUED)
    .bind(req.credit_note_id.to_string())
    .bind(payload_json)
    .bind(&req.app_id)
    .bind(&envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE ar_credit_notes SET outbox_event_id = $1 WHERE id = $2")
        .bind(outbox_event_id)
        .bind(credit_note_row_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(IssueCreditMemoResult::Issued {
        credit_note_row_id,
        credit_note_id: req.credit_note_id,
        issued_at: now,
    })
}

/// Backward-compatible endpoint behavior:
/// create draft -> approve -> issue in one call.
pub async fn issue_credit_note(
    pool: &PgPool,
    req: IssueCreditNoteRequest,
) -> Result<IssueCreditNoteResult, CreditNoteError> {
    let create_key = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("credit-memo:create:{}:{}", req.app_id, req.credit_note_id).as_bytes(),
    );
    let issue_key = Uuid::new_v5(
        &Uuid::NAMESPACE_OID,
        format!("credit-memo:issue:{}:{}", req.app_id, req.credit_note_id).as_bytes(),
    );

    let create = create_credit_memo(
        pool,
        CreateCreditMemoRequest {
            credit_note_id: req.credit_note_id,
            app_id: req.app_id.clone(),
            customer_id: req.customer_id.clone(),
            invoice_id: req.invoice_id,
            amount_minor: req.amount_minor,
            currency: req.currency.clone(),
            reason: req.reason.clone(),
            reference_id: req.reference_id.clone(),
            created_by: req.issued_by.clone(),
            create_idempotency_key: create_key,
            correlation_id: req.correlation_id.clone(),
            causation_id: req.causation_id.clone(),
        },
    )
    .await?;

    match create {
        CreateCreditMemoResult::Created { .. } | CreateCreditMemoResult::AlreadyProcessed { .. } => {
            let _ = approve_credit_memo(
                pool,
                ApproveCreditMemoRequest {
                    app_id: req.app_id.clone(),
                    credit_note_id: req.credit_note_id,
                    approved_by: req.issued_by.clone(),
                    correlation_id: req.correlation_id.clone(),
                    causation_id: req.causation_id.clone(),
                },
            )
            .await?;
        }
    }

    match issue_credit_memo(
        pool,
        IssueCreditMemoRequest {
            app_id: req.app_id.clone(),
            credit_note_id: req.credit_note_id,
            issued_by: req.issued_by.clone(),
            issue_idempotency_key: issue_key,
            correlation_id: req.correlation_id.clone(),
            causation_id: req.causation_id.clone(),
        },
    )
    .await?
    {
        IssueCreditMemoResult::Issued {
            credit_note_row_id,
            credit_note_id,
            issued_at,
        } => Ok(IssueCreditNoteResult::Issued {
            credit_note_row_id,
            credit_note_id,
            issued_at,
        }),
        IssueCreditMemoResult::AlreadyProcessed {
            existing_row_id,
            credit_note_id,
        } => Ok(IssueCreditNoteResult::AlreadyProcessed {
            existing_row_id,
            credit_note_id,
        }),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_note_error_display() {
        let err = CreditNoteError::InvoiceNotFound {
            invoice_id: 42,
            app_id: "tenant-1".to_string(),
        };
        assert_eq!(err.to_string(), "Invoice 42 not found for tenant tenant-1");

        let err = CreditNoteError::InvalidAmount(-100);
        assert_eq!(err.to_string(), "Amount must be > 0, got -100");

        let err = CreditNoteError::InvalidAmount(0);
        assert_eq!(err.to_string(), "Amount must be > 0, got 0");

        let err = CreditNoteError::InvalidCurrency;
        assert_eq!(err.to_string(), "Currency must not be empty");

        let err = CreditNoteError::DatabaseError("internal database error".to_string());
        assert_eq!(err.to_string(), "Database error: internal database error");
    }

    #[test]
    fn issue_credit_note_result_variants() {
        let issued = IssueCreditNoteResult::Issued {
            credit_note_row_id: 1,
            credit_note_id: Uuid::new_v4(),
            issued_at: Utc::now(),
        };
        assert!(matches!(issued, IssueCreditNoteResult::Issued { .. }));

        let dup = IssueCreditNoteResult::AlreadyProcessed {
            existing_row_id: 1,
            credit_note_id: Uuid::new_v4(),
        };
        assert!(matches!(
            dup,
            IssueCreditNoteResult::AlreadyProcessed { .. }
        ));
    }
}
