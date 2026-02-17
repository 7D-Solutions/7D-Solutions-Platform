//! Benchmark scenario implementations for the stabilization gate.
//!
//! Wave 0 (bd-jsko): each function validates connectivity and collects baseline
//! measurements. Wave 1 beads (bd-3oio, bd-3mzn, bd-1kmq) will replace the
//! stub bodies with production-load workloads.

use std::time::Duration;

use anyhow::Result;
use tracing::info;

use crate::config::Config;
use crate::metrics::{MetricsSamples, Timer};
use crate::report::ScenarioResult;

// ── Thresholds (Wave 0 baseline — loosened for connectivity-only runs) ──────

/// Maximum allowed P99 latency for a Postgres ping (ms).
const DB_PING_P99_MS: f64 = 200.0;
// NATS_PUBLISH_P99_MS threshold moved to eventbus.rs (Wave 1, bd-3oio).

// ── Aggregate runner ─────────────────────────────────────────────────────────

/// Run every scenario and return their individual results.
pub async fn run_all(cfg: &Config, dry_run: bool) -> Result<Vec<ScenarioResult>> {
    Ok(vec![
        bench_eventbus(cfg, dry_run).await?,
        bench_projections(cfg, dry_run).await?,
        bench_recon(cfg, dry_run).await?,
        bench_dunning(cfg, dry_run).await?,
        bench_tenants(cfg, dry_run).await?,
    ])
}

// ── Individual scenarios ─────────────────────────────────────────────────────

/// NATS event bus publish/consume throughput.
///
/// Delegates to the full Wave-1 implementation in `crate::eventbus`.
pub async fn bench_eventbus(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    info!("Running scenario: eventbus (dry_run={})", dry_run);
    crate::eventbus::run(cfg, dry_run).await
}

/// Projection table query latency.
pub async fn bench_projections(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    info!("Running scenario: projections (dry_run={})", dry_run);

    let pool = pg_pool(&cfg.database_url, 4).await?;
    let total = if dry_run { 5 } else { cfg.recon_rows };

    let mut samples = MetricsSamples::new();
    let wall = Timer::start();

    for _ in 0..total {
        let t = Timer::start();
        let res = sqlx::query("SELECT 1 AS ping")
            .fetch_one(&pool)
            .await;
        match res {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    samples.set_wall_clock(wall.elapsed());

    let mut violations = Vec::new();
    if samples.p99() > DB_PING_P99_MS && !dry_run {
        violations.push(format!(
            "Projection P99 query latency {:.1}ms exceeds threshold {:.1}ms",
            samples.p99(),
            DB_PING_P99_MS
        ));
    }

    Ok(ScenarioResult {
        name: "projections".to_string(),
        passed: violations.is_empty(),
        metrics: samples.to_json(),
        threshold_violations: violations,
        notes: dry_note(dry_run, total, "DB pings (projection proxy)"),
    })
}

/// AR reconciliation batch throughput.
pub async fn bench_recon(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    info!("Running scenario: recon (dry_run={})", dry_run);

    let pool = pg_pool(&cfg.database_url, cfg.concurrency as u32).await?;
    let total = if dry_run { 5 } else { cfg.recon_rows };

    let mut samples = MetricsSamples::new();
    let wall = Timer::start();

    for _ in 0..total {
        let t = Timer::start();
        let res = sqlx::query("SELECT 1").fetch_one(&pool).await;
        match res {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    samples.set_wall_clock(wall.elapsed());

    let mut violations = Vec::new();
    if samples.p99() > DB_PING_P99_MS && !dry_run {
        violations.push(format!(
            "Recon P99 latency {:.1}ms exceeds threshold {:.1}ms",
            samples.p99(),
            DB_PING_P99_MS
        ));
    }

    Ok(ScenarioResult {
        name: "recon".to_string(),
        passed: violations.is_empty(),
        metrics: samples.to_json(),
        threshold_violations: violations,
        notes: dry_note(dry_run, total, "row-probe pings (recon proxy)"),
    })
}

/// Dunning scheduler row processing throughput.
pub async fn bench_dunning(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    info!("Running scenario: dunning (dry_run={})", dry_run);

    let pool = pg_pool(&cfg.database_url, cfg.concurrency as u32).await?;
    let total = if dry_run { 5 } else { cfg.dunning_rows };

    let mut samples = MetricsSamples::new();
    let wall = Timer::start();

    for _ in 0..total {
        let t = Timer::start();
        let res = sqlx::query("SELECT 1").fetch_one(&pool).await;
        match res {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    samples.set_wall_clock(wall.elapsed());

    let mut violations = Vec::new();
    if samples.p99() > DB_PING_P99_MS && !dry_run {
        violations.push(format!(
            "Dunning P99 latency {:.1}ms exceeds threshold {:.1}ms",
            samples.p99(),
            DB_PING_P99_MS
        ));
    }

    Ok(ScenarioResult {
        name: "dunning".to_string(),
        passed: violations.is_empty(),
        metrics: samples.to_json(),
        threshold_violations: violations,
        notes: dry_note(dry_run, total, "row-probe pings (dunning proxy)"),
    })
}

/// Multi-tenant isolation and stress.
pub async fn bench_tenants(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    info!("Running scenario: tenants (dry_run={})", dry_run);

    let pool = pg_pool(&cfg.database_url, cfg.concurrency as u32).await?;
    let total = if dry_run { cfg.tenant_count.min(3) } else { cfg.tenant_count };

    let mut samples = MetricsSamples::new();
    let wall = Timer::start();

    for _ in 0..total {
        let t = Timer::start();
        let res = sqlx::query("SELECT 1").fetch_one(&pool).await;
        match res {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    samples.set_wall_clock(wall.elapsed());

    let mut violations = Vec::new();
    if samples.p99() > DB_PING_P99_MS && !dry_run {
        violations.push(format!(
            "Tenants P99 latency {:.1}ms exceeds threshold {:.1}ms",
            samples.p99(),
            DB_PING_P99_MS
        ));
    }

    Ok(ScenarioResult {
        name: "tenants".to_string(),
        passed: violations.is_empty(),
        metrics: samples.to_json(),
        threshold_violations: violations,
        notes: dry_note(dry_run, total, "tenant pings"),
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

async fn pg_pool(url: &str, max_conn: u32) -> Result<sqlx::PgPool> {
    Ok(sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_conn)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await?)
}

fn dry_note(dry_run: bool, total: usize, label: &str) -> Option<String> {
    if dry_run {
        Some(format!("dry-run: {} {} (connectivity check only)", total, label))
    } else {
        None
    }
}
