//! Consolidated Balance Sheet from `csl_trial_balance_cache`.
//!
//! Account classification by code prefix:
//!   1xxx → Assets      (debit-normal:  amount = debit_minor − credit_minor)
//!   2xxx → Liabilities (credit-normal: amount = credit_minor − debit_minor)
//!   3xxx → Equity      (credit-normal: amount = credit_minor − debit_minor)
//!
//! Scoped by (group_id, as_of) — one consolidation snapshot per period.

use std::collections::HashMap;

use chrono::NaiveDate;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct BsAccountLine {
    pub account_code: String,
    pub account_name: String,
    pub currency: String,
    pub amount_minor: i64,
}

#[derive(Debug, Serialize)]
pub struct BsSection {
    pub section: String,
    pub accounts: Vec<BsAccountLine>,
    pub total_by_currency: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct ConsolidatedBalanceSheet {
    pub group_id: Uuid,
    pub as_of: NaiveDate,
    pub sections: Vec<BsSection>,
}

// ── Computation ───────────────────────────────────────────────────────────────

/// Compute a consolidated Balance Sheet from the cached consolidated TB.
pub async fn compute_consolidated_bs(
    pool: &PgPool,
    group_id: Uuid,
    as_of: NaiveDate,
) -> Result<ConsolidatedBalanceSheet, sqlx::Error> {
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

    let mut assets: Vec<BsAccountLine> = Vec::new();
    let mut liabilities: Vec<BsAccountLine> = Vec::new();
    let mut equity: Vec<BsAccountLine> = Vec::new();

    for (account_code, account_name, currency, debit_minor, credit_minor) in rows {
        let prefix = account_prefix(&account_code);
        match prefix {
            1 => assets.push(BsAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: debit_minor - credit_minor,
            }),
            2 => liabilities.push(BsAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: credit_minor - debit_minor,
            }),
            3 => equity.push(BsAccountLine {
                account_code,
                account_name,
                currency,
                amount_minor: credit_minor - debit_minor,
            }),
            _ => {} // P&L accounts — not part of balance sheet
        }
    }

    let sections = vec![
        BsSection {
            section: "assets".into(),
            total_by_currency: sum_by_currency(&assets),
            accounts: assets,
        },
        BsSection {
            section: "liabilities".into(),
            total_by_currency: sum_by_currency(&liabilities),
            accounts: liabilities,
        },
        BsSection {
            section: "equity".into(),
            total_by_currency: sum_by_currency(&equity),
            accounts: equity,
        },
    ];

    Ok(ConsolidatedBalanceSheet {
        group_id,
        as_of,
        sections,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn account_prefix(code: &str) -> u32 {
    code.chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .unwrap_or(0)
}

fn sum_by_currency(lines: &[BsAccountLine]) -> HashMap<String, i64> {
    let mut m: HashMap<String, i64> = HashMap::new();
    for l in lines {
        *m.entry(l.currency.clone()).or_insert(0) += l.amount_minor;
    }
    m
}
