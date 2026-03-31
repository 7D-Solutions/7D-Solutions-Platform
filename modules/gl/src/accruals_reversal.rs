//! Accrual auto-reversal engine (bd-2ob)
//!
//! Executes auto-reversals for accrual instances scheduled to reverse in a target period.
//! Each reversal posts a compensating (swapped debit/credit) journal entry atomically
//! with the reversal record and outbox event.
//!
//! ## Exactly-once guarantee
//! - Deterministic reversal_id = Uuid::v5(original_accrual_id, "reversal")
//! - Deterministic event_id = Uuid::v5(original_accrual_id, "reversal_event")
//! - processed_events dedupe prevents duplicate journal postings on replay
//! - gl_accrual_reversals.original_accrual_id UNIQUE constraint prevents double reversal

use chrono::{NaiveDate, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::accruals::{
    AccrualError, ExecuteReversalsRequest, ExecuteReversalsResult, ReversalResult,
};
use crate::events::contracts::{
    AccrualReversedPayload, CashFlowClass, EVENT_TYPE_ACCRUAL_REVERSED, MUTATION_CLASS_REVERSAL,
};
use crate::repos::outbox_repo;

/// Execute auto-reversals for all accruals scheduled to reverse in `target_period`.
///
/// Finds accrual instances from the prior period that have
/// `reversal_policy.auto_reverse_next_period = true` and have not yet been reversed.
/// For each, posts a compensating (reversed debit/credit) journal entry atomically
/// with the reversal record and outbox event.
///
/// ## Idempotency
/// If a reversal already exists (idempotency_key match), it is counted as
/// `reversals_skipped` and the existing record is returned.
pub async fn execute_auto_reversals(
    db: &PgPool,
    req: &ExecuteReversalsRequest,
) -> Result<ExecuteReversalsResult, AccrualError> {
    let reversal_date = NaiveDate::parse_from_str(&req.reversal_date, "%Y-%m-%d")
        .map_err(|e| AccrualError::Validation(format!("Invalid reversal_date: {}", e)))?;

    let prior_period = compute_prior_period(&req.target_period).ok_or_else(|| {
        AccrualError::Validation(format!("Invalid target_period: {}", req.target_period))
    })?;

    let candidates = sqlx::query(
        r#"
        SELECT i.instance_id, i.accrual_id, i.template_id, i.tenant_id,
               i.period, i.debit_account, i.credit_account,
               i.amount_minor, i.currency, i.reversal_policy, i.cashflow_class,
               i.journal_entry_id AS original_journal_entry_id, i.outbox_event_id
        FROM gl_accrual_instances i
        WHERE i.tenant_id = $1
          AND i.period = $2
          AND i.status = 'posted'
          AND (i.reversal_policy->>'auto_reverse_next_period')::boolean = true
          AND NOT EXISTS (
              SELECT 1 FROM gl_accrual_reversals r
              WHERE r.original_accrual_id = i.accrual_id
          )
        ORDER BY i.created_at
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&prior_period)
    .fetch_all(db)
    .await
    .map_err(AccrualError::Database)?;

    let mut results = Vec::new();
    let mut reversals_executed = 0usize;
    let mut reversals_skipped = 0usize;

    for row in &candidates {
        let instance_id: Uuid = row.get("instance_id");
        let accrual_id: Uuid = row.get("accrual_id");
        let template_id: Uuid = row.get("template_id");
        let original_debit: String = row.get("debit_account");
        let original_credit: String = row.get("credit_account");
        let amount_minor: i64 = row.get("amount_minor");
        let currency: String = row.get("currency");
        let cashflow_str: String = row.get("cashflow_class");
        let original_event_id: Option<Uuid> = row.get("outbox_event_id");

        // Deterministic IDs for exactly-once
        let reversal_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("reversal:{}", accrual_id).as_bytes(),
        );
        let event_id = Uuid::new_v5(
            &Uuid::NAMESPACE_OID,
            format!("reversal_event:{}", accrual_id).as_bytes(),
        );
        let idem_key = format!("reversal:{}:{}", accrual_id, req.target_period);

        // Idempotency check: already reversed?
        let existing = sqlx::query(
            r#"
            SELECT reversal_id, journal_entry_id, amount_minor, currency
            FROM gl_accrual_reversals
            WHERE idempotency_key = $1
            "#,
        )
        .bind(&idem_key)
        .fetch_optional(db)
        .await
        .map_err(AccrualError::Database)?;

        if let Some(existing_row) = existing {
            reversals_skipped += 1;
            results.push(ReversalResult {
                reversal_id: existing_row.get("reversal_id"),
                original_accrual_id: accrual_id,
                journal_entry_id: existing_row.get("journal_entry_id"),
                amount_minor: existing_row.get("amount_minor"),
                currency: existing_row.get("currency"),
                idempotent_hit: true,
            });
            continue;
        }

        // Check processed_events for this event_id (replay dedupe)
        let already_processed: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM processed_events WHERE event_id = $1)")
                .bind(event_id)
                .fetch_one(db)
                .await
                .map_err(AccrualError::Database)?;

        if already_processed {
            reversals_skipped += 1;
            continue;
        }

        // Reversal swaps debit/credit accounts
        let rev_debit = original_credit.clone();
        let rev_credit = original_debit.clone();

        let posted_at = reversal_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| AccrualError::Validation("Invalid reversal time".to_string()))?
            .and_utc();

        let reversal_name = format!("Reversal of accrual {}", accrual_id);
        let cashflow_class = parse_cashflow_class(&cashflow_str);

        // Begin atomic transaction
        let mut tx = db.begin().await.map_err(AccrualError::Database)?;

        // 1. Post reversing journal entry
        let journal_entry_id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO journal_entries (
                id, tenant_id, source_module, source_event_id, source_subject,
                posted_at, currency, description, reference_type, reference_id, correlation_id
            )
            VALUES ($1, $2, 'gl', $3, 'accrual_reversal', $4, $5, $6, 'GL_ACCRUAL_REVERSAL', $7, $8)
            "#,
        )
        .bind(journal_entry_id)
        .bind(&req.tenant_id)
        .bind(event_id)
        .bind(posted_at)
        .bind(&currency)
        .bind(&reversal_name)
        .bind(reversal_id.to_string())
        .bind(Some(accrual_id))
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // Debit line (swapped: was credit in original)
        sqlx::query(
            r#"
            INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
            VALUES ($1, $2, 1, $3, $4, 0, $5)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(journal_entry_id)
        .bind(&rev_debit)
        .bind(amount_minor)
        .bind(format!("Reversal DR: {}", reversal_name))
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // Credit line (swapped: was debit in original)
        sqlx::query(
            r#"
            INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
            VALUES ($1, $2, 2, $3, 0, $4, $5)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(journal_entry_id)
        .bind(&rev_credit)
        .bind(amount_minor)
        .bind(format!("Reversal CR: {}", reversal_name))
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // 2. Mark event as processed (dedupe on replay)
        sqlx::query(
            r#"
            INSERT INTO processed_events (event_id, event_type, processor)
            VALUES ($1, $2, 'gl-accrual-reversal')
            ON CONFLICT (event_id) DO NOTHING
            "#,
        )
        .bind(event_id)
        .bind(EVENT_TYPE_ACCRUAL_REVERSED)
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // 3. Insert reversal record with linkage
        sqlx::query(
            r#"
            INSERT INTO gl_accrual_reversals (
                reversal_id, original_accrual_id, original_instance_id,
                tenant_id, reversal_period, reversal_date,
                debit_account, credit_account, amount_minor, currency,
                journal_entry_id, outbox_event_id, idempotency_key, reason
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "#,
        )
        .bind(reversal_id)
        .bind(accrual_id)
        .bind(instance_id)
        .bind(&req.tenant_id)
        .bind(&req.target_period)
        .bind(reversal_date)
        .bind(&rev_debit)
        .bind(&rev_credit)
        .bind(amount_minor)
        .bind(&currency)
        .bind(journal_entry_id)
        .bind(event_id)
        .bind(&idem_key)
        .bind("auto_reverse_next_period")
        .execute(&mut *tx)
        .await
        .map_err(AccrualError::Database)?;

        // 4. Update accrual instance status to 'reversed'
        sqlx::query("UPDATE gl_accrual_instances SET status = 'reversed' WHERE instance_id = $1")
            .bind(instance_id)
            .execute(&mut *tx)
            .await
            .map_err(AccrualError::Database)?;

        // 5. Emit gl.accrual_reversed outbox event
        let payload = AccrualReversedPayload {
            reversal_id,
            original_accrual_id: accrual_id,
            template_id: Some(template_id),
            tenant_id: req.tenant_id.clone(),
            reversal_period: req.target_period.clone(),
            reversal_date: req.reversal_date.clone(),
            debit_account: rev_debit.clone(),
            credit_account: rev_credit.clone(),
            amount_minor,
            currency: currency.clone(),
            cashflow_class,
            journal_entry_id: Some(journal_entry_id),
            reason: "auto_reverse_next_period".to_string(),
            reversed_at: Utc::now(),
        };

        let event_payload = serde_json::to_value(&payload).map_err(AccrualError::Serialization)?;

        outbox_repo::insert_outbox_event_with_linkage(
            &mut tx,
            event_id,
            EVENT_TYPE_ACCRUAL_REVERSED,
            "accrual",
            &accrual_id.to_string(),
            event_payload,
            original_event_id, // reverses_event_id — links to original accrual's outbox event
            None,              // supersedes_event_id — not applicable
            MUTATION_CLASS_REVERSAL,
        )
        .await
        .map_err(AccrualError::Database)?;

        tx.commit().await.map_err(AccrualError::Database)?;

        reversals_executed += 1;
        results.push(ReversalResult {
            reversal_id,
            original_accrual_id: accrual_id,
            journal_entry_id,
            amount_minor,
            currency,
            idempotent_hit: false,
        });
    }

    Ok(ExecuteReversalsResult {
        target_period: req.target_period.clone(),
        reversals_executed,
        reversals_skipped,
        results,
    })
}

/// Compute the period (YYYY-MM) immediately before the given period.
fn compute_prior_period(period: &str) -> Option<String> {
    let parts: Vec<&str> = period.split('-').collect();
    if parts.len() != 2 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    if month == 1 {
        Some(format!("{}-12", year - 1))
    } else {
        Some(format!("{}-{:02}", year, month - 1))
    }
}

fn parse_cashflow_class(s: &str) -> CashFlowClass {
    match s {
        "investing" => CashFlowClass::Investing,
        "financing" => CashFlowClass::Financing,
        "non_cash" => CashFlowClass::NonCash,
        _ => CashFlowClass::Operating,
    }
}
