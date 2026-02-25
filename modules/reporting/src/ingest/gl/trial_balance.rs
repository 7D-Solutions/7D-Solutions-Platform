//! Trial balance cache builder from GL posting events.
//!
//! Subscribes to `gl.events.posting.requested` and accumulates debit/credit
//! totals into `rpt_trial_balance_cache` keyed by
//! `(tenant_id, as_of, account_code, currency)`.
//!
//! ## Idempotency
//!
//! Two layers protect against duplicate cache rows:
//! 1. **Framework layer** (`IngestConsumer`): skips events whose `event_id` matches
//!    the last recorded checkpoint — covers normal re-delivery scenarios.
//! 2. **Handler layer** (`ON CONFLICT DO UPDATE`): safe for concurrent writers;
//!    accumulates deltas rather than replacing rows so multiple events on the
//!    same date/account aggregate correctly.
//!
//! ## Reconciliation invariant
//!
//! For any balanced GL posting the sum of debit lines equals the sum of credit
//! lines. After ingestion, querying `SUM(debit_minor)` and `SUM(credit_minor)`
//! for a tenant/currency over the same set of posted events must be equal.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Deserialize;
use sqlx::PgPool;

use crate::ingest::StreamHandler;

// ── Local payload mirror ──────────────────────────────────────────────────────
//
// Reporting must not depend on the gl-rs crate. We mirror just the fields
// we need from GlPostingRequestV1.

#[derive(Debug, Deserialize)]
struct GlLine {
    account_ref: String,
    debit: f64,
    credit: f64,
}

#[derive(Debug, Deserialize)]
struct GlPostingPayload {
    posting_date: String,
    currency: String,
    lines: Vec<GlLine>,
}

// ── Handler ───────────────────────────────────────────────────────────────────

/// Builds the trial balance cache from GL posting events.
pub struct TrialBalanceHandler;

#[async_trait]
impl StreamHandler for TrialBalanceHandler {
    async fn handle(
        &self,
        pool: &PgPool,
        tenant_id: &str,
        _event_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), anyhow::Error> {
        let posting = GlPostingPayload::deserialize(payload)
            .map_err(|e| anyhow::anyhow!("Failed to parse GL posting payload: {}", e))?;

        let as_of = NaiveDate::parse_from_str(&posting.posting_date, "%Y-%m-%d")
            .map_err(|e| anyhow::anyhow!("Invalid posting_date '{}': {}", posting.posting_date, e))?;

        for line in &posting.lines {
            // Convert major units (e.g. USD dollars) → minor units (cents)
            let debit_minor = (line.debit * 100.0).round() as i64;
            let credit_minor = (line.credit * 100.0).round() as i64;
            let net_minor = debit_minor - credit_minor;

            // account_name is not in the posting request; use account_ref as placeholder.
            // Downstream statements beads can enrich via a COA join if needed.
            let account_name = &line.account_ref;

            sqlx::query(
                r#"
                INSERT INTO rpt_trial_balance_cache
                    (tenant_id, as_of, account_code, account_name, currency,
                     debit_minor, credit_minor, net_minor, computed_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
                ON CONFLICT (tenant_id, as_of, account_code, currency) DO UPDATE SET
                    debit_minor  = rpt_trial_balance_cache.debit_minor  + EXCLUDED.debit_minor,
                    credit_minor = rpt_trial_balance_cache.credit_minor + EXCLUDED.credit_minor,
                    net_minor    = rpt_trial_balance_cache.net_minor    + EXCLUDED.net_minor,
                    computed_at  = NOW()
                "#,
            )
            .bind(tenant_id)
            .bind(as_of)
            .bind(&line.account_ref)
            .bind(account_name)
            .bind(&posting.currency)
            .bind(debit_minor)
            .bind(credit_minor)
            .bind(net_minor)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to upsert trial balance cache: {}", e))?;
        }

        Ok(())
    }
}

// ── Integrated tests (real DB + InMemoryBus, no mocks) ───────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::{start_consumer, IngestConsumer};
    use event_bus::BusMessage;
    use serial_test::serial;
    use std::sync::Arc;

    const TENANT: &str = "test-tb-tenant";

    fn test_db_url() -> String {
        std::env::var("REPORTING_DATABASE_URL").unwrap_or_else(|_| {
            "postgres://ap_user:ap_pass@localhost:5443/reporting_test".to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to reporting test DB");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Failed to run reporting migrations");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM rpt_trial_balance_cache WHERE tenant_id = $1",
        )
        .bind(TENANT)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM rpt_ingestion_checkpoints WHERE tenant_id = $1",
        )
        .bind(TENANT)
        .execute(pool)
        .await
        .ok();
    }

    fn make_posting_envelope(
        event_id: &str,
        posting_date: &str,
        currency: &str,
        lines: &[(&str, f64, f64)],
    ) -> Vec<u8> {
        let line_values: Vec<serde_json::Value> = lines
            .iter()
            .map(|(acct, dr, cr)| {
                serde_json::json!({
                    "account_ref": acct,
                    "debit": dr,
                    "credit": cr
                })
            })
            .collect();

        serde_json::to_vec(&serde_json::json!({
            "event_id": event_id,
            "tenant_id": TENANT,
            "data": {
                "posting_date": posting_date,
                "currency": currency,
                "source_doc_type": "AR_INVOICE",
                "source_doc_id": "inv-001",
                "description": "Test posting",
                "lines": line_values
            }
        }))
        .unwrap()
    }

    /// Fetch trial balance rows for the test tenant on a given date.
    async fn fetch_cache(
        pool: &PgPool,
        as_of: &str,
        currency: &str,
    ) -> Vec<(String, i64, i64, i64)> {
        let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(
            r#"
            SELECT account_code, debit_minor, credit_minor, net_minor
            FROM rpt_trial_balance_cache
            WHERE tenant_id = $1
              AND as_of = $2::date
              AND currency = $3
            ORDER BY account_code
            "#,
        )
        .bind(TENANT)
        .bind(as_of)
        .bind(currency)
        .fetch_all(pool)
        .await
        .expect("fetch cache failed");
        rows
    }

    // ── Test 1: handler creates cache entries from a simple posting ──────────

    #[tokio::test]
    #[serial]
    async fn test_handle_creates_trial_balance_entries() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(TrialBalanceHandler);
        let consumer = IngestConsumer::new("test-tb-basic", pool.clone(), handler);

        let msg = BusMessage::new(
            "gl.events.posting.requested".to_string(),
            make_posting_envelope(
                "evt-tb-001",
                "2026-01-31",
                "USD",
                &[
                    ("1100", 2599.00, 0.0), // AR debit
                    ("4000", 0.0, 2599.00), // Revenue credit
                ],
            ),
        );

        let processed = consumer.process_message(&msg).await.expect("process failed");
        assert!(processed, "first delivery must be processed");

        let rows = fetch_cache(&pool, "2026-01-31", "USD").await;
        assert_eq!(rows.len(), 2, "two account rows must be created");

        let ar = rows.iter().find(|(acct, ..)| acct == "1100").expect("AR row");
        assert_eq!(ar.1, 259900, "AR debit_minor = 2599.00 * 100");
        assert_eq!(ar.2, 0, "AR credit_minor = 0");
        assert_eq!(ar.3, 259900, "AR net_minor = debit - credit");

        let rev = rows.iter().find(|(acct, ..)| acct == "4000").expect("Revenue row");
        assert_eq!(rev.1, 0, "Revenue debit_minor = 0");
        assert_eq!(rev.2, 259900, "Revenue credit_minor = 2599.00 * 100");
        assert_eq!(rev.3, -259900, "Revenue net_minor = 0 - 259900");

        cleanup(&pool).await;
    }

    // ── Test 2: amounts accumulate across multiple postings ──────────────────

    #[tokio::test]
    #[serial]
    async fn test_handle_accumulates_multiple_postings() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(TrialBalanceHandler);
        let consumer_a = IngestConsumer::new("test-tb-accum-a", pool.clone(), handler.clone());
        let consumer_b = IngestConsumer::new("test-tb-accum-b", pool.clone(), handler);

        // First posting: AR 1000.00 / Revenue 1000.00
        let msg_a = BusMessage::new(
            "gl.events.posting.requested".to_string(),
            make_posting_envelope(
                "evt-tb-accum-001",
                "2026-02-01",
                "USD",
                &[("1100", 1000.00, 0.0), ("4000", 0.0, 1000.00)],
            ),
        );
        consumer_a.process_message(&msg_a).await.expect("msg_a failed");

        // Second posting (different event): AR 500.00 / Revenue 500.00 same date
        let msg_b = BusMessage::new(
            "gl.events.posting.requested".to_string(),
            make_posting_envelope(
                "evt-tb-accum-002",
                "2026-02-01",
                "USD",
                &[("1100", 500.00, 0.0), ("4000", 0.0, 500.00)],
            ),
        );
        consumer_b.process_message(&msg_b).await.expect("msg_b failed");

        let rows = fetch_cache(&pool, "2026-02-01", "USD").await;
        assert_eq!(rows.len(), 2);

        let ar = rows.iter().find(|(acct, ..)| acct == "1100").expect("AR row");
        assert_eq!(ar.1, 150000, "AR debit: 1000.00 + 500.00 = 1500.00 = 150000 minor");

        let rev = rows.iter().find(|(acct, ..)| acct == "4000").expect("Revenue row");
        assert_eq!(rev.2, 150000, "Revenue credit: 1000.00 + 500.00 = 150000 minor");

        cleanup(&pool).await;
    }

    // ── Test 3: idempotent on re-delivery (checkpoint gate) ─────────────────

    #[tokio::test]
    #[serial]
    async fn test_idempotent_on_redelivery_via_checkpoint() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(TrialBalanceHandler);
        let consumer = IngestConsumer::new("test-tb-idem", pool.clone(), handler);

        let msg = BusMessage::new(
            "gl.events.posting.requested".to_string(),
            make_posting_envelope(
                "evt-tb-idem-001",
                "2026-02-05",
                "USD",
                &[("1100", 800.00, 0.0), ("4000", 0.0, 800.00)],
            ),
        );

        // First delivery
        let first = consumer.process_message(&msg).await.expect("first failed");
        assert!(first, "first delivery processed");

        // Re-delivery of same event_id — framework gate must skip it
        let second = consumer.process_message(&msg).await.expect("second failed");
        assert!(!second, "re-delivery must be skipped by checkpoint gate");

        let rows = fetch_cache(&pool, "2026-02-05", "USD").await;
        let ar = rows.iter().find(|(acct, ..)| acct == "1100").expect("AR row");
        assert_eq!(ar.1, 80000, "debit must not be doubled by re-delivery");

        cleanup(&pool).await;
    }

    // ── Test 4: trial balance reconciles (debits == credits per currency) ────

    #[tokio::test]
    #[serial]
    async fn test_trial_balance_reconciles_debits_equal_credits() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(TrialBalanceHandler);

        // Post three balanced transactions on the same date
        let postings = vec![
            ("evt-tb-rec-001", "2026-02-10", "USD", vec![
                ("1100", 3000.00, 0.0),
                ("4000", 0.0, 3000.00),
            ]),
            ("evt-tb-rec-002", "2026-02-10", "USD", vec![
                ("1000", 500.00, 0.0),
                ("2000", 0.0, 500.00),
            ]),
            ("evt-tb-rec-003", "2026-02-10", "USD", vec![
                ("5000", 250.00, 0.0),
                ("1000", 0.0, 250.00),
            ]),
        ];

        for (i, (event_id, date, currency, lines)) in postings.iter().enumerate() {
            let consumer = IngestConsumer::new(
                format!("test-tb-rec-{}", i),
                pool.clone(),
                handler.clone(),
            );
            let line_refs: Vec<(&str, f64, f64)> =
                lines.iter().map(|(a, d, c)| (*a, *d, *c)).collect();
            let msg = BusMessage::new(
                "gl.events.posting.requested".to_string(),
                make_posting_envelope(event_id, date, currency, &line_refs),
            );
            consumer.process_message(&msg).await.expect("posting failed");
        }

        // Reconciliation: SUM(debit_minor) must equal SUM(credit_minor)
        let (total_debit, total_credit): (i64, i64) = sqlx::query_as(
            r#"
            SELECT COALESCE(SUM(debit_minor), 0)::BIGINT,
                   COALESCE(SUM(credit_minor), 0)::BIGINT
            FROM rpt_trial_balance_cache
            WHERE tenant_id = $1
              AND as_of = '2026-02-10'
              AND currency = 'USD'
            "#,
        )
        .bind(TENANT)
        .fetch_one(&pool)
        .await
        .expect("reconciliation query failed");

        assert_eq!(
            total_debit, total_credit,
            "trial balance must reconcile: total debits ({}) == total credits ({})",
            total_debit, total_credit
        );
        assert!(total_debit > 0, "must have actual postings in cache");

        cleanup(&pool).await;
    }

    // ── Test 5: multi-currency postings stay isolated ────────────────────────

    #[tokio::test]
    #[serial]
    async fn test_multi_currency_postings_isolated() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let handler = Arc::new(TrialBalanceHandler);

        let currencies = [("evt-tb-cur-usd", "USD"), ("evt-tb-cur-eur", "EUR")];

        for (i, (event_id, currency)) in currencies.iter().enumerate() {
            let consumer = IngestConsumer::new(
                format!("test-tb-cur-{}", i),
                pool.clone(),
                handler.clone(),
            );
            let msg = BusMessage::new(
                "gl.events.posting.requested".to_string(),
                make_posting_envelope(
                    event_id,
                    "2026-02-15",
                    currency,
                    &[("1100", 100.00, 0.0), ("4000", 0.0, 100.00)],
                ),
            );
            consumer.process_message(&msg).await.expect("posting failed");
        }

        // Each currency should have its own rows
        let usd_rows = fetch_cache(&pool, "2026-02-15", "USD").await;
        let eur_rows = fetch_cache(&pool, "2026-02-15", "EUR").await;

        assert_eq!(usd_rows.len(), 2, "USD must have 2 account rows");
        assert_eq!(eur_rows.len(), 2, "EUR must have 2 account rows");

        // Amounts must not bleed across currencies
        let usd_ar = usd_rows.iter().find(|(a, ..)| a == "1100").unwrap();
        let eur_ar = eur_rows.iter().find(|(a, ..)| a == "1100").unwrap();
        assert_eq!(usd_ar.1, 10000, "USD AR = 100.00 * 100 = 10000");
        assert_eq!(eur_ar.1, 10000, "EUR AR = 100.00 * 100 = 10000");

        cleanup(&pool).await;
    }
}
