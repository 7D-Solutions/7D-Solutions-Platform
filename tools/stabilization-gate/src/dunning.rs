//! AR dunning scheduler processing throughput benchmark (bd-1kmq, Wave 1).
//!
//! Seeds overdue invoices per tenant, runs init_dunning + transition_dunning
//! (Pending → Warned) cycles concurrently, and validates the no-duplicate-
//! processing invariant via the `version` column (each row = 1 init + 1
//! transition → version should be exactly 2).
//!
//! Env thresholds:
//!   DUNNING_MIN_THROUGHPUT_PER_SEC  (default 50)
//!   DUNNING_MAX_DRAIN_SECS          (default 300)

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use ar_rs::dunning::{
    init_dunning, transition_dunning, DunningStateValue, InitDunningRequest,
    TransitionDunningRequest,
};
use sqlx::PgPool;
use tokio::task::JoinSet;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::metrics::{MetricsSamples, Timer};
use crate::report::ScenarioResult;

// ── Thresholds ────────────────────────────────────────────────────────────────

const DEFAULT_DUNNING_MIN_THROUGHPUT: f64 = 50.0;
const DEFAULT_DUNNING_MAX_DRAIN_SECS: f64 = 300.0;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    if dry_run {
        return run_dry(cfg).await;
    }

    let concurrency = cfg.concurrency;
    let tenant_count = cfg.tenant_count;
    let rows_per_tenant = (cfg.dunning_rows / tenant_count.max(1)).max(1);

    let max_conn = (concurrency + 4).min(100) as u32;
    let pool = pg_pool(&cfg.database_url, max_conn)
        .await
        .context("dunning: DB connect")?;

    let run_prefix = Uuid::new_v4().simple().to_string()[..8].to_string();

    info!(
        "dunning: seeding {} tenants × {} rows (run_prefix={})",
        tenant_count, rows_per_tenant, run_prefix
    );

    // Phase 1: Seed invoices per tenant
    let seeds = seed_tenants(&pool, &run_prefix, tenant_count, rows_per_tenant)
        .await
        .context("dunning: seed_tenants")?;

    let total_rows: u64 = seeds.iter().map(|(_, ids)| ids.len() as u64).sum();

    info!(
        "dunning: running init+transition cycles for {} rows (concurrency={})…",
        total_rows, concurrency
    );

    // Phase 2+3: Init dunning + transition Pending→Warned, one task per invoice
    let wall = Timer::start();
    let mut samples = MetricsSamples::new();
    let mut processed: u64 = 0;

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut join_set: JoinSet<_> = JoinSet::new();

    for (app_id, invoice_ids) in &seeds {
        for &invoice_id in invoice_ids {
            let pool2 = pool.clone();
            let app_id2 = app_id.clone();
            let sem2 = sem.clone();
            join_set.spawn(async move {
                let _permit = sem2.acquire_owned().await.unwrap();
                let t = Timer::start();
                let ok = process_one(&pool2, &app_id2, invoice_id).await;
                (ok, t.elapsed())
            });
        }
    }

    while let Some(outcome) = join_set.join_next().await {
        match outcome {
            Ok((true, lat)) => {
                processed += 1;
                samples.record_latency(lat);
            }
            Ok((false, _)) => {
                samples.record_error();
            }
            Err(e) => {
                tracing::warn!("dunning task join error: {}", e);
                samples.record_error();
            }
        }
    }

    samples.set_wall_clock(wall.elapsed());
    let wall_secs = wall.elapsed().as_secs_f64();

    // Phase 4: Invariant — no row processed more than once (version must be ≤ 2)
    let (dupe_count, invariant_ok) = check_no_duplicate_processing(&pool, &run_prefix)
        .await
        .context("dunning: invariant check")?;

    let throughput = processed as f64 / wall_secs.max(0.001);

    info!(
        "dunning: done — processed={}/{} errors={} throughput={:.1}/s drain={:.2}s invariant={}",
        processed,
        total_rows,
        samples.errors,
        throughput,
        wall_secs,
        if invariant_ok { "PASS" } else { "FAIL" }
    );

    // Phase 5: Threshold enforcement
    let min_throughput =
        read_env_f64("DUNNING_MIN_THROUGHPUT_PER_SEC", DEFAULT_DUNNING_MIN_THROUGHPUT);
    let max_drain_secs =
        read_env_f64("DUNNING_MAX_DRAIN_SECS", DEFAULT_DUNNING_MAX_DRAIN_SECS);

    let mut violations: Vec<String> = Vec::new();

    if !invariant_ok {
        violations.push(format!(
            "invariant: {} dunning rows processed >1 time (version > 2) — FAIL",
            dupe_count
        ));
    }
    if processed > 0 && throughput < min_throughput {
        violations.push(format!(
            "throughput {:.1} rows/s < threshold {:.1} rows/s",
            throughput, min_throughput
        ));
    }
    if wall_secs > max_drain_secs {
        violations.push(format!(
            "drain time {:.1}s > threshold {:.1}s",
            wall_secs, max_drain_secs
        ));
    }

    Ok(ScenarioResult {
        name: "dunning".to_string(),
        passed: violations.is_empty(),
        metrics: serde_json::json!({
            "tenant_count": tenant_count,
            "dunning_rows": total_rows,
            "rows_per_tenant": rows_per_tenant,
            "concurrency": concurrency,
            "processed": processed,
            "errors": samples.errors,
            "throughput_per_sec": throughput,
            "wall_secs": wall_secs,
            "invariant_duplicate_transitions": dupe_count,
            "invariant_passed": invariant_ok,
            "p50_cycle_ms": samples.p50(),
            "p95_cycle_ms": samples.p95(),
            "p99_cycle_ms": samples.p99(),
        }),
        threshold_violations: violations,
        notes: None,
    })
}

// ── Per-invoice cycle ─────────────────────────────────────────────────────────

/// Run one full dunning cycle: init (→ Pending) then transition (Pending → Warned).
/// Returns `true` if both operations succeed.
async fn process_one(pool: &PgPool, app_id: &str, invoice_id: i32) -> bool {
    let dunning_id = Uuid::new_v4();
    let customer_id = format!("bench-dn-cust-{}", invoice_id);
    let correlation_id = format!("bench-dn-{}", dunning_id.simple());

    let init_res = init_dunning(
        pool,
        InitDunningRequest {
            dunning_id,
            app_id: app_id.to_string(),
            invoice_id,
            customer_id,
            next_attempt_at: None,
            correlation_id: correlation_id.clone(),
            causation_id: None,
        },
    )
    .await;

    if let Err(e) = init_res {
        tracing::warn!("init_dunning invoice={} error: {}", invoice_id, e);
        return false;
    }

    let transition_res = transition_dunning(
        pool,
        TransitionDunningRequest {
            app_id: app_id.to_string(),
            invoice_id,
            to_state: DunningStateValue::Warned,
            reason: "bench_overdue_attempt".to_string(),
            next_attempt_at: None,
            last_error: None,
            correlation_id,
            causation_id: Some(dunning_id.to_string()),
        },
    )
    .await;

    if let Err(e) = transition_res {
        tracing::warn!("transition_dunning invoice={} error: {}", invoice_id, e);
        return false;
    }

    true
}

// ── Seeding ───────────────────────────────────────────────────────────────────

/// Seed one customer and `rows_per_tenant` overdue invoices per tenant.
/// Returns Vec<(app_id, invoice_ids)>.
async fn seed_tenants(
    pool: &PgPool,
    run_prefix: &str,
    tenant_count: usize,
    rows_per_tenant: usize,
) -> Result<Vec<(String, Vec<i32>)>> {
    let mut seeds = Vec::with_capacity(tenant_count);

    for i in 0..tenant_count {
        let app_id = format!("bench-dn-{}-{:04}", run_prefix, i);

        let customer_id: i32 = sqlx::query_scalar(
            r#"
            INSERT INTO ar_customers (app_id, external_customer_id, email, status)
            VALUES ($1, $2, $3, 'active')
            RETURNING id
            "#,
        )
        .bind(&app_id)
        .bind(format!("bench-dn-cust-{}-{:04}", run_prefix, i))
        .bind(format!("bench-dn-{}-{}@example.com", run_prefix, i))
        .fetch_one(pool)
        .await
        .with_context(|| format!("seed customer for tenant {}", app_id))?;

        let mut invoice_ids = Vec::with_capacity(rows_per_tenant);

        for j in 0..rows_per_tenant {
            let tilled_invoice_id =
                format!("bench-dn-inv-{}-{:04}-{:06}", run_prefix, i, j);
            let invoice_id: i32 = sqlx::query_scalar(
                r#"
                INSERT INTO ar_invoices (
                    app_id, tilled_invoice_id, ar_customer_id,
                    status, amount_cents, currency,
                    due_at, updated_at
                ) VALUES (
                    $1, $2, $3,
                    'open', $4, 'usd',
                    NOW() - INTERVAL '30 days', NOW()
                )
                RETURNING id
                "#,
            )
            .bind(&app_id)
            .bind(&tilled_invoice_id)
            .bind(customer_id)
            .bind(10_000 + (j as i32) * 100)
            .fetch_one(pool)
            .await
            .with_context(|| format!("seed invoice {} for {}", j, app_id))?;

            invoice_ids.push(invoice_id);
        }

        seeds.push((app_id, invoice_ids));
    }

    Ok(seeds)
}

// ── Invariant check ───────────────────────────────────────────────────────────

/// Returns (over_processed_count, invariant_ok).
///
/// Invariant: after exactly 1 init + 1 transition, version = 2.
/// version > 2 indicates a row was transitioned more than once.
async fn check_no_duplicate_processing(
    pool: &PgPool,
    run_prefix: &str,
) -> Result<(i64, bool)> {
    let over_processed: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM ar_dunning_states
        WHERE app_id LIKE $1
          AND version > 2
        "#,
    )
    .bind(format!("bench-dn-{}-%", run_prefix))
    .fetch_one(pool)
    .await
    .context("check_no_duplicate_processing")?;

    Ok((over_processed, over_processed == 0))
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
        name: "dunning".to_string(),
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
