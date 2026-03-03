//! DLQ replay drill for the reporting module.
//!
//! Exercises the checkpoint-reset → re-ingest replay path:
//!   1. Seeds a checkpoint for a test consumer/tenant
//!   2. Resets it via checkpoint API
//!   3. Verifies the checkpoint is gone (consumer would re-process from start)
//!   4. Seeds two tenant checkpoints and does a reset_all
//!   5. Verifies both are gone
//!   6. Verifies snapshot runner idempotency (re-run produces same result)
//!
//! This validates that the operational replay procedure works end-to-end
//! against the real database.

use reporting::domain::jobs::snapshot_runner;
use reporting::ingest::checkpoints;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgres://ap_user:ap_pass@localhost:5443/reporting_test";
const DRILL_CONSUMER: &str = "drill-replay-consumer";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url = std::env::var("REPORTING_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| DEFAULT_DB_URL.to_string());

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let tenant_a = format!("drill-tenant-{}", Uuid::new_v4());
    let tenant_b = format!("drill-tenant-{}", Uuid::new_v4());

    println!("dlq_replay_drill: starting");
    println!("  db={db_url}");
    println!("  tenant_a={tenant_a}");
    println!("  tenant_b={tenant_b}");

    // ── Step 1: Seed checkpoint for tenant_a ───────────────────────────

    checkpoints::save(&pool, DRILL_CONSUMER, &tenant_a, 42, "evt-drill-001").await?;
    let cp = checkpoints::load(&pool, DRILL_CONSUMER, &tenant_a).await?;
    assert!(cp.is_some(), "checkpoint must exist after save");
    println!("  checkpoint_saved=true");

    // ── Step 2: Reset single tenant checkpoint ─────────────────────────

    let deleted = checkpoints::reset(&pool, DRILL_CONSUMER, &tenant_a).await?;
    assert!(deleted > 0, "reset must delete the checkpoint");

    let cp_after = checkpoints::load(&pool, DRILL_CONSUMER, &tenant_a).await?;
    assert!(cp_after.is_none(), "checkpoint must be gone after reset");
    println!("  single_reset=ok deleted={deleted}");

    // ── Step 3: Seed two tenant checkpoints, reset_all ─────────────────

    checkpoints::save(&pool, DRILL_CONSUMER, &tenant_a, 10, "evt-drill-010").await?;
    checkpoints::save(&pool, DRILL_CONSUMER, &tenant_b, 20, "evt-drill-020").await?;

    let deleted_all = checkpoints::reset_all(&pool, DRILL_CONSUMER).await?;
    assert_eq!(deleted_all, 2, "reset_all must delete both checkpoints");

    let cp_a = checkpoints::load(&pool, DRILL_CONSUMER, &tenant_a).await?;
    let cp_b = checkpoints::load(&pool, DRILL_CONSUMER, &tenant_b).await?;
    assert!(cp_a.is_none(), "tenant_a checkpoint must be gone");
    assert!(cp_b.is_none(), "tenant_b checkpoint must be gone");
    println!("  reset_all=ok deleted={deleted_all}");

    // ── Step 4: Verify snapshot runner idempotency ─────────────────────

    let date = chrono::NaiveDate::from_ymd_opt(2026, 6, 1).expect("valid date");
    sqlx::query(
        r#"INSERT INTO rpt_trial_balance_cache
           (tenant_id, as_of, account_code, account_name, currency,
            debit_minor, credit_minor, net_minor, computed_at)
           VALUES ($1, $2, '4000', 'Revenue', 'USD', 0, 100000, -100000, NOW())
           ON CONFLICT (tenant_id, as_of, account_code, currency) DO NOTHING"#,
    )
    .bind(&tenant_a)
    .bind(date)
    .execute(&pool)
    .await?;

    let r1 = snapshot_runner::run_snapshot(&pool, &tenant_a, date, date).await?;
    let r2 = snapshot_runner::run_snapshot(&pool, &tenant_a, date, date).await?;
    assert_eq!(
        r1.rows_upserted, r2.rows_upserted,
        "snapshot must be idempotent"
    );
    println!("  snapshot_idempotent=ok rows={}", r1.rows_upserted);

    // ── Cleanup ────────────────────────────────────────────────────────

    sqlx::query("DELETE FROM rpt_ingestion_checkpoints WHERE consumer_name = $1")
        .bind(DRILL_CONSUMER)
        .execute(&pool)
        .await
        .ok();

    for table in &["rpt_trial_balance_cache", "rpt_statement_cache"] {
        sqlx::query(&format!(
            "DELETE FROM {} WHERE tenant_id = $1 OR tenant_id = $2",
            table
        ))
        .bind(&tenant_a)
        .bind(&tenant_b)
        .execute(&pool)
        .await
        .ok();
    }

    println!("dlq_replay_drill=ok");
    Ok(())
}
