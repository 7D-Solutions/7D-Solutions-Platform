//! Consolidated Profit & Loss statement from `csl_trial_balance_cache`.
//!
//! Account classification by code prefix:
//!   4xxx → Revenue  (credit-normal: amount = credit_minor − debit_minor)
//!   5xxx → COGS     (debit-normal:  amount = debit_minor − credit_minor)
//!   6xxx → Expenses (debit-normal:  amount = debit_minor − credit_minor)
//!
//! Scoped by (group_id, as_of) — one consolidation snapshot per period.

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct PlAccountLine {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    pub amount_minor: i64,
}

#[derive(Debug, Serialize)]
pub struct PlSection {
    pub section: String,
    pub accounts: Vec<PlAccountLine>,
    pub total_by_currency: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct ConsolidatedPl {
    pub group_id: Uuid,
    pub as_of: NaiveDate,
    pub sections: Vec<PlSection>,
    pub net_income_by_currency: HashMap<String, i64>,
}

// ── Computation ───────────────────────────────────────────────────────────────

/// Compute a consolidated P&L from the cached consolidated trial balance.
pub async fn compute_consolidated_pl(
    pool: &PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
) -> Result<ConsolidatedPl, sqlx::Error> {
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_code,
               account_name,
               currency,
               debit_minor,
               credit_minor
        FROM csl_trial_balance_cache
        WHERE group_id = $1
          AND as_of = $2
        ORDER BY account_code, currency
        "#,
    )
    .bind(group_id)
    .bind(as_of)
    .fetch_all(pool)
    .await?;

    let mut revenue: Vec<PlAccountLine> = Vec::new();
    let mut cogs: Vec<PlAccountLine> = Vec::new();
    let mut expenses: Vec<PlAccountLine> = Vec::new();

    for (account_code, account_name, currency, debit_minor, credit_minor) in rows {
        let prefix = account_prefix(&account_code);
        match prefix {
            4 => revenue.push(PlAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: credit_minor - debit_minor,
            }),
            5 => cogs.push(PlAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: debit_minor - credit_minor,
            }),
            6 => expenses.push(PlAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: debit_minor - credit_minor,
            }),
            _ => {} // balance-sheet accounts — not part of P&L
        }
    }

    let rev_totals = sum_by_currency(&revenue);
    let cogs_totals = sum_by_currency(&cogs);
    let exp_totals = sum_by_currency(&expenses);

    let mut net: HashMap<String, i64> = HashMap::new();
    for (cur, &rev) in &rev_totals {
        *net.entry(cur.clone()).or_insert(0) += rev;
    }
    for (cur, &cost) in &cogs_totals {
        *net.entry(cur.clone()).or_insert(0) -= cost;
    }
    for (cur, &exp) in &exp_totals {
        *net.entry(cur.clone()).or_insert(0) -= exp;
    }

    let sections = vec![
        PlSection {
            section: "revenue".into(),
            total_by_currency: rev_totals,
            accounts: revenue,
        },
        PlSection {
            section: "cogs".into(),
            total_by_currency: cogs_totals,
            accounts: cogs,
        },
        PlSection {
            section: "expenses".into(),
            total_by_currency: exp_totals,
            accounts: expenses,
        },
    ];

    Ok(ConsolidatedPl {
        group_id,
        as_of,
        sections,
        net_income_by_currency: net,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn account_prefix(code: &str) -> u32 {
    code.chars().next().and_then(|c| c.to_digit(10)).unwrap_or(0)
}

fn sum_by_currency(lines: &[PlAccountLine]) -> HashMap<String, i64> {
    let mut m: HashMap<String, i64> = HashMap::new();
    for l in lines {
        *m.entry(l.currency.clone()).or_insert(0) += l.amount_minor;
    }
    m
}
