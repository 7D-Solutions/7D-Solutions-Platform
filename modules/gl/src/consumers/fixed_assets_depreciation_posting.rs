//! GL fixed assets depreciation posting logic — journal entry construction
//!
//! Per depreciation schedule period:
//! DR  <expense_account_ref>      amount   ← depreciation expense recognized
//! CR  <accum_depreciation_ref>   amount   ← accumulated depreciation built up

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::contracts::gl_posting_request_v1::{
    Dimensions, GlPostingRequestV1, JournalLine, SourceDocType,
};
use crate::services::journal_service::{process_gl_posting_request, JournalError};

// ============================================================================
// Event payload mirror (anti-corruption layer)
// ============================================================================

/// Per-entry GL data embedded in the run-completed event.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DepreciationGlEntry {
    pub entry_id: Uuid,
    pub asset_id: Uuid,
    pub period_end: NaiveDate,
    pub depreciation_amount_minor: i64,
    pub currency: String,
    pub expense_account_ref: String,
    pub accum_depreciation_ref: String,
}

/// Mirror of fixed_assets::domain::depreciation::models::DepreciationRunCompletedEvent.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct DepreciationRunCompletedPayload {
    pub run_id: Uuid,
    pub tenant_id: String,
    pub periods_posted: i32,
    pub total_depreciation_minor: i64,
    #[serde(default)]
    pub gl_entries: Vec<DepreciationGlEntry>,
}

// ============================================================================
// Posting function (testable without NATS)
// ============================================================================

/// Post a balanced GL depreciation journal entry for one schedule period.
pub async fn process_depreciation_entry_posting(
    pool: &PgPool,
    tenant_id: &str,
    entry: &DepreciationGlEntry,
) -> Result<Uuid, JournalError> {
    let amount = entry.depreciation_amount_minor as f64 / 100.0;
    let posting_date = entry.period_end.to_string();
    let asset_id_str = entry.asset_id.to_string();

    let posting = GlPostingRequestV1 {
        posting_date: posting_date.clone(),
        currency: entry.currency.to_uppercase(),
        source_doc_type: SourceDocType::FixedAssetDepreciation,
        source_doc_id: entry.entry_id.to_string(),
        description: format!(
            "Depreciation — asset {} period ending {}",
            entry.asset_id, entry.period_end,
        ),
        lines: vec![
            JournalLine {
                account_ref: entry.expense_account_ref.clone(),
                debit: amount,
                credit: 0.0,
                memo: Some(format!(
                    "Depreciation expense — asset {} ({})",
                    entry.asset_id,
                    entry.currency.to_uppercase(),
                )),
                dimensions: Some(Dimensions {
                    customer_id: None,
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: Some("fixed_assets".to_string()),
                    project: None,
                }),
            },
            JournalLine {
                account_ref: entry.accum_depreciation_ref.clone(),
                debit: 0.0,
                credit: amount,
                memo: Some(format!(
                    "Accumulated depreciation — asset {} period ending {}",
                    entry.asset_id, entry.period_end,
                )),
                dimensions: Some(Dimensions {
                    customer_id: None,
                    vendor_id: None,
                    location_id: None,
                    job_id: None,
                    department: None,
                    class: Some("fixed_assets".to_string()),
                    project: Some(asset_id_str),
                }),
            },
        ],
    };

    let subject = format!("fa.depreciation.entry.{}", entry.entry_id);

    process_gl_posting_request(
        pool,
        entry.entry_id,
        tenant_id,
        "fixed-assets",
        &subject,
        &posting,
        None,
    )
    .await
}

// ============================================================================
// Integrated tests — require running GL Postgres instance
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serial_test::serial;

    const TEST_TENANT: &str = "test-fa-depr-gl-consumer";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://gl_user:gl_pass@localhost:5438/gl_db".to_string())
    }

    async fn test_pool() -> PgPool {
        PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to GL test DB")
    }

    async fn ensure_open_period(pool: &PgPool) {
        sqlx::query(
            r#"
            INSERT INTO accounting_periods
                (id, tenant_id, period_start, period_end,
                 is_closed, created_at)
            VALUES (gen_random_uuid(), $1, '2026-01-01', '2026-01-31', FALSE, NOW())
            ON CONFLICT DO NOTHING
            "#,
        )
        .bind(TEST_TENANT)
        .execute(pool)
        .await
        .ok();
    }

    async fn ensure_accounts(pool: &PgPool) {
        for (code, name, atype, nb) in [
            ("6100", "Depreciation Expense", "expense", "debit"),
            ("1510", "Accumulated Depreciation", "asset", "credit"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO accounts
                    (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
                VALUES (gen_random_uuid(), $1, $2, $3, $4::account_type, $5::normal_balance, TRUE, NOW())
                ON CONFLICT (tenant_id, code) DO NOTHING
                "#,
            )
            .bind(TEST_TENANT)
            .bind(code)
            .bind(name)
            .bind(atype)
            .bind(nb)
            .execute(pool)
            .await
            .ok();
        }
    }

    async fn cleanup(pool: &PgPool) {
        for q in [
            "DELETE FROM processed_events WHERE event_id IN (SELECT source_event_id FROM journal_entries WHERE tenant_id = $1)",
            "DELETE FROM journal_lines     WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
            "DELETE FROM journal_entries   WHERE tenant_id = $1",
            "DELETE FROM account_balances  WHERE tenant_id = $1",
            "DELETE FROM accounting_periods WHERE tenant_id = $1",
            "DELETE FROM accounts           WHERE tenant_id = $1",
        ] {
            sqlx::query(q).bind(TEST_TENANT).execute(pool).await.ok();
        }
    }

    fn make_entry(amount: i64) -> DepreciationGlEntry {
        DepreciationGlEntry {
            entry_id: Uuid::new_v4(),
            asset_id: Uuid::new_v4(),
            period_end: NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid test date"),
            depreciation_amount_minor: amount,
            currency: "USD".to_string(),
            expense_account_ref: "6100".to_string(),
            accum_depreciation_ref: "1510".to_string(),
        }
    }

    #[tokio::test]
    #[serial]
    async fn posts_balanced_journal_entry() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        ensure_open_period(&pool).await;
        ensure_accounts(&pool).await;

        let entry = make_entry(10_000);

        process_depreciation_entry_posting(&pool, TEST_TENANT, &entry)
            .await
            .expect("posting should succeed");

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("count journal entries");
        assert_eq!(count, 1, "exactly one journal entry created");

        let lines: Vec<(String, f64, f64)> = sqlx::query_as(
            "SELECT account_ref, debit_minor::float8/100.0, credit_minor::float8/100.0 FROM journal_lines \
             WHERE journal_entry_id IN (SELECT id FROM journal_entries WHERE tenant_id = $1)",
        )
        .bind(TEST_TENANT)
        .fetch_all(&pool)
        .await
        .expect("fetch lines");
        assert_eq!(lines.len(), 2, "two lines (DR + CR)");

        let debit_line = lines
            .iter()
            .find(|(a, d, _)| a == "6100" && *d > 0.0)
            .expect("DR line");
        let credit_line = lines
            .iter()
            .find(|(a, _, c)| a == "1510" && *c > 0.0)
            .expect("CR line");
        assert_eq!(debit_line.1, credit_line.2, "balanced: debit == credit");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn idempotent_on_replay() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        ensure_open_period(&pool).await;
        ensure_accounts(&pool).await;

        let entry = make_entry(5_000);

        process_depreciation_entry_posting(&pool, TEST_TENANT, &entry)
            .await
            .expect("first posting");

        let result = process_depreciation_entry_posting(&pool, TEST_TENANT, &entry).await;
        assert!(
            matches!(result, Err(JournalError::DuplicateEvent(_))),
            "replay must return DuplicateEvent, got {:?}",
            result
        );

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count, 1, "no duplicate journal entries on replay");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn multiple_entries_in_run_all_posted() {
        let pool = test_pool().await;
        cleanup(&pool).await;
        ensure_open_period(&pool).await;
        ensure_accounts(&pool).await;

        let entries = vec![make_entry(10_000), make_entry(10_000), make_entry(10_000)];

        for entry in &entries {
            process_depreciation_entry_posting(&pool, TEST_TENANT, entry)
                .await
                .expect("posting should succeed");
        }

        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1")
                .bind(TEST_TENANT)
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(count, 3, "one journal entry per schedule period");

        cleanup(&pool).await;
    }
}
