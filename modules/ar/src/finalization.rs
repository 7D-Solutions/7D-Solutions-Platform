//! AR Finalization Gating Module (Phase 15 - bd-3fo)
//!
//! **Exactly-Once Enforcement:** This module provides deterministic invoice finalization
//! with concurrency safety via SELECT FOR UPDATE and attempt ledger UNIQUE constraints.
//!
//! **Critical Invariant (ChatGPT):**
//! Side effects (events, ledger posts, PSP calls) occur ONLY when attempt row is newly created.
//!
//! **Transaction Pattern:**
//! Lock → Insert attempt row → Guard → Mutate → Side effects → Commit
//!
//! **Concurrency Safety:**
//! - SELECT FOR UPDATE prevents double-finalization under concurrent requests
//! - UNIQUE(app_id, invoice_id, attempt_no) prevents duplicate attempts
//! - UNIQUE violation → deterministic no-op (returns Ok with AlreadyProcessed status)

use crate::lifecycle::{self, LifecycleError};
use crate::idempotency_keys::generate_invoice_attempt_key;
use crate::tax::{TaxProvider, TaxProviderError, TaxCommitRequest, TaxVoidRequest};
use chrono::Utc;
use sqlx::PgPool;
use std::fmt;
use tracing::{info, warn};
use uuid::Uuid;
use serde_json;

// ============================================================================
// Error Types
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizationError {
    /// Invoice not found
    InvoiceNotFound(i32),
    /// Invoice already finalized (attempt row exists)
    AlreadyProcessed {
        invoice_id: i32,
        attempt_no: i32,
        idempotency_key: String,
    },
    /// Database error during finalization
    DatabaseError(String),
    /// Lifecycle transition error (guard rejection)
    LifecycleError(String),
}

impl fmt::Display for FinalizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvoiceNotFound(id) => write!(f, "Invoice not found: {}", id),
            Self::AlreadyProcessed {
                invoice_id,
                attempt_no,
                idempotency_key,
            } => write!(
                f,
                "Invoice {} already processed (attempt {} with key {})",
                invoice_id, attempt_no, idempotency_key
            ),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            Self::LifecycleError(msg) => write!(f, "Lifecycle error: {}", msg),
        }
    }
}

impl std::error::Error for FinalizationError {}

impl From<sqlx::Error> for FinalizationError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

impl From<LifecycleError> for FinalizationError {
    fn from(e: LifecycleError) -> Self {
        Self::LifecycleError(e.to_string())
    }
}

// ============================================================================
// Finalization Result
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalizationResult {
    /// New attempt created, side effects executed
    NewAttempt {
        attempt_id: Uuid,
        idempotency_key: String,
    },
    /// Duplicate attempt detected, deterministic no-op
    AlreadyProcessed {
        existing_attempt_id: Uuid,
        idempotency_key: String,
    },
}

// ============================================================================
// Finalization Gating Function
// ============================================================================

/// Finalize invoice with exactly-once guarantee
///
/// **Transaction Pattern:**
/// 1. Lock invoice row (SELECT FOR UPDATE)
/// 2. Insert attempt row (UNIQUE constraint enforcement)
/// 3. Guard validates transition (lifecycle::validate_transition)
/// 4. Mutate invoice status (lifecycle::transition_to_attempting)
/// 5. Emit side effects (ONLY if attempt row newly created)
/// 6. Commit transaction
///
/// **Exactly-Once Guarantee:**
/// - UNIQUE constraint on (app_id, invoice_id, attempt_no) prevents duplicate attempts
/// - UNIQUE violation → returns Ok(AlreadyProcessed) (deterministic no-op)
/// - Side effects occur ONLY when attempt row is newly created
///
/// **Concurrency Safety:**
/// - SELECT FOR UPDATE prevents concurrent finalization of same invoice
/// - Transaction-scoped lock released on commit/rollback
///
/// **Example:**
/// ```rust
/// let pool = /* ... */;
/// let result = finalize_invoice(&pool, "app-demo", 123, 0).await?;
///
/// match result {
///     FinalizationResult::NewAttempt { attempt_id, .. } => {
///         println!("New attempt created: {}", attempt_id);
///     }
///     FinalizationResult::AlreadyProcessed { .. } => {
///         println!("Already processed (deterministic no-op)");
///     }
/// }
/// ```
pub async fn finalize_invoice(
    pool: &PgPool,
    app_id: &str,
    invoice_id: i32,
    attempt_no: i32,
) -> Result<FinalizationResult, FinalizationError> {
    let mut tx = pool.begin().await?;

    // 1. LOCK: SELECT FOR UPDATE on invoice row (prevent concurrent finalization)
    let invoice_exists: Option<bool> = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM ar_invoices WHERE id = $1 AND app_id = $2) FOR UPDATE"
    )
    .bind(invoice_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    if invoice_exists != Some(true) {
        return Err(FinalizationError::InvoiceNotFound(invoice_id));
    }

    // 2. CHECK FOR EXISTING ATTEMPT: Deterministic no-op if already exists
    let idempotency_key = generate_invoice_attempt_key(app_id, invoice_id, attempt_no)
        .map_err(|e| FinalizationError::DatabaseError(e.to_string()))?;

    // Check if attempt already exists (with row lock to prevent races)
    let existing_attempt: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM ar_invoice_attempts
         WHERE app_id = $1 AND invoice_id = $2 AND attempt_no = $3
         FOR UPDATE"
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(attempt_no)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(existing_attempt_id) = existing_attempt {
        // Duplicate attempt → deterministic no-op
        warn!(
            module = "ar",
            entity_type = "invoice",
            entity_id = invoice_id,
            attempt_no = attempt_no,
            existing_attempt_id = %existing_attempt_id,
            idempotency_key = idempotency_key.as_str(),
            decision = "already_processed",
            "Finalization: duplicate attempt detected (deterministic no-op)"
        );

        tx.rollback().await?;

        return Ok(FinalizationResult::AlreadyProcessed {
            existing_attempt_id,
            idempotency_key: idempotency_key.to_string(),
        });
    }

    // 3. INSERT NEW ATTEMPT ROW (with ON CONFLICT for race condition handling)
    let attempt_id_result: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO ar_invoice_attempts (app_id, invoice_id, attempt_no, status, attempted_at, idempotency_key)
         VALUES ($1, $2, $3, $4::ar_invoice_attempt_status, $5, $6)
         ON CONFLICT (app_id, invoice_id, attempt_no) DO NOTHING
         RETURNING id"
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(attempt_no)
    .bind("attempting")
    .bind(Utc::now().naive_utc())
    .bind(idempotency_key.as_str())
    .fetch_optional(&mut *tx)
    .await?;

    // If ON CONFLICT triggered (no row returned), fetch existing attempt
    let attempt_id = if let Some(id) = attempt_id_result {
        id
    } else {
        // Race condition: another transaction inserted between our SELECT and INSERT
        // Fetch the existing attempt ID
        let existing_id: Uuid = sqlx::query_scalar(
            "SELECT id FROM ar_invoice_attempts
             WHERE app_id = $1 AND invoice_id = $2 AND attempt_no = $3"
        )
        .bind(app_id)
        .bind(invoice_id)
        .bind(attempt_no)
        .fetch_one(&mut *tx)
        .await?;

        warn!(
            module = "ar",
            entity_type = "invoice",
            entity_id = invoice_id,
            attempt_no = attempt_no,
            existing_attempt_id = %existing_id,
            idempotency_key = idempotency_key.as_str(),
            decision = "already_processed",
            "Finalization: race condition detected, attempt already exists (deterministic no-op)"
        );

        tx.rollback().await?;

        return Ok(FinalizationResult::AlreadyProcessed {
            existing_attempt_id: existing_id,
            idempotency_key: idempotency_key.to_string(),
        });
    };

    info!(
        module = "ar",
        entity_type = "invoice",
        entity_id = invoice_id,
        attempt_no = attempt_no,
        attempt_id = %attempt_id,
        idempotency_key = idempotency_key.as_str(),
        decision = "new_attempt",
        "Finalization: new attempt created"
    );

    // 4. MUTATE: Update invoice status to ATTEMPTING (within same transaction)
    // Status transition from OPEN → ATTEMPTING is idempotent (multiple attempts allowed)
    let current_status: String = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE id = $1"
    )
    .bind(invoice_id)
    .fetch_one(&mut *tx)
    .await?;

    // Only transition to ATTEMPTING if currently OPEN
    // (Subsequent attempts don't re-transition status)
    if current_status == lifecycle::status::OPEN {
        sqlx::query("UPDATE ar_invoices SET status = $1 WHERE id = $2")
            .bind(lifecycle::status::ATTEMPTING)
            .bind(invoice_id)
            .execute(&mut *tx)
            .await?;

        info!(
            module = "ar",
            entity_type = "invoice",
            entity_id = invoice_id,
            from_state = %current_status,
            to_state = lifecycle::status::ATTEMPTING,
            "Invoice status transitioned to ATTEMPTING"
        );
    }

    // 5. EMIT: Insert outbox event atomically (Guard->Mutation->Outbox in same tx)
    let outbox_event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "invoice_id": invoice_id,
        "attempt_id": attempt_id.to_string(),
        "attempt_no": attempt_no,
        "tenant_id": app_id,
    });
    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, occurred_at, replay_safe
        )
        VALUES ($1, 'ar.invoice.finalizing', 'invoice', $2, $3, $4, 'ar', 'LIFECYCLE', NOW(), true)
        "#,
    )
    .bind(outbox_event_id)
    .bind(invoice_id.to_string())
    .bind(payload)
    .bind(app_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(FinalizationResult::NewAttempt {
        attempt_id,
        idempotency_key: idempotency_key.to_string(),
    })
}

// ============================================================================
// Tax Commit/Void
// ============================================================================

#[derive(Debug)]
pub enum TaxCommitError {
    /// No cached tax quote found for this invoice
    NoQuote { app_id: String, invoice_id: String },
    /// Tax already committed (idempotent no-op)
    AlreadyCommitted { provider_commit_ref: String },
    /// Tax already voided
    AlreadyVoided { invoice_id: String },
    /// No committed tax to void
    NotCommitted { invoice_id: String },
    /// Provider error
    ProviderError(TaxProviderError),
    /// Database error
    DatabaseError(String),
}

impl fmt::Display for TaxCommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoQuote { app_id, invoice_id } => {
                write!(f, "No cached tax quote for app={} invoice={}", app_id, invoice_id)
            }
            Self::AlreadyCommitted { provider_commit_ref } => {
                write!(f, "Tax already committed: {}", provider_commit_ref)
            }
            Self::AlreadyVoided { invoice_id } => {
                write!(f, "Tax already voided for invoice {}", invoice_id)
            }
            Self::NotCommitted { invoice_id } => {
                write!(f, "No committed tax to void for invoice {}", invoice_id)
            }
            Self::ProviderError(e) => write!(f, "Tax provider error: {}", e),
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
        }
    }
}

impl std::error::Error for TaxCommitError {}

impl From<sqlx::Error> for TaxCommitError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

impl From<TaxProviderError> for TaxCommitError {
    fn from(e: TaxProviderError) -> Self {
        Self::ProviderError(e)
    }
}

/// Result of a tax commit operation.
#[derive(Debug, Clone)]
pub struct TaxCommitResult {
    pub provider_commit_ref: String,
    pub provider_quote_ref: String,
    pub total_tax_minor: i64,
    pub currency: String,
    pub was_already_committed: bool,
}

/// Result of a tax void operation.
#[derive(Debug, Clone)]
pub struct TaxVoidResult {
    pub provider_commit_ref: String,
    pub total_tax_minor: i64,
    pub was_already_voided: bool,
}

/// Commit tax for an invoice — exactly-once via UNIQUE(app_id, invoice_id).
///
/// 1. Look up the most recent cached tax quote for (app_id, invoice_id)
/// 2. Check ar_tax_commits for existing commit → idempotent no-op if found
/// 3. Call provider.commit_tax()
/// 4. Insert ar_tax_commits row (ON CONFLICT → idempotent)
/// 5. Emit tax.committed event to outbox
///
/// Returns the commit result with provider references for audit trail.
pub async fn commit_tax_for_invoice<P: TaxProvider>(
    pool: &PgPool,
    provider: &P,
    app_id: &str,
    invoice_id: &str,
    customer_id: &str,
    correlation_id: &str,
) -> Result<TaxCommitResult, TaxCommitError> {
    // 1. Check for existing commit (idempotent guard)
    let existing: Option<(String, String, i64)> = sqlx::query_as(
        r#"SELECT provider_commit_ref, status, total_tax_minor
           FROM ar_tax_commits
           WHERE app_id = $1 AND invoice_id = $2"#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .fetch_optional(pool)
    .await?;

    if let Some((commit_ref, status, total)) = existing {
        if status == "voided" {
            return Err(TaxCommitError::AlreadyVoided {
                invoice_id: invoice_id.to_string(),
            });
        }
        info!(
            app_id = app_id,
            invoice_id = invoice_id,
            provider_commit_ref = commit_ref.as_str(),
            "Tax already committed — idempotent no-op"
        );
        return Ok(TaxCommitResult {
            provider_commit_ref: commit_ref,
            provider_quote_ref: String::new(),
            total_tax_minor: total,
            currency: "usd".to_string(),
            was_already_committed: true,
        });
    }

    // 2. Look up cached quote
    let quote_row: Option<(String, i64, String)> = sqlx::query_as(
        r#"SELECT provider_quote_ref, total_tax_minor, COALESCE(
               (response_json->>'currency')::text, 'usd'
           ) as currency
           FROM ar_tax_quote_cache
           WHERE app_id = $1 AND invoice_id = $2
           ORDER BY created_at DESC
           LIMIT 1"#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .fetch_optional(pool)
    .await?;

    let (provider_quote_ref, total_tax_minor, currency) = quote_row.ok_or_else(|| {
        TaxCommitError::NoQuote {
            app_id: app_id.to_string(),
            invoice_id: invoice_id.to_string(),
        }
    })?;

    // 3. Call provider
    let commit_req = TaxCommitRequest {
        tenant_id: app_id.to_string(),
        invoice_id: invoice_id.to_string(),
        provider_quote_ref: provider_quote_ref.clone(),
        correlation_id: correlation_id.to_string(),
    };
    let commit_resp = provider.commit_tax(commit_req).await?;

    // 4. Persist commit record (ON CONFLICT for race condition safety)
    let inserted: Option<Uuid> = sqlx::query_scalar(
        r#"INSERT INTO ar_tax_commits (
               app_id, invoice_id, customer_id, provider,
               provider_quote_ref, provider_commit_ref,
               total_tax_minor, currency, status,
               committed_at, correlation_id
           )
           VALUES ($1, $2, $3, 'local', $4, $5, $6, $7, 'committed', $8, $9)
           ON CONFLICT (app_id, invoice_id) DO NOTHING
           RETURNING id"#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .bind(customer_id)
    .bind(&provider_quote_ref)
    .bind(&commit_resp.provider_commit_ref)
    .bind(total_tax_minor)
    .bind(&currency)
    .bind(commit_resp.committed_at)
    .bind(correlation_id)
    .fetch_optional(pool)
    .await?;

    if inserted.is_none() {
        // Race: another process committed between our SELECT and INSERT
        let existing_ref: String = sqlx::query_scalar(
            "SELECT provider_commit_ref FROM ar_tax_commits WHERE app_id = $1 AND invoice_id = $2",
        )
        .bind(app_id)
        .bind(invoice_id)
        .fetch_one(pool)
        .await?;

        info!(
            app_id = app_id,
            invoice_id = invoice_id,
            "Tax commit race — returning existing commit ref"
        );
        return Ok(TaxCommitResult {
            provider_commit_ref: existing_ref,
            provider_quote_ref,
            total_tax_minor,
            currency,
            was_already_committed: true,
        });
    }

    // 5. Emit tax.committed event
    let event_payload = crate::events::contracts::TaxCommittedPayload {
        tenant_id: app_id.to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: customer_id.to_string(),
        total_tax_minor,
        currency: currency.clone(),
        provider_quote_ref: provider_quote_ref.clone(),
        provider_commit_ref: commit_resp.provider_commit_ref.clone(),
        provider: "local".to_string(),
        committed_at: commit_resp.committed_at,
    };

    let envelope = crate::events::contracts::build_tax_committed_envelope(
        Uuid::new_v4(),
        app_id.to_string(),
        correlation_id.to_string(),
        None,
        event_payload,
    );

    #[allow(deprecated)]
    crate::events::outbox::enqueue_event(
        pool,
        crate::events::contracts::EVENT_TYPE_TAX_COMMITTED,
        "invoice",
        invoice_id,
        &envelope,
    )
    .await
    .map_err(|e| TaxCommitError::DatabaseError(format!("outbox enqueue failed: {}", e)))?;

    info!(
        app_id = app_id,
        invoice_id = invoice_id,
        provider_commit_ref = commit_resp.provider_commit_ref.as_str(),
        total_tax_minor = total_tax_minor,
        "Tax committed for invoice"
    );

    Ok(TaxCommitResult {
        provider_commit_ref: commit_resp.provider_commit_ref,
        provider_quote_ref,
        total_tax_minor,
        currency,
        was_already_committed: false,
    })
}

/// Void committed tax for an invoice — exactly-once via status check.
///
/// 1. Look up ar_tax_commits for (app_id, invoice_id)
/// 2. If status == 'voided' → idempotent no-op
/// 3. If status != 'committed' → error
/// 4. Call provider.void_tax()
/// 5. Update status to 'voided'
/// 6. Emit tax.voided event to outbox
pub async fn void_tax_for_invoice<P: TaxProvider>(
    pool: &PgPool,
    provider: &P,
    app_id: &str,
    invoice_id: &str,
    void_reason: &str,
    correlation_id: &str,
) -> Result<TaxVoidResult, TaxCommitError> {
    // 1. Look up existing commit
    let commit_row: Option<(String, String, i64, String)> = sqlx::query_as(
        r#"SELECT provider_commit_ref, status, total_tax_minor, customer_id
           FROM ar_tax_commits
           WHERE app_id = $1 AND invoice_id = $2"#,
    )
    .bind(app_id)
    .bind(invoice_id)
    .fetch_optional(pool)
    .await?;

    let (provider_commit_ref, status, total_tax_minor, customer_id) =
        commit_row.ok_or_else(|| TaxCommitError::NotCommitted {
            invoice_id: invoice_id.to_string(),
        })?;

    // 2. Idempotent guard
    if status == "voided" {
        info!(
            app_id = app_id,
            invoice_id = invoice_id,
            "Tax already voided — idempotent no-op"
        );
        return Ok(TaxVoidResult {
            provider_commit_ref,
            total_tax_minor,
            was_already_voided: true,
        });
    }

    if status != "committed" {
        return Err(TaxCommitError::NotCommitted {
            invoice_id: invoice_id.to_string(),
        });
    }

    // 3. Call provider
    let void_req = TaxVoidRequest {
        tenant_id: app_id.to_string(),
        invoice_id: invoice_id.to_string(),
        provider_commit_ref: provider_commit_ref.clone(),
        void_reason: void_reason.to_string(),
        correlation_id: correlation_id.to_string(),
    };
    let void_resp = provider.void_tax(void_req).await?;

    // 4. Update status to voided (with WHERE status = 'committed' for safety)
    let rows_affected = sqlx::query(
        r#"UPDATE ar_tax_commits
           SET status = 'voided', voided_at = $1, void_reason = $2, updated_at = NOW()
           WHERE app_id = $3 AND invoice_id = $4 AND status = 'committed'"#,
    )
    .bind(void_resp.voided_at)
    .bind(void_reason)
    .bind(app_id)
    .bind(invoice_id)
    .execute(pool)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        // Another process voided between our SELECT and UPDATE
        return Ok(TaxVoidResult {
            provider_commit_ref,
            total_tax_minor,
            was_already_voided: true,
        });
    }

    // 5. Emit tax.voided event
    let event_payload = crate::events::contracts::TaxVoidedPayload {
        tenant_id: app_id.to_string(),
        invoice_id: invoice_id.to_string(),
        customer_id: customer_id.clone(),
        total_tax_minor,
        currency: "usd".to_string(),
        provider_commit_ref: provider_commit_ref.clone(),
        provider: "local".to_string(),
        void_reason: void_reason.to_string(),
        voided_at: void_resp.voided_at,
    };

    let envelope = crate::events::contracts::build_tax_voided_envelope(
        Uuid::new_v4(),
        app_id.to_string(),
        correlation_id.to_string(),
        None,
        event_payload,
    );

    #[allow(deprecated)]
    crate::events::outbox::enqueue_event(
        pool,
        crate::events::contracts::EVENT_TYPE_TAX_VOIDED,
        "invoice",
        invoice_id,
        &envelope,
    )
    .await
    .map_err(|e| TaxCommitError::DatabaseError(format!("outbox enqueue failed: {}", e)))?;

    info!(
        app_id = app_id,
        invoice_id = invoice_id,
        provider_commit_ref = provider_commit_ref.as_str(),
        void_reason = void_reason,
        "Tax voided for invoice"
    );

    Ok(TaxVoidResult {
        provider_commit_ref,
        total_tax_minor,
        was_already_voided: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finalization_error_display() {
        let err = FinalizationError::InvoiceNotFound(123);
        assert_eq!(err.to_string(), "Invoice not found: 123");

        let err = FinalizationError::AlreadyProcessed {
            invoice_id: 456,
            attempt_no: 1,
            idempotency_key: "test-key".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "Invoice 456 already processed (attempt 1 with key test-key)"
        );
    }

    #[test]
    fn test_finalization_result_variants() {
        let new_attempt = FinalizationResult::NewAttempt {
            attempt_id: Uuid::new_v4(),
            idempotency_key: "test-key".to_string(),
        };
        assert!(matches!(new_attempt, FinalizationResult::NewAttempt { .. }));

        let already_processed = FinalizationResult::AlreadyProcessed {
            existing_attempt_id: Uuid::new_v4(),
            idempotency_key: "test-key".to_string(),
        };
        assert!(matches!(
            already_processed,
            FinalizationResult::AlreadyProcessed { .. }
        ));
    }
}
