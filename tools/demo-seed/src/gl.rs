//! GL chart of accounts seeder for demo-seed
//!
//! Creates a standard aerospace-oriented chart of accounts and FX rates
//! via the GL service API.
//!
//! - Account creation: POST /api/gl/accounts — 409 Conflict treated as success
//! - FX rates: POST /api/gl/fx-rates — idempotent via idempotency_key (200 with created=false)

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

use crate::digest::DigestTracker;

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CreateAccountRequest {
    code: String,
    name: String,
    account_type: String,
    normal_balance: String,
}

#[derive(Serialize)]
struct CreateFxRateRequest {
    base_currency: String,
    quote_currency: String,
    rate: f64,
    effective_at: String,
    source: String,
    idempotency_key: String,
}

#[derive(Debug, Deserialize)]
struct FxRateResponse {
    rate_id: Uuid,
    created: bool,
}

// ---------------------------------------------------------------------------
// Static account data
// ---------------------------------------------------------------------------

struct AccountDef {
    code: &'static str,
    name: &'static str,
    account_type: &'static str,
    normal_balance: &'static str,
}

const ACCOUNTS: &[AccountDef] = &[
    AccountDef { code: "1000", name: "Cash", account_type: "Asset", normal_balance: "Debit" },
    AccountDef { code: "1100", name: "Accounts Receivable", account_type: "Asset", normal_balance: "Debit" },
    AccountDef { code: "1200", name: "Raw Materials Inventory", account_type: "Asset", normal_balance: "Debit" },
    AccountDef { code: "1210", name: "WIP Inventory", account_type: "Asset", normal_balance: "Debit" },
    AccountDef { code: "1220", name: "Finished Goods Inventory", account_type: "Asset", normal_balance: "Debit" },
    AccountDef { code: "1300", name: "Fixed Assets", account_type: "Asset", normal_balance: "Debit" },
    AccountDef { code: "2000", name: "Accounts Payable", account_type: "Liability", normal_balance: "Credit" },
    AccountDef { code: "2100", name: "Accrued Expenses", account_type: "Liability", normal_balance: "Credit" },
    AccountDef { code: "3000", name: "Retained Earnings", account_type: "Equity", normal_balance: "Credit" },
    AccountDef { code: "4000", name: "Product Sales Revenue", account_type: "Revenue", normal_balance: "Credit" },
    AccountDef { code: "4100", name: "Service Revenue", account_type: "Revenue", normal_balance: "Credit" },
    AccountDef { code: "5000", name: "COGS - Direct Materials", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "5010", name: "COGS - Direct Labor", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "5020", name: "COGS - Manufacturing Overhead", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "5030", name: "COGS - Scrap and Rework", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "5040", name: "Freight In", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "5100", name: "Purchase Price Variance", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "5120", name: "Inventory Adjustments", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "6000", name: "SGA Expenses", account_type: "Expense", normal_balance: "Debit" },
    AccountDef { code: "6100", name: "R&D Expenses", account_type: "Expense", normal_balance: "Debit" },
];

struct FxRateDef {
    base: &'static str,
    quote: &'static str,
    rate: f64,
}

const FX_RATES: &[FxRateDef] = &[
    FxRateDef { base: "USD", quote: "EUR", rate: 0.92 },
    FxRateDef { base: "USD", quote: "GBP", rate: 0.79 },
];

const FX_EFFECTIVE_AT: &str = "2026-01-01T00:00:00Z";
const FX_SOURCE: &str = "demo-seed";

// ---------------------------------------------------------------------------
// Public return type
// ---------------------------------------------------------------------------

/// Account codes created, for downstream modules to reference
pub struct GlAccounts {
    pub codes: Vec<String>,
    pub accounts: Vec<(String, String)>, // (code, name)
    pub fx_rates: Vec<(uuid::Uuid, String)>, // (rate_id, pair)
}

// ---------------------------------------------------------------------------
// Seeding logic
// ---------------------------------------------------------------------------

/// Seed GL chart of accounts and FX rates. Returns account codes for downstream use.
pub async fn seed_gl(
    client: &reqwest::Client,
    gl_url: &str,
    tenant: &str,
    seed: u64,
    tracker: &mut DigestTracker,
) -> Result<GlAccounts> {
    let mut codes = Vec::with_capacity(ACCOUNTS.len());
    let mut accounts = Vec::with_capacity(ACCOUNTS.len());
    let mut fx_rates_out = Vec::with_capacity(FX_RATES.len());

    // Create accounts
    for acct in ACCOUNTS {
        create_account(client, gl_url, acct).await?;
        tracker.record_gl_account(acct.code, acct.name);
        codes.push(acct.code.to_string());
        accounts.push((acct.code.to_string(), acct.name.to_string()));
        info!(code = acct.code, name = acct.name, "GL account seeded");
    }

    // Create FX rates
    for fx in FX_RATES {
        let idempotency_key = format!("{}-fx-{}-{}-{}", tenant, fx.base, fx.quote, seed);
        let rate_id = create_fx_rate(client, gl_url, fx, &idempotency_key).await?;
        let pair = format!("{}/{}", fx.base, fx.quote);
        tracker.record_fx_rate(rate_id, &pair);
        fx_rates_out.push((rate_id, pair.clone()));
        info!(pair, rate = fx.rate, "FX rate seeded");
    }

    Ok(GlAccounts { codes, accounts, fx_rates: fx_rates_out })
}

async fn create_account(
    client: &reqwest::Client,
    gl_url: &str,
    acct: &AccountDef,
) -> Result<()> {
    let url = format!("{}/api/gl/accounts", gl_url);

    let body = CreateAccountRequest {
        code: acct.code.to_string(),
        name: acct.name.to_string(),
        account_type: acct.account_type.to_string(),
        normal_balance: acct.normal_balance.to_string(),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST /api/gl/accounts ({}) network error", acct.code))?;

    let status = resp.status();

    // 201 Created = new account, 409 Conflict = already exists (both are success)
    if status == reqwest::StatusCode::CREATED || status == reqwest::StatusCode::CONFLICT {
        return Ok(());
    }

    let text = resp.text().await.unwrap_or_default();
    bail!(
        "POST /api/gl/accounts ({}) failed {status}: {text}",
        acct.code
    );
}

async fn create_fx_rate(
    client: &reqwest::Client,
    gl_url: &str,
    fx: &FxRateDef,
    idempotency_key: &str,
) -> Result<Uuid> {
    let url = format!("{}/api/gl/fx-rates", gl_url);

    let body = CreateFxRateRequest {
        base_currency: fx.base.to_string(),
        quote_currency: fx.quote.to_string(),
        rate: fx.rate,
        effective_at: FX_EFFECTIVE_AT.to_string(),
        source: FX_SOURCE.to_string(),
        idempotency_key: idempotency_key.to_string(),
    };

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .with_context(|| {
            format!(
                "POST /api/gl/fx-rates ({}/{}) network error",
                fx.base, fx.quote
            )
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!(
            "POST /api/gl/fx-rates ({}/{}) failed {status}: {text}",
            fx.base,
            fx.quote
        );
    }

    let fx_resp: FxRateResponse = resp
        .json()
        .await
        .context("Failed to parse FX rate response")?;

    if fx_resp.created {
        info!(
            pair = format!("{}/{}", fx.base, fx.quote),
            rate_id = %fx_resp.rate_id,
            "Created new FX rate"
        );
    } else {
        info!(
            pair = format!("{}/{}", fx.base, fx.quote),
            rate_id = %fx_resp.rate_id,
            "FX rate already existed"
        );
    }

    Ok(fx_resp.rate_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_accounts_defined() {
        assert_eq!(ACCOUNTS.len(), 20, "Expected 20 GL accounts");
    }

    #[test]
    fn account_codes_are_unique() {
        let mut codes: Vec<&str> = ACCOUNTS.iter().map(|a| a.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), ACCOUNTS.len(), "Duplicate account codes found");
    }

    #[test]
    fn inventory_critical_accounts_present() {
        let required = ["1200", "1210", "1220", "5000", "5040", "5100", "5120"];
        for code in &required {
            assert!(
                ACCOUNTS.iter().any(|a| a.code == *code),
                "Missing critical inventory account: {code}"
            );
        }
    }

    #[test]
    fn account_types_are_valid() {
        let valid_types = ["Asset", "Liability", "Equity", "Revenue", "Expense"];
        for acct in ACCOUNTS {
            assert!(
                valid_types.contains(&acct.account_type),
                "Account {} has invalid type: {}",
                acct.code,
                acct.account_type
            );
        }
    }

    #[test]
    fn normal_balances_are_valid() {
        let valid = ["Debit", "Credit"];
        for acct in ACCOUNTS {
            assert!(
                valid.contains(&acct.normal_balance),
                "Account {} has invalid normal_balance: {}",
                acct.code,
                acct.normal_balance
            );
        }
    }

    #[test]
    fn normal_balance_follows_accounting_standards() {
        for acct in ACCOUNTS {
            let expected = match acct.account_type {
                "Asset" | "Expense" => "Debit",
                "Liability" | "Equity" | "Revenue" => "Credit",
                _ => panic!("Unknown account type: {}", acct.account_type),
            };
            assert_eq!(
                acct.normal_balance, expected,
                "Account {} ({}) should have normal_balance={} but has {}",
                acct.code, acct.account_type, expected, acct.normal_balance
            );
        }
    }

    #[test]
    fn two_fx_rates_defined() {
        assert_eq!(FX_RATES.len(), 2, "Expected 2 FX rates");
    }

    #[test]
    fn fx_rates_are_positive() {
        for fx in FX_RATES {
            assert!(
                fx.rate > 0.0,
                "FX rate {}/{} must be positive",
                fx.base,
                fx.quote
            );
        }
    }

    #[test]
    fn fx_idempotency_key_format() {
        let key = format!("{}-fx-{}-{}-{}", "t1", "USD", "EUR", 42);
        assert_eq!(key, "t1-fx-USD-EUR-42");
    }

    #[test]
    fn fx_effective_at_is_deterministic() {
        assert_eq!(FX_EFFECTIVE_AT, "2026-01-01T00:00:00Z");
    }

    #[test]
    fn fx_pairs_are_unique() {
        let mut pairs: Vec<String> = FX_RATES
            .iter()
            .map(|fx| format!("{}/{}", fx.base, fx.quote))
            .collect();
        pairs.sort();
        pairs.dedup();
        assert_eq!(pairs.len(), FX_RATES.len(), "Duplicate FX pairs found");
    }
}
