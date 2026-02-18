//! Reconciliation orchestration engine — DB persistence + outbox events.
//!
//! This module handles the full lifecycle of a reconciliation run:
//! idempotency check, data loading, invoking the matching algorithm,
//! and persisting results (matches, exceptions, outbox events) atomically.
//!
//! ## Performance (bd-h9et)
//!
//! Matches and exceptions are persisted with two bulk UNNEST INSERTs each
//! (one for the domain table, one for events_outbox) instead of N individual
//! round-trips. Payment/invoice loads use LEFT JOIN anti-joins which the
//! planner can satisfy with hash-anti or merge-anti strategies.

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_recon_exception_raised_envelope, build_recon_match_applied_envelope,
    build_recon_run_started_envelope, ReconExceptionRaisedPayload, ReconMatchAppliedPayload,
    ReconRunStartedPayload, EVENT_TYPE_RECON_EXCEPTION_RAISED, EVENT_TYPE_RECON_MATCH_APPLIED,
    EVENT_TYPE_RECON_RUN_STARTED,
};

use super::matching::{exception_kind_to_str, match_payments_to_invoices};
use super::{
    OpenInvoice, ReconError, ReconRunResult, RunReconOutcome, RunReconRequest, UnmatchedPayment,
};

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
/// persisted in a single transaction using bulk UNNEST INSERTs.
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

    // 2. Load unmatched payments via LEFT JOIN anti-join (avoids correlated NOT EXISTS).
    //    Sorted by (ar_customer_id, amount_cents, id) for determinism.
    let payments: Vec<UnmatchedPayment> = sqlx::query_as(
        r#"
        SELECT c.id AS charge_id, c.ar_customer_id, c.amount_cents, c.currency, c.reference_id
        FROM ar_charges c
        LEFT JOIN ar_recon_matches m
            ON m.app_id = $1 AND m.payment_id = c.id::TEXT
        WHERE c.app_id = $1
          AND c.status = 'succeeded'
          AND m.payment_id IS NULL
        ORDER BY c.ar_customer_id, c.amount_cents, c.id
        "#,
    )
    .bind(&req.app_id)
    .fetch_all(pool)
    .await?;

    // 3. Load open invoices via LEFT JOIN anti-join.
    //    Sorted by (ar_customer_id, amount_cents, id) for determinism.
    let invoices: Vec<OpenInvoice> = sqlx::query_as(
        r#"
        SELECT i.id AS invoice_id, i.ar_customer_id, i.amount_cents, i.currency, i.tilled_invoice_id
        FROM ar_invoices i
        LEFT JOIN ar_recon_matches m
            ON m.app_id = $1 AND m.invoice_id = i.id::TEXT
        WHERE i.app_id = $1
          AND i.status = 'open'
          AND m.invoice_id IS NULL
        ORDER BY i.ar_customer_id, i.amount_cents, i.id
        "#,
    )
    .bind(&req.app_id)
    .fetch_all(pool)
    .await?;

    let payment_count = payments.len() as i32;
    let invoice_count = invoices.len() as i32;

    // 4. Run deterministic matching engine (pure, no I/O).
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

    let run_event_id_str = run_event_id.to_string();

    // 5c. Batch insert all matches + their outbox events (2 round-trips total).
    if !matches.is_empty() {
        let mut match_ids: Vec<Uuid> = Vec::with_capacity(matches.len());
        let mut m_event_ids: Vec<Uuid> = Vec::with_capacity(matches.len());
        let mut m_match_id_strs: Vec<String> = Vec::with_capacity(matches.len());
        let mut m_payment_ids: Vec<String> = Vec::with_capacity(matches.len());
        let mut m_invoice_ids: Vec<String> = Vec::with_capacity(matches.len());
        let mut m_amounts: Vec<i64> = Vec::with_capacity(matches.len());
        let mut m_currencies: Vec<String> = Vec::with_capacity(matches.len());
        let mut m_scores: Vec<f64> = Vec::with_capacity(matches.len());
        let mut m_methods: Vec<String> = Vec::with_capacity(matches.len());
        let mut m_outbox_payloads: Vec<String> = Vec::with_capacity(matches.len());
        let mut m_schema_versions: Vec<String> = Vec::with_capacity(matches.len());

        for m in &matches {
            let match_id = Uuid::new_v4();
            let match_event_id = Uuid::new_v4();

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
            let payload_str = serde_json::to_string(&match_envelope)
                .map_err(|e| ReconError::SerializationError(e.to_string()))?;

            match_ids.push(match_id);
            m_event_ids.push(match_event_id);
            m_match_id_strs.push(match_id.to_string());
            m_payment_ids.push(m.payment.charge_id.to_string());
            m_invoice_ids.push(m.invoice.invoice_id.to_string());
            m_amounts.push(m.matched_amount_minor);
            m_currencies.push(m.invoice.currency.clone());
            m_scores.push(m.confidence_score);
            m_methods.push(m.match_method.clone());
            m_outbox_payloads.push(payload_str);
            m_schema_versions.push(match_envelope.schema_version.clone());
        }

        // Single bulk INSERT for all match records.
        sqlx::query(
            r#"
            INSERT INTO ar_recon_matches (
                match_id, recon_run_id, app_id, payment_id, invoice_id,
                matched_amount_minor, currency, confidence_score, match_method, matched_at
            )
            SELECT
                unnest($1::uuid[]), $2, $3,
                unnest($4::text[]), unnest($5::text[]),
                unnest($6::int8[]), unnest($7::text[]),
                unnest($8::float8[]), unnest($9::text[]),
                $10
            "#,
        )
        .bind(&match_ids)
        .bind(req.recon_run_id)
        .bind(&req.app_id)
        .bind(&m_payment_ids)
        .bind(&m_invoice_ids)
        .bind(&m_amounts)
        .bind(&m_currencies)
        .bind(&m_scores)
        .bind(&m_methods)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        // Single bulk INSERT for all match outbox events.
        sqlx::query(
            r#"
            INSERT INTO events_outbox (
                event_id, event_type, aggregate_type, aggregate_id, payload,
                tenant_id, source_module, mutation_class, schema_version,
                occurred_at, replay_safe, correlation_id, causation_id
            )
            SELECT
                unnest($1::uuid[]),
                $2, 'recon_match',
                unnest($3::text[]),
                unnest($4::text[])::jsonb,
                $5, 'ar', 'DATA_MUTATION',
                unnest($6::text[]),
                $7, true, $8, $9
            "#,
        )
        .bind(&m_event_ids)
        .bind(EVENT_TYPE_RECON_MATCH_APPLIED)
        .bind(&m_match_id_strs)
        .bind(&m_outbox_payloads)
        .bind(&req.app_id)
        .bind(&m_schema_versions)
        .bind(now)
        .bind(&req.correlation_id)
        .bind(&run_event_id_str)
        .execute(&mut *tx)
        .await?;
    }

    // 5d. Batch insert all exceptions + their outbox events (2 round-trips total).
    if !exceptions.is_empty() {
        let mut exc_ids: Vec<Uuid> = Vec::with_capacity(exceptions.len());
        let mut exc_event_ids: Vec<Uuid> = Vec::with_capacity(exceptions.len());
        let mut exc_id_strs: Vec<String> = Vec::with_capacity(exceptions.len());
        let mut exc_payment_ids: Vec<Option<String>> = Vec::with_capacity(exceptions.len());
        let mut exc_invoice_ids: Vec<Option<String>> = Vec::with_capacity(exceptions.len());
        let mut exc_kinds: Vec<String> = Vec::with_capacity(exceptions.len());
        let mut exc_descs: Vec<String> = Vec::with_capacity(exceptions.len());
        let mut exc_amounts: Vec<Option<i64>> = Vec::with_capacity(exceptions.len());
        let mut exc_currencies: Vec<Option<String>> = Vec::with_capacity(exceptions.len());
        let mut exc_outbox_payloads: Vec<String> = Vec::with_capacity(exceptions.len());
        let mut exc_schema_versions: Vec<String> = Vec::with_capacity(exceptions.len());

        for exc in &exceptions {
            let exception_id = Uuid::new_v4();
            let exc_event_id = Uuid::new_v4();

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
            let payload_str = serde_json::to_string(&exc_envelope)
                .map_err(|e| ReconError::SerializationError(e.to_string()))?;

            exc_ids.push(exception_id);
            exc_event_ids.push(exc_event_id);
            exc_id_strs.push(exception_id.to_string());
            exc_payment_ids.push(exc.payment_id.clone());
            exc_invoice_ids.push(exc.invoice_id.clone());
            exc_kinds.push(exception_kind_to_str(&exc.exception_kind).to_string());
            exc_descs.push(exc.description.clone());
            exc_amounts.push(exc.amount_minor);
            exc_currencies.push(exc.currency.clone());
            exc_outbox_payloads.push(payload_str);
            exc_schema_versions.push(exc_envelope.schema_version.clone());
        }

        // Single bulk INSERT for all exception records.
        sqlx::query(
            r#"
            INSERT INTO ar_recon_exceptions (
                exception_id, recon_run_id, app_id, payment_id, invoice_id,
                exception_kind, description, amount_minor, currency, raised_at
            )
            SELECT
                unnest($1::uuid[]), $2, $3,
                unnest($4::text[]), unnest($5::text[]),
                unnest($6::text[]), unnest($7::text[]),
                unnest($8::int8[]), unnest($9::text[]),
                $10
            "#,
        )
        .bind(&exc_ids)
        .bind(req.recon_run_id)
        .bind(&req.app_id)
        .bind(&exc_payment_ids)
        .bind(&exc_invoice_ids)
        .bind(&exc_kinds)
        .bind(&exc_descs)
        .bind(&exc_amounts)
        .bind(&exc_currencies)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        // Single bulk INSERT for all exception outbox events.
        sqlx::query(
            r#"
            INSERT INTO events_outbox (
                event_id, event_type, aggregate_type, aggregate_id, payload,
                tenant_id, source_module, mutation_class, schema_version,
                occurred_at, replay_safe, correlation_id, causation_id
            )
            SELECT
                unnest($1::uuid[]),
                $2, 'recon_exception',
                unnest($3::text[]),
                unnest($4::text[])::jsonb,
                $5, 'ar', 'DATA_MUTATION',
                unnest($6::text[]),
                $7, true, $8, $9
            "#,
        )
        .bind(&exc_event_ids)
        .bind(EVENT_TYPE_RECON_EXCEPTION_RAISED)
        .bind(&exc_id_strs)
        .bind(&exc_outbox_payloads)
        .bind(&req.app_id)
        .bind(&exc_schema_versions)
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
