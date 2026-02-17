//! Reconciliation Matching Engine v1 — Deterministic Heuristics (bd-2cn)
//!
//! ## Matching Strategy
//!
//! The engine proposes matches between unmatched payments (charges with
//! status='succeeded') and open invoices using stable, deterministic rules:
//!
//! 1. **Exact match**: same customer + same amount + same currency → confidence 1.0
//! 2. **Reference match**: external reference ID correlation → confidence 0.95
//! 3. Payments left unmatched after all rules → exception (UnmatchedPayment)
//! 4. Multiple invoices match with equal score → exception (AmbiguousMatch)
//!
//! ## Invariants
//!
//! - **Deterministic**: same inputs always produce same matches across runs.
//! - **Append-only**: match decisions are immutable. Raw inputs are never mutated.
//! - **Atomic**: match persistence + outbox event in a single transaction.
//! - **Idempotent**: duplicate `recon_run_id` returns existing run without error.

use crate::events::{
    build_recon_exception_raised_envelope, build_recon_match_applied_envelope,
    build_recon_run_started_envelope, ReconExceptionKind, ReconExceptionRaisedPayload,
    ReconMatchAppliedPayload, ReconRunStartedPayload, EVENT_TYPE_RECON_EXCEPTION_RAISED,
    EVENT_TYPE_RECON_MATCH_APPLIED, EVENT_TYPE_RECON_RUN_STARTED,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

// ============================================================================
// Request / Response types
// ============================================================================

/// Request to execute a reconciliation matching run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReconRequest {
    /// Stable ID for this reconciliation run (idempotency anchor).
    pub recon_run_id: Uuid,
    /// Tenant identifier.
    pub app_id: String,
    /// Distributed trace correlation ID.
    pub correlation_id: String,
    /// Causation ID (event/action that triggered this run).
    pub causation_id: Option<String>,
}

/// Result of a reconciliation run.
#[derive(Debug, Clone, Serialize)]
pub struct ReconRunResult {
    pub recon_run_id: Uuid,
    pub status: String,
    pub payment_count: i32,
    pub invoice_count: i32,
    pub match_count: i32,
    pub exception_count: i32,
}

/// Result of checking for an existing run (idempotency).
#[derive(Debug, Clone)]
pub enum RunReconOutcome {
    /// New run executed.
    Executed(ReconRunResult),
    /// Run already exists (idempotency).
    AlreadyExists(ReconRunResult),
}

// ============================================================================
// Error types
// ============================================================================

#[derive(Debug)]
pub enum ReconError {
    /// Database error.
    DatabaseError(String),
    /// Serialization error.
    SerializationError(String),
}

impl fmt::Display for ReconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            Self::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
        }
    }
}

impl std::error::Error for ReconError {}

impl From<sqlx::Error> for ReconError {
    fn from(e: sqlx::Error) -> Self {
        Self::DatabaseError(e.to_string())
    }
}

// ============================================================================
// Internal row types
// ============================================================================

/// An unmatched payment (charge with status succeeded, no prior match).
#[derive(Debug, Clone)]
struct UnmatchedPayment {
    charge_id: i32,
    ar_customer_id: i32,
    amount_cents: i32,
    currency: String,
    reference_id: Option<String>,
}

/// An open invoice available for matching.
#[derive(Debug, Clone)]
struct OpenInvoice {
    invoice_id: i32,
    ar_customer_id: i32,
    amount_cents: i32,
    currency: String,
    tilled_invoice_id: String,
}

/// A proposed match from the matching engine.
#[derive(Debug, Clone)]
struct ProposedMatch {
    payment: UnmatchedPayment,
    invoice: OpenInvoice,
    matched_amount_minor: i64,
    confidence_score: f64,
    match_method: String,
}

/// A proposed exception from the matching engine.
#[derive(Debug, Clone)]
struct ProposedException {
    payment_id: Option<String>,
    invoice_id: Option<String>,
    exception_kind: ReconExceptionKind,
    description: String,
    amount_minor: Option<i64>,
    currency: Option<String>,
}

// ============================================================================
// Core function
// ============================================================================

/// Execute a reconciliation matching run.
///
/// **Idempotency**: if a run with the same `recon_run_id` exists, returns
/// `AlreadyExists` with the existing summary.
///
/// **Determinism**: payments and invoices are sorted by (customer_id, amount, id)
/// before matching. Each payment is matched to at most one invoice. The matching
/// order is stable across runs for the same input data.
///
/// **Atomicity**: all matches, exceptions, run record, and outbox events are
/// persisted in a single transaction.
pub async fn run_reconciliation(
    pool: &PgPool,
    req: RunReconRequest,
) -> Result<RunReconOutcome, ReconError> {
    // 1. Idempotency check: has this run already executed?
    let existing: Option<(String, i32, i32, i32, i32)> = sqlx::query_as(
        "SELECT status, payment_count, invoice_count, match_count, exception_count \
         FROM ar_recon_runs WHERE recon_run_id = $1",
    )
    .bind(req.recon_run_id)
    .fetch_optional(pool)
    .await?;

    if let Some((status, payment_count, invoice_count, match_count, exception_count)) = existing {
        return Ok(RunReconOutcome::AlreadyExists(ReconRunResult {
            recon_run_id: req.recon_run_id,
            status,
            payment_count,
            invoice_count,
            match_count,
            exception_count,
        }));
    }

    // 2. Load unmatched payments: succeeded charges not yet in ar_recon_matches.
    //    Sorted by (ar_customer_id, amount_cents, id) for determinism.
    let payments: Vec<UnmatchedPayment> = sqlx::query_as(
        r#"
        SELECT c.id AS charge_id, c.ar_customer_id, c.amount_cents, c.currency, c.reference_id
        FROM ar_charges c
        WHERE c.app_id = $1
          AND c.status = 'succeeded'
          AND NOT EXISTS (
              SELECT 1 FROM ar_recon_matches m
              WHERE m.app_id = $1 AND m.payment_id = c.id::TEXT
          )
        ORDER BY c.ar_customer_id, c.amount_cents, c.id
        "#,
    )
    .bind(&req.app_id)
    .fetch_all(pool)
    .await?;

    // 3. Load open invoices: status 'open', not yet matched.
    //    Sorted by (ar_customer_id, amount_cents, id) for determinism.
    let invoices: Vec<OpenInvoice> = sqlx::query_as(
        r#"
        SELECT i.id AS invoice_id, i.ar_customer_id, i.amount_cents, i.currency, i.tilled_invoice_id
        FROM ar_invoices i
        WHERE i.app_id = $1
          AND i.status = 'open'
          AND NOT EXISTS (
              SELECT 1 FROM ar_recon_matches m
              WHERE m.app_id = $1 AND m.invoice_id = i.id::TEXT
          )
        ORDER BY i.ar_customer_id, i.amount_cents, i.id
        "#,
    )
    .bind(&req.app_id)
    .fetch_all(pool)
    .await?;

    let payment_count = payments.len() as i32;
    let invoice_count = invoices.len() as i32;

    // 4. Run deterministic matching engine.
    let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);
    let match_count = matches.len() as i32;
    let exception_count = exceptions.len() as i32;
    let now = Utc::now();

    // 5. Persist everything in a single transaction.
    let mut tx = pool.begin().await?;

    // 5a. Insert the run record.
    sqlx::query(
        r#"
        INSERT INTO ar_recon_runs (
            recon_run_id, app_id, status, matching_strategy,
            payment_count, invoice_count, match_count, exception_count,
            started_at, finished_at, correlation_id
        )
        VALUES ($1, $2, 'completed', 'deterministic_v1', $3, $4, $5, $6, $7, $7, $8)
        "#,
    )
    .bind(req.recon_run_id)
    .bind(&req.app_id)
    .bind(payment_count)
    .bind(invoice_count)
    .bind(match_count)
    .bind(exception_count)
    .bind(now)
    .bind(&req.correlation_id)
    .execute(&mut *tx)
    .await?;

    // 5b. Emit recon_run_started outbox event.
    let run_event_id = Uuid::new_v4();
    let run_payload = ReconRunStartedPayload {
        tenant_id: req.app_id.clone(),
        recon_run_id: req.recon_run_id,
        payment_count,
        invoice_count,
        matching_strategy: "deterministic_v1".to_string(),
        started_at: now,
    };
    let run_envelope = build_recon_run_started_envelope(
        run_event_id,
        req.app_id.clone(),
        req.correlation_id.clone(),
        req.causation_id.clone(),
        run_payload,
    );
    let run_payload_json = serde_json::to_value(&run_envelope)
        .map_err(|e| ReconError::SerializationError(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO events_outbox (
            event_id, event_type, aggregate_type, aggregate_id, payload,
            tenant_id, source_module, mutation_class, schema_version,
            occurred_at, replay_safe, correlation_id, causation_id
        )
        VALUES ($1, $2, 'recon_run', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
        "#,
    )
    .bind(run_event_id)
    .bind(EVENT_TYPE_RECON_RUN_STARTED)
    .bind(req.recon_run_id.to_string())
    .bind(run_payload_json)
    .bind(&req.app_id)
    .bind(&run_envelope.schema_version)
    .bind(now)
    .bind(&req.correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // 5c. Insert matches and emit match events.
    let run_event_id_str = run_event_id.to_string();
    for m in &matches {
        let match_id = Uuid::new_v4();
        let match_event_id = Uuid::new_v4();

        // Insert match record (append-only).
        sqlx::query(
            r#"
            INSERT INTO ar_recon_matches (
                match_id, recon_run_id, app_id, payment_id, invoice_id,
                matched_amount_minor, currency, confidence_score, match_method, matched_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(match_id)
        .bind(req.recon_run_id)
        .bind(&req.app_id)
        .bind(m.payment.charge_id.to_string())
        .bind(m.invoice.invoice_id.to_string())
        .bind(m.matched_amount_minor)
        .bind(&m.invoice.currency)
        .bind(m.confidence_score)
        .bind(&m.match_method)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        // Emit ar.recon_match_applied event.
        let match_payload = ReconMatchAppliedPayload {
            tenant_id: req.app_id.clone(),
            recon_run_id: req.recon_run_id,
            payment_id: m.payment.charge_id.to_string(),
            invoice_id: m.invoice.invoice_id.to_string(),
            matched_amount_minor: m.matched_amount_minor,
            currency: m.invoice.currency.clone(),
            confidence_score: m.confidence_score,
            match_method: m.match_method.clone(),
            matched_at: now,
        };
        let match_envelope = build_recon_match_applied_envelope(
            match_event_id,
            req.app_id.clone(),
            req.correlation_id.clone(),
            Some(run_event_id_str.clone()),
            match_payload,
        );
        let match_payload_json = serde_json::to_value(&match_envelope)
            .map_err(|e| ReconError::SerializationError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO events_outbox (
                event_id, event_type, aggregate_type, aggregate_id, payload,
                tenant_id, source_module, mutation_class, schema_version,
                occurred_at, replay_safe, correlation_id, causation_id
            )
            VALUES ($1, $2, 'recon_match', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
            "#,
        )
        .bind(match_event_id)
        .bind(EVENT_TYPE_RECON_MATCH_APPLIED)
        .bind(match_id.to_string())
        .bind(match_payload_json)
        .bind(&req.app_id)
        .bind(&match_envelope.schema_version)
        .bind(now)
        .bind(&req.correlation_id)
        .bind(&run_event_id_str)
        .execute(&mut *tx)
        .await?;
    }

    // 5d. Insert exceptions and emit exception events.
    for exc in &exceptions {
        let exception_id = Uuid::new_v4();
        let exc_event_id = Uuid::new_v4();

        // Insert exception record (append-only).
        sqlx::query(
            r#"
            INSERT INTO ar_recon_exceptions (
                exception_id, recon_run_id, app_id, payment_id, invoice_id,
                exception_kind, description, amount_minor, currency, raised_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(exception_id)
        .bind(req.recon_run_id)
        .bind(&req.app_id)
        .bind(&exc.payment_id)
        .bind(&exc.invoice_id)
        .bind(exception_kind_to_str(&exc.exception_kind))
        .bind(&exc.description)
        .bind(exc.amount_minor)
        .bind(&exc.currency)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        // Emit ar.recon_exception_raised event.
        let exc_payload = ReconExceptionRaisedPayload {
            tenant_id: req.app_id.clone(),
            recon_run_id: req.recon_run_id,
            payment_id: exc.payment_id.clone(),
            invoice_id: exc.invoice_id.clone(),
            exception_kind: exc.exception_kind.clone(),
            description: exc.description.clone(),
            amount_minor: exc.amount_minor,
            currency: exc.currency.clone(),
            raised_at: now,
        };
        let exc_envelope = build_recon_exception_raised_envelope(
            exc_event_id,
            req.app_id.clone(),
            req.correlation_id.clone(),
            Some(run_event_id_str.clone()),
            exc_payload,
        );
        let exc_payload_json = serde_json::to_value(&exc_envelope)
            .map_err(|e| ReconError::SerializationError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO events_outbox (
                event_id, event_type, aggregate_type, aggregate_id, payload,
                tenant_id, source_module, mutation_class, schema_version,
                occurred_at, replay_safe, correlation_id, causation_id
            )
            VALUES ($1, $2, 'recon_exception', $3, $4, $5, 'ar', 'DATA_MUTATION', $6, $7, true, $8, $9)
            "#,
        )
        .bind(exc_event_id)
        .bind(EVENT_TYPE_RECON_EXCEPTION_RAISED)
        .bind(exception_id.to_string())
        .bind(exc_payload_json)
        .bind(&req.app_id)
        .bind(&exc_envelope.schema_version)
        .bind(now)
        .bind(&req.correlation_id)
        .bind(&run_event_id_str)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(RunReconOutcome::Executed(ReconRunResult {
        recon_run_id: req.recon_run_id,
        status: "completed".to_string(),
        payment_count,
        invoice_count,
        match_count,
        exception_count,
    }))
}

// ============================================================================
// Deterministic matching engine (pure logic)
// ============================================================================

/// Match payments to invoices using deterministic heuristic rules.
///
/// **Determinism guarantee**: Both input slices must be sorted by
/// (ar_customer_id, amount_cents, id). The matching algorithm processes
/// payments in order and greedily assigns the best match. Because both
/// sides are sorted identically, the same inputs always yield the same
/// output.
///
/// **Rules (evaluated in priority order):**
/// 1. Exact match: same customer, same amount, same currency → confidence 1.0
/// 2. Reference match: payment reference_id matches invoice tilled_invoice_id → confidence 0.95
///
/// Ties (equal confidence for multiple invoices) → AmbiguousMatch exception.
/// No match at all → UnmatchedPayment exception.
fn match_payments_to_invoices(
    payments: &[UnmatchedPayment],
    invoices: &[OpenInvoice],
) -> (Vec<ProposedMatch>, Vec<ProposedException>) {
    let mut matches = Vec::new();
    let mut exceptions = Vec::new();
    // Track which invoices have already been matched (by invoice_id).
    let mut matched_invoice_ids: std::collections::HashSet<i32> = std::collections::HashSet::new();

    // Build invoice lookup by customer for efficient searching.
    let mut invoices_by_customer: HashMap<i32, Vec<&OpenInvoice>> = HashMap::new();
    for inv in invoices {
        invoices_by_customer
            .entry(inv.ar_customer_id)
            .or_default()
            .push(inv);
    }

    for payment in payments {
        let mut candidates: Vec<(&OpenInvoice, f64, &str)> = Vec::new();

        // Rule 1: Exact match — same customer + same amount + same currency.
        if let Some(customer_invoices) = invoices_by_customer.get(&payment.ar_customer_id) {
            for inv in customer_invoices {
                if matched_invoice_ids.contains(&inv.invoice_id) {
                    continue;
                }
                if inv.amount_cents == payment.amount_cents
                    && inv.currency.eq_ignore_ascii_case(&payment.currency)
                {
                    candidates.push((inv, 1.0, "exact"));
                }
            }
        }

        // Rule 2: Reference match — payment reference_id matches invoice tilled_invoice_id.
        if candidates.is_empty() {
            if let Some(ref ref_id) = payment.reference_id {
                for inv in invoices {
                    if matched_invoice_ids.contains(&inv.invoice_id) {
                        continue;
                    }
                    if inv.tilled_invoice_id == *ref_id
                        && inv.currency.eq_ignore_ascii_case(&payment.currency)
                    {
                        candidates.push((inv, 0.95, "reference"));
                    }
                }
            }
        }

        match candidates.len() {
            0 => {
                // No match found → UnmatchedPayment exception.
                exceptions.push(ProposedException {
                    payment_id: Some(payment.charge_id.to_string()),
                    invoice_id: None,
                    exception_kind: ReconExceptionKind::UnmatchedPayment,
                    description: format!(
                        "No matching invoice found for payment {} (amount: {}, currency: {})",
                        payment.charge_id, payment.amount_cents, payment.currency
                    ),
                    amount_minor: Some(payment.amount_cents as i64),
                    currency: Some(payment.currency.clone()),
                });
            }
            1 => {
                // Single best match — apply it.
                let (inv, score, method) = candidates[0];
                matched_invoice_ids.insert(inv.invoice_id);
                matches.push(ProposedMatch {
                    payment: payment.clone(),
                    invoice: inv.clone(),
                    matched_amount_minor: payment.amount_cents as i64,
                    confidence_score: score,
                    match_method: method.to_string(),
                });
            }
            _ => {
                // Multiple candidates with equal confidence → ambiguous.
                // Check if all candidates share the same top score.
                let top_score = candidates
                    .iter()
                    .map(|(_, s, _)| *s)
                    .fold(f64::NEG_INFINITY, f64::max);
                let top_candidates: Vec<_> =
                    candidates.iter().filter(|(_, s, _)| (*s - top_score).abs() < f64::EPSILON).collect();

                if top_candidates.len() == 1 {
                    // One clear winner at the top score.
                    let (inv, score, method) = top_candidates[0];
                    matched_invoice_ids.insert(inv.invoice_id);
                    matches.push(ProposedMatch {
                        payment: payment.clone(),
                        invoice: (*inv).clone(),
                        matched_amount_minor: payment.amount_cents as i64,
                        confidence_score: *score,
                        match_method: method.to_string(),
                    });
                } else {
                    // True ambiguity — raise exception.
                    let invoice_ids: Vec<String> = top_candidates
                        .iter()
                        .map(|(inv, _, _)| inv.invoice_id.to_string())
                        .collect();
                    exceptions.push(ProposedException {
                        payment_id: Some(payment.charge_id.to_string()),
                        invoice_id: None,
                        exception_kind: ReconExceptionKind::AmbiguousMatch,
                        description: format!(
                            "Payment {} matches multiple invoices with equal confidence: [{}]",
                            payment.charge_id,
                            invoice_ids.join(", ")
                        ),
                        amount_minor: Some(payment.amount_cents as i64),
                        currency: Some(payment.currency.clone()),
                    });
                }
            }
        }
    }

    (matches, exceptions)
}

/// Convert ReconExceptionKind to the DB string representation.
fn exception_kind_to_str(kind: &ReconExceptionKind) -> &'static str {
    match kind {
        ReconExceptionKind::UnmatchedPayment => "unmatched_payment",
        ReconExceptionKind::UnmatchedInvoice => "unmatched_invoice",
        ReconExceptionKind::AmountMismatch => "amount_mismatch",
        ReconExceptionKind::AmbiguousMatch => "ambiguous_match",
        ReconExceptionKind::DuplicateReference => "duplicate_reference",
    }
}

// ============================================================================
// SQLx row mappings
// ============================================================================

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for UnmatchedPayment {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            charge_id: row.try_get("charge_id")?,
            ar_customer_id: row.try_get("ar_customer_id")?,
            amount_cents: row.try_get("amount_cents")?,
            currency: row.try_get("currency")?,
            reference_id: row.try_get("reference_id")?,
        })
    }
}

impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for OpenInvoice {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            invoice_id: row.try_get("invoice_id")?,
            ar_customer_id: row.try_get("ar_customer_id")?,
            amount_cents: row.try_get("amount_cents")?,
            currency: row.try_get("currency")?,
            tilled_invoice_id: row.try_get("tilled_invoice_id")?,
        })
    }
}

// ============================================================================
// Unit tests (pure logic — no DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_payment(id: i32, customer: i32, amount: i32, currency: &str, ref_id: Option<&str>) -> UnmatchedPayment {
        UnmatchedPayment {
            charge_id: id,
            ar_customer_id: customer,
            amount_cents: amount,
            currency: currency.to_string(),
            reference_id: ref_id.map(|s| s.to_string()),
        }
    }

    fn make_invoice(id: i32, customer: i32, amount: i32, currency: &str, tilled_id: &str) -> OpenInvoice {
        OpenInvoice {
            invoice_id: id,
            ar_customer_id: customer,
            amount_cents: amount,
            currency: currency.to_string(),
            tilled_invoice_id: tilled_id.to_string(),
        }
    }

    #[test]
    fn exact_match_single_payment_single_invoice() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![make_invoice(10, 100, 5000, "usd", "inv-10")];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 1);
        assert_eq!(exceptions.len(), 0);
        assert_eq!(matches[0].payment.charge_id, 1);
        assert_eq!(matches[0].invoice.invoice_id, 10);
        assert_eq!(matches[0].confidence_score, 1.0);
        assert_eq!(matches[0].match_method, "exact");
    }

    #[test]
    fn unmatched_payment_raises_exception() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![make_invoice(10, 200, 5000, "usd", "inv-10")]; // different customer

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0].exception_kind, ReconExceptionKind::UnmatchedPayment);
        assert_eq!(exceptions[0].payment_id, Some("1".to_string()));
    }

    #[test]
    fn reference_match_when_no_exact_match() {
        let payments = vec![make_payment(1, 100, 5000, "usd", Some("inv-10"))];
        let invoices = vec![make_invoice(10, 200, 3000, "usd", "inv-10")]; // different customer & amount

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 1);
        assert_eq!(exceptions.len(), 0);
        assert_eq!(matches[0].confidence_score, 0.95);
        assert_eq!(matches[0].match_method, "reference");
    }

    #[test]
    fn ambiguous_match_raises_exception() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![
            make_invoice(10, 100, 5000, "usd", "inv-10"),
            make_invoice(11, 100, 5000, "usd", "inv-11"),
        ];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0].exception_kind, ReconExceptionKind::AmbiguousMatch);
    }

    #[test]
    fn multiple_payments_matched_correctly() {
        let payments = vec![
            make_payment(1, 100, 5000, "usd", None),
            make_payment(2, 200, 3000, "usd", None),
        ];
        let invoices = vec![
            make_invoice(10, 100, 5000, "usd", "inv-10"),
            make_invoice(11, 200, 3000, "usd", "inv-11"),
        ];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 2);
        assert_eq!(exceptions.len(), 0);
        // First payment matches first invoice
        assert_eq!(matches[0].payment.charge_id, 1);
        assert_eq!(matches[0].invoice.invoice_id, 10);
        // Second payment matches second invoice
        assert_eq!(matches[1].payment.charge_id, 2);
        assert_eq!(matches[1].invoice.invoice_id, 11);
    }

    #[test]
    fn currency_mismatch_prevents_match() {
        let payments = vec![make_payment(1, 100, 5000, "usd", None)];
        let invoices = vec![make_invoice(10, 100, 5000, "eur", "inv-10")];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(exceptions[0].exception_kind, ReconExceptionKind::UnmatchedPayment);
    }

    #[test]
    fn determinism_across_runs() {
        let payments = vec![
            make_payment(1, 100, 5000, "usd", None),
            make_payment(2, 200, 3000, "usd", None),
            make_payment(3, 300, 7000, "usd", None),
        ];
        let invoices = vec![
            make_invoice(10, 100, 5000, "usd", "inv-10"),
            make_invoice(11, 200, 3000, "usd", "inv-11"),
        ];

        // Run twice — must produce identical results.
        let (matches1, exceptions1) = match_payments_to_invoices(&payments, &invoices);
        let (matches2, exceptions2) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches1.len(), matches2.len());
        assert_eq!(exceptions1.len(), exceptions2.len());
        for (m1, m2) in matches1.iter().zip(matches2.iter()) {
            assert_eq!(m1.payment.charge_id, m2.payment.charge_id);
            assert_eq!(m1.invoice.invoice_id, m2.invoice.invoice_id);
            assert_eq!(m1.confidence_score, m2.confidence_score);
            assert_eq!(m1.match_method, m2.match_method);
        }
    }

    #[test]
    fn empty_inputs_produce_no_output() {
        let (matches, exceptions) = match_payments_to_invoices(&[], &[]);
        assert_eq!(matches.len(), 0);
        assert_eq!(exceptions.len(), 0);
    }

    #[test]
    fn already_matched_invoice_not_reused() {
        // Two payments, same amount, same customer — but only one invoice available.
        // First payment matches; second becomes unmatched.
        let payments = vec![
            make_payment(1, 100, 5000, "usd", None),
            make_payment(2, 100, 5000, "usd", None),
        ];
        let invoices = vec![make_invoice(10, 100, 5000, "usd", "inv-10")];

        let (matches, exceptions) = match_payments_to_invoices(&payments, &invoices);

        assert_eq!(matches.len(), 1);
        assert_eq!(exceptions.len(), 1);
        assert_eq!(matches[0].payment.charge_id, 1); // first payment wins
        assert_eq!(exceptions[0].exception_kind, ReconExceptionKind::UnmatchedPayment);
        assert_eq!(exceptions[0].payment_id, Some("2".to_string()));
    }

    #[test]
    fn exception_kind_to_str_roundtrip() {
        assert_eq!(exception_kind_to_str(&ReconExceptionKind::UnmatchedPayment), "unmatched_payment");
        assert_eq!(exception_kind_to_str(&ReconExceptionKind::UnmatchedInvoice), "unmatched_invoice");
        assert_eq!(exception_kind_to_str(&ReconExceptionKind::AmountMismatch), "amount_mismatch");
        assert_eq!(exception_kind_to_str(&ReconExceptionKind::AmbiguousMatch), "ambiguous_match");
        assert_eq!(exception_kind_to_str(&ReconExceptionKind::DuplicateReference), "duplicate_reference");
    }
}
