//! Unrealized FX Revaluation Service (Phase 23a, bd-1yu)
//!
//! At period close, identifies open foreign-currency balances and posts
//! balanced revaluation journal entries in the reporting currency.
//!
//! ## Design
//!
//! 1. Query all `account_balances` where `currency != reporting_currency`
//! 2. For each, look up opening rate (as-of period_start) and closing rate (as-of period_end)
//! 3. Compute adjustment = balance * closing_rate - balance * opening_rate
//! 4. Post a single balanced journal entry with all adjustments
//!
//! ## Idempotency
//!
//! Uses a deterministic `source_event_id` (UUID v5 of period_id) so the journal
//! insert fails on duplicate. Since this runs inside the period close transaction
//! (which is itself idempotent via closed_at check), this is defense-in-depth.

use chrono::{DateTime, NaiveDate, Utc};
use sqlx::{Postgres, Transaction};
use thiserror::Error;
use uuid::Uuid;

use std::collections::HashMap;

use crate::repos::{balance_repo, journal_repo};
use crate::services::currency_conversion::{ConversionError, RateSnapshot};

use super::fx_helpers::{convert_with_sign, deterministic_event_id, lookup_rate_tx};

/// Well-known account code for unrealized FX gain/loss.
///
/// This account must exist in the tenant's chart of accounts.
/// Gains are posted as credits, losses as debits.
pub const UNREALIZED_FX_GAIN_LOSS_ACCOUNT: &str = "7100";

#[derive(Debug, Error)]
pub enum FxRevaluationError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Conversion error: {0}")]
    Conversion(#[from] ConversionError),

    #[error("Balance error: {0}")]
    Balance(#[from] balance_repo::BalanceError),
}

/// Individual adjustment computed for one account/currency balance.
#[derive(Debug, Clone)]
pub struct RevaluationAdjustment {
    pub account_code: String,
    pub currency: String,
    pub net_balance_minor: i64,
    pub opening_value_minor: i64,
    pub closing_value_minor: i64,
    /// Positive = gain, negative = loss (in reporting currency minor units)
    pub adjustment_minor: i64,
}

/// Result of the revaluation step.
#[derive(Debug, Clone)]
pub struct RevaluationResult {
    /// Journal entry ID if any adjustments were posted (None if no FX balances)
    pub journal_entry_id: Option<Uuid>,
    /// Individual adjustments computed
    pub adjustments: Vec<RevaluationAdjustment>,
}

/// Foreign-currency balance row from the query.
#[derive(Debug, sqlx::FromRow)]
struct ForeignCurrencyBalance {
    account_code: String,
    currency: String,
    net_balance_minor: i64,
}

/// Revalue foreign-currency balances at period close.
///
/// Called inside the `close_period` transaction AFTER validation passes
/// and BEFORE the sealed snapshot is created. This ensures revaluation
/// entries are included in the snapshot hash.
///
/// # Arguments
/// * `tx` - Period close transaction (for posting journals + updating balances)
/// * `tenant_id` - Tenant identifier
/// * `period_id` - Accounting period being closed
/// * `period_start` - Period start date (for opening rate lookup)
/// * `period_end` - Period end date (for closing rate lookup)
/// * `reporting_currency` - Tenant's reporting/functional currency (e.g., "USD")
///
/// # Returns
/// `RevaluationResult` with the journal entry ID and adjustments list.
/// Returns an empty result (no entry) if no foreign-currency balances exist.
pub async fn revalue_foreign_balances(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: &str,
    period_id: Uuid,
    period_start: NaiveDate,
    period_end: NaiveDate,
    reporting_currency: &str,
) -> Result<RevaluationResult, FxRevaluationError> {
    // Step 1: Check if revaluation already done (defense-in-depth)
    let reval_event_id = deterministic_event_id(period_id);
    let already_exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM journal_entries WHERE source_event_id = $1)",
    )
    .bind(reval_event_id)
    .fetch_one(&mut **tx)
    .await?;

    if already_exists {
        return Ok(RevaluationResult {
            journal_entry_id: None,
            adjustments: vec![],
        });
    }

    // Step 2: Query foreign-currency balances for this period
    let foreign_balances = sqlx::query_as::<_, ForeignCurrencyBalance>(
        r#"
        SELECT account_code, currency, net_balance_minor
        FROM account_balances
        WHERE tenant_id = $1
          AND period_id = $2
          AND UPPER(currency) != UPPER($3)
          AND net_balance_minor != 0
        ORDER BY account_code, currency
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .bind(reporting_currency)
    .fetch_all(&mut **tx)
    .await?;

    if foreign_balances.is_empty() {
        return Ok(RevaluationResult {
            journal_entry_id: None,
            adjustments: vec![],
        });
    }

    // Step 3: Compute adjustments for each balance
    let opening_as_of = period_start.and_hms_opt(23, 59, 59).unwrap().and_utc();
    let closing_as_of = period_end.and_hms_opt(23, 59, 59).unwrap().and_utc();

    let mut adjustments = Vec::new();

    // Group rate lookups by currency to avoid redundant queries
    let mut currencies: Vec<String> = foreign_balances
        .iter()
        .map(|b| b.currency.to_uppercase())
        .collect();
    currencies.sort();
    currencies.dedup();

    // Cache rates per currency
    let mut opening_rates: HashMap<String, RateSnapshot> = HashMap::new();
    let mut closing_rates: HashMap<String, RateSnapshot> = HashMap::new();

    for currency in &currencies {
        // Look up opening rate
        let opening =
            lookup_rate_tx(tx, tenant_id, currency, reporting_currency, opening_as_of).await?;
        // Look up closing rate
        let closing =
            lookup_rate_tx(tx, tenant_id, currency, reporting_currency, closing_as_of).await?;

        match (opening, closing) {
            (Some(o), Some(c)) => {
                opening_rates.insert(currency.clone(), o);
                closing_rates.insert(currency.clone(), c);
            }
            (None, _) | (_, None) => {
                // No rate available — skip this currency with a warning
                tracing::warn!(
                    tenant_id = %tenant_id,
                    currency = %currency,
                    reporting_currency = %reporting_currency,
                    "Skipping FX revaluation for currency: no rate available"
                );
            }
        }
    }

    // Compute adjustments
    for balance in &foreign_balances {
        let currency = balance.currency.to_uppercase();
        let opening_rate = match opening_rates.get(&currency) {
            Some(r) => r,
            None => continue, // skipped due to missing rate
        };
        let closing_rate = match closing_rates.get(&currency) {
            Some(r) => r,
            None => continue,
        };

        let opening_value = convert_with_sign(
            balance.net_balance_minor,
            opening_rate,
            &currency,
            reporting_currency,
        )?;
        let closing_value = convert_with_sign(
            balance.net_balance_minor,
            closing_rate,
            &currency,
            reporting_currency,
        )?;

        let adjustment = closing_value - opening_value;

        if adjustment == 0 {
            continue; // no movement, skip
        }

        adjustments.push(RevaluationAdjustment {
            account_code: balance.account_code.clone(),
            currency: currency.clone(),
            net_balance_minor: balance.net_balance_minor,
            opening_value_minor: opening_value,
            closing_value_minor: closing_value,
            adjustment_minor: adjustment,
        });
    }

    if adjustments.is_empty() {
        return Ok(RevaluationResult {
            journal_entry_id: None,
            adjustments: vec![],
        });
    }

    // Step 4: Post balanced journal entry
    let entry_id = Uuid::new_v4();
    let posted_at: DateTime<Utc> = period_end.and_hms_opt(23, 59, 59).unwrap().and_utc();

    journal_repo::insert_entry(
        tx,
        entry_id,
        tenant_id,
        "gl",
        reval_event_id,
        "gl.revaluation.period_close",
        posted_at,
        reporting_currency,
        Some("Unrealized FX revaluation at period close"),
        Some("FX_REVALUATION"),
        Some(&period_id.to_string()),
        None, // no correlation_id for system-generated entries
    )
    .await?;

    // Build journal lines: for each adjustment, two lines
    let mut lines = Vec::new();
    let mut line_no = 1;

    for adj in &adjustments {
        let abs_amount = adj.adjustment_minor.unsigned_abs() as i64;

        if adj.adjustment_minor > 0 {
            // Gain: DR account (increase value), CR unrealized FX gain/loss
            lines.push(journal_repo::JournalLineInsert {
                id: Uuid::new_v4(),
                line_no,
                account_ref: adj.account_code.clone(),
                debit_minor: abs_amount,
                credit_minor: 0,
                memo: Some(format!(
                    "FX reval {} {}: gain",
                    adj.currency, adj.account_code
                )),
            });
            line_no += 1;

            lines.push(journal_repo::JournalLineInsert {
                id: Uuid::new_v4(),
                line_no,
                account_ref: UNREALIZED_FX_GAIN_LOSS_ACCOUNT.to_string(),
                debit_minor: 0,
                credit_minor: abs_amount,
                memo: Some(format!(
                    "FX reval {} {}: unrealized gain",
                    adj.currency, adj.account_code
                )),
            });
            line_no += 1;
        } else {
            // Loss: DR unrealized FX gain/loss, CR account (decrease value)
            lines.push(journal_repo::JournalLineInsert {
                id: Uuid::new_v4(),
                line_no,
                account_ref: UNREALIZED_FX_GAIN_LOSS_ACCOUNT.to_string(),
                debit_minor: abs_amount,
                credit_minor: 0,
                memo: Some(format!(
                    "FX reval {} {}: unrealized loss",
                    adj.currency, adj.account_code
                )),
            });
            line_no += 1;

            lines.push(journal_repo::JournalLineInsert {
                id: Uuid::new_v4(),
                line_no,
                account_ref: adj.account_code.clone(),
                debit_minor: 0,
                credit_minor: abs_amount,
                memo: Some(format!(
                    "FX reval {} {}: loss",
                    adj.currency, adj.account_code
                )),
            });
            line_no += 1;
        }
    }

    journal_repo::bulk_insert_lines(tx, entry_id, lines).await?;

    // Step 5: Update account balances for revaluation entries
    for adj in &adjustments {
        let abs_amount = adj.adjustment_minor.unsigned_abs() as i64;

        if adj.adjustment_minor > 0 {
            // DR account
            balance_repo::tx_upsert_rollup(
                tx,
                tenant_id,
                period_id,
                &adj.account_code,
                reporting_currency,
                abs_amount,
                0,
                entry_id,
            )
            .await?;
            // CR gain/loss account
            balance_repo::tx_upsert_rollup(
                tx,
                tenant_id,
                period_id,
                UNREALIZED_FX_GAIN_LOSS_ACCOUNT,
                reporting_currency,
                0,
                abs_amount,
                entry_id,
            )
            .await?;
        } else {
            // DR gain/loss account
            balance_repo::tx_upsert_rollup(
                tx,
                tenant_id,
                period_id,
                UNREALIZED_FX_GAIN_LOSS_ACCOUNT,
                reporting_currency,
                abs_amount,
                0,
                entry_id,
            )
            .await?;
            // CR account
            balance_repo::tx_upsert_rollup(
                tx,
                tenant_id,
                period_id,
                &adj.account_code,
                reporting_currency,
                0,
                abs_amount,
                entry_id,
            )
            .await?;
        }
    }

    // Mark revaluation event as processed (idempotency record)
    sqlx::query(
        r#"
        INSERT INTO processed_events (event_id, event_type, processor)
        VALUES ($1, $2, $3)
        ON CONFLICT (event_id) DO NOTHING
        "#,
    )
    .bind(reval_event_id)
    .bind("gl.revaluation.period_close")
    .bind("fx-revaluation")
    .execute(&mut **tx)
    .await?;

    Ok(RevaluationResult {
        journal_entry_id: Some(entry_id),
        adjustments,
    })
}
