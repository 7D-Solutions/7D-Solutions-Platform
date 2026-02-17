//! AR reconciliation batch throughput benchmark (bd-1kmq, Wave 1).
//!
//! Seeds controlled charge/invoice pairs per tenant and measures
//! run_reconciliation() throughput and invariant correctness.
//!
//! Design:
//! - One customer per tenant; N charges + N matching invoices (unique amounts → 1:1 exact match).
//! - All tenant reconciliation runs execute concurrently (semaphore capped at `concurrency`).
//! - Invariant: no payment_id appears in ar_recon_matches more than once per tenant.
//!
//! Env thresholds:
//!   RECON_MIN_MATCHES_PER_SEC  (default 50)
//!   RECON_MAX_EXCEPTION_RATE   (default 0.05 = 5%)

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ar_rs::reconciliation::{run_reconciliation, RunReconOutcome, RunReconRequest};
use sqlx::PgPool;
use tokio::task::JoinSet;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::metrics::{MetricsSamples, Timer};
use crate::report::ScenarioResult;

// ── Thresholds ────────────────────────────────────────────────────────────────

const DEFAULT_RECON_MIN_MATCHES_PER_SEC: f64 = 50.0;
const DEFAULT_RECON_MAX_EXCEPTION_RATE: f64 = 0.05;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    if dry_run {
        return run_dry(cfg).await;
    }

    let concurrency = cfg.concurrency;
    let tenant_count = cfg.tenant_count;
    let rows_per_tenant = (cfg.recon_rows / tenant_count.max(1)).max(1);

    let max_conn = (concurrency + 4).min(100) as u32;
    let pool = pg_pool(&cfg.database_url, max_conn)
        .await
        .context("recon: DB connect")?;

    let run_prefix = Uuid::new_v4().simple().to_string()[..8].to_string();

    info!(
        "recon: seeding {} tenants × {} rows (run_prefix={})",
        tenant_count, rows_per_tenant, run_prefix
    );

    // Phase 1: Seed charges + invoices
    let seeds = seed_tenants(&pool, &run_prefix, tenant_count, rows_per_tenant)
        .await
        .context("recon: seed_tenants")?;

    info!(
        "recon: running reconciliation (concurrency={})…",
        concurrency
    );

    // Phase 2: Concurrent reconciliation runs — one per tenant
    let wall = Timer::start();
    let mut samples = MetricsSamples::new();
    let mut total_matches: u64 = 0;
    let mut total_exceptions: u64 = 0;

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut join_set: JoinSet<_> = JoinSet::new();

    for app_id in seeds.iter().map(|(a, _)| a.clone()) {
        let pool2 = pool.clone();
        let sem2 = sem.clone();
        join_set.spawn(async move {
            let _permit = sem2.acquire_owned().await.unwrap();
            let t = Timer::start();
            let recon_run_id = Uuid::new_v4();
            let result = run_reconciliation(
                &pool2,
                RunReconRequest {
                    recon_run_id,
                    app_id,
                    correlation_id: format!("bench-recon-{}", recon_run_id.simple()),
                    causation_id: None,
                },
            )
            .await;
            (result, t.elapsed())
        });
    }

    while let Some(outcome) = join_set.join_next().await {
        match outcome {
            Ok((Ok(RunReconOutcome::Executed(r)), lat)) => {
                total_matches += r.match_count as u64;
                total_exceptions += r.exception_count as u64;
                samples.record_latency(lat);
            }
            Ok((Ok(RunReconOutcome::AlreadyExists(r)), lat)) => {
                total_matches += r.match_count as u64;
                total_exceptions += r.exception_count as u64;
                samples.record_latency(lat);
            }
            Ok((Err(e), _)) => {
                tracing::warn!("recon run error: {}", e);
                samples.record_error();
            }
            Err(e) => {
                tracing::warn!("recon task join error: {}", e);
                samples.record_error();
            }
        }
    }

    samples.set_wall_clock(wall.elapsed());
    let wall_secs = wall.elapsed().as_secs_f64();

    // Phase 3: Invariant — no payment_id matched more than once per tenant
    let (dupe_count, invariant_ok) = check_match_uniqueness(&pool, &run_prefix)
        .await
        .context("recon: invariant check")?;

    let total_rows = seeds.iter().map(|(_, n)| n).sum::<usize>() as u64;
    let exception_rate = if total_rows > 0 {
        total_exceptions as f64 / total_rows as f64
    } else {
        0.0
    };
    let matches_per_sec = total_matches as f64 / wall_secs.max(0.001);

    info!(
        "recon: done — matches={} exceptions={} exception_rate={:.1}% matches/s={:.1} invariant={}",
        total_matches,
        total_exceptions,
        exception_rate * 100.0,
        matches_per_sec,
        if invariant_ok { "PASS" } else { "FAIL" }
    );

    // Phase 4: Threshold enforcement
    let min_throughput =
        read_env_f64("RECON_MIN_MATCHES_PER_SEC", DEFAULT_RECON_MIN_MATCHES_PER_SEC);
    let max_exception_rate =
        read_env_f64("RECON_MAX_EXCEPTION_RATE", DEFAULT_RECON_MAX_EXCEPTION_RATE);

    let mut violations: Vec<String> = Vec::new();

    if !invariant_ok {
        violations.push(format!(
            "invariant: {} duplicate match entries (payment matched >1 time) — FAIL",
            dupe_count
        ));
    }
    if total_matches > 0 && matches_per_sec < min_throughput {
        violations.push(format!(
            "throughput {:.1} matches/s < threshold {:.1} matches/s",
            matches_per_sec, min_throughput
        ));
    }
    if exception_rate > max_exception_rate {
        violations.push(format!(
            "exception rate {:.1}% > threshold {:.1}%",
            exception_rate * 100.0,
            max_exception_rate * 100.0
        ));
    }

    Ok(ScenarioResult {
        name: "recon".to_string(),
        passed: violations.is_empty(),
        metrics: serde_json::json!({
            "tenant_count": tenant_count,
            "recon_rows": total_rows,
            "rows_per_tenant": rows_per_tenant,
            "concurrency": concurrency,
            "total_matches": total_matches,
            "total_exceptions": total_exceptions,
            "exception_rate": exception_rate,
            "matches_per_sec": matches_per_sec,
            "wall_secs": wall_secs,
            "invariant_duplicate_matches": dupe_count,
            "invariant_passed": invariant_ok,
            "p50_run_ms": samples.p50(),
            "p95_run_ms": samples.p95(),
            "p99_run_ms": samples.p99(),
            "total_ops": samples.total_ops,
            "errors": samples.errors,
        }),
        threshold_violations: violations,
        notes: None,
    })
}

// ── Seeding ───────────────────────────────────────────────────────────────────

/// Seed one customer + `rows_per_tenant` charges and invoices per tenant.
///
/// Each row gets a unique `amount_cents = 10_000 + j*100` within its tenant,
/// guaranteeing unambiguous 1:1 exact matches for the reconciliation engine.
///
/// Returns Vec<(app_id, row_count)>.
async fn seed_tenants(
    pool: &PgPool,
    run_prefix: &str,
    tenant_count: usize,
    rows_per_tenant: usize,
) -> Result<Vec<(String, usize)>> {
    let mut seeds = Vec::with_capacity(tenant_count);

    for i in 0..tenant_count {
        let app_id = format!("bench-rc-{}-{:04}", run_prefix, i);

        let customer_id: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO ar_customers (app_id, external_customer_id, email, status)
            VALUES ($1, $2, $3, 'active')
            RETURNING id
            "#,
        )
        .bind(&app_id)
        .bind(format!("bench-cust-{}-{:04}", run_prefix, i))
        .bind(format!("bench-rc-{}-{}@example.com", run_prefix, i))
        .fetch_one(pool)
        .await
        .with_context(|| format!("seed customer for tenant {}", app_id))?;

        for j in 0..rows_per_tenant {
            let amount_cents = 10_000 + (j as i32) * 100;
            let tilled_invoice_id =
                format!("bench-rc-inv-{}-{:04}-{:06}", run_prefix, i, j);

            sqlx::query(
                r#"
                INSERT INTO ar_invoices (
                    app_id, tilled_invoice_id, ar_customer_id,
                    status, amount_cents, currency, updated_at
                ) VALUES ($1, $2, $3, 'open', $4, 'usd', NOW())
                "#,
            )
            .bind(&app_id)
            .bind(&tilled_invoice_id)
            .bind(customer_id)
            .bind(amount_cents)
            .execute(pool)
            .await
            .with_context(|| format!("seed invoice {} for {}", j, app_id))?;

            sqlx::query(
                r#"
                INSERT INTO ar_charges (
                    app_id, ar_customer_id,
                    status, amount_cents, currency, updated_at
                ) VALUES ($1, $2, 'succeeded', $3, 'usd', NOW())
                "#,
            )
            .bind(&app_id)
            .bind(customer_id)
            .bind(amount_cents)
            .execute(pool)
            .await
            .with_context(|| format!("seed charge {} for {}", j, app_id))?;
        }

        seeds.push((app_id, rows_per_tenant));
    }

    Ok(seeds)
}

// ── Invariant check ───────────────────────────────────────────────────────────

/// Returns (duplicate_count, invariant_ok).
/// Invariant: each payment_id appears at most once per tenant in ar_recon_matches.
async fn check_match_uniqueness(pool: &PgPool, run_prefix: &str) -> Result<(i64, bool)> {
    let dupe_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM (
            SELECT app_id, payment_id
            FROM ar_recon_matches
            WHERE app_id LIKE $1
            GROUP BY app_id, payment_id
            HAVING COUNT(*) > 1
        ) dupes
        "#,
    )
    .bind(format!("bench-rc-{}-%", run_prefix))
    .fetch_one(pool)
    .await
    .context("check_match_uniqueness")?;

    Ok((dupe_count, dupe_count == 0))
}

// ── Dry run ───────────────────────────────────────────────────────────────────

async fn run_dry(cfg: &Config) -> Result<ScenarioResult> {
    let pool = pg_pool(&cfg.database_url, 2).await?;
    let mut samples = MetricsSamples::new();
    let wall = Timer::start();
    for _ in 0..5usize {
        let t = Timer::start();
        match sqlx::query("SELECT 1").fetch_one(&pool).await {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    samples.set_wall_clock(wall.elapsed());
    Ok(ScenarioResult {
        name: "recon".to_string(),
        passed: true,
        metrics: samples.to_json(),
        threshold_violations: vec![],
        notes: Some("dry-run: 5 AR DB pings (connectivity check only)".to_string()),
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn pg_pool(url: &str, max_conn: u32) -> Result<PgPool> {
    Ok(sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_conn)
        .acquire_timeout(Duration::from_secs(10))
        .connect(url)
        .await?)
}

fn read_env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
