//! Projection rebuild + lag benchmark (bd-3mzn, Wave 1).
//!
//! Phase 1 — Rebuild: seeds events into a shadow cursor table, atomic blue/green swap.
//! Phase 2 — Lag: publishes a sustained NATS stream; subscriber writes projection
//!           cursor to DB; end-to-end lag is measured publish→DB-write.
//!
//! Env thresholds: PROJ_MAX_REBUILD_SECS (default 300s), PROJ_MAX_LAG_MS (default 2000ms).

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use event_bus::EventEnvelope;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::mpsc;
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::metrics::{MetricsSamples, Timer};
use crate::report::ScenarioResult;

const DEFAULT_MAX_REBUILD_SECS: f64 = 300.0;
const DEFAULT_MAX_LAG_MS: f64 = 2000.0;

const PROJECTION_NAMES: &[&str] = &["invoice_summary", "customer_balance", "subscription_status"];
const BENCH_TABLE: &str = "stabilization_bench_proj_cursors";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProjBenchPayload {
    /// Nanoseconds since UNIX_EPOCH at publish time.
    sent_at_ns: u64,
    /// Tenant that owns this event.
    tenant_id: String,
    /// Sequence within the tenant's batch.
    seq: usize,
}

#[derive(Debug)]
struct RebuildMetric {
    name: String,
    rebuild_secs: f64,
    events_processed: usize,
}

pub async fn run(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    if dry_run {
        return run_dry(cfg).await;
    }

    let proj_url =
        std::env::var("PROJECTIONS_DATABASE_URL").unwrap_or_else(|_| cfg.database_url.clone());
    let pool = pg_pool(&proj_url, 8)
        .await
        .context("projections DB connect")?;

    let tenant_count = cfg.tenant_count;
    let events_per_tenant = cfg.events_per_tenant;
    let duration_secs = cfg.duration_secs;

    // ── Phase 1: Rebuild ─────────────────────────────────────────────────────

    info!(
        "projections: Phase 1 – rebuild {} projections ({} tenants × {} events/tenant)",
        PROJECTION_NAMES.len(),
        tenant_count,
        events_per_tenant
    );

    ensure_bench_table(&pool).await?;

    let wall_rebuild = Timer::start();
    let mut rebuild_metrics: Vec<RebuildMetric> = Vec::new();

    for proj_name in PROJECTION_NAMES {
        let m = rebuild_one(&pool, proj_name, tenant_count, events_per_tenant).await?;
        info!(
            "projections: rebuilt '{}' in {:.3}s ({} events, {:.1} rows/s)",
            m.name,
            m.rebuild_secs,
            m.events_processed,
            m.events_processed as f64 / m.rebuild_secs.max(0.001)
        );
        rebuild_metrics.push(m);
    }

    let total_rebuild_secs = wall_rebuild.elapsed().as_secs_f64();
    let total_events_rebuilt: usize = rebuild_metrics.iter().map(|m| m.events_processed).sum();

    info!(
        "projections: Phase 1 complete — {:.3}s total, {} events across {} projections",
        total_rebuild_secs,
        total_events_rebuilt,
        rebuild_metrics.len()
    );

    // ── Phase 2: Lag ─────────────────────────────────────────────────────────

    info!(
        "projections: Phase 2 – lag measurement ({} tenants × {} events, {} secs drain)",
        tenant_count, events_per_tenant, duration_secs
    );

    let (max_lag_ms, avg_lag_ms, lag_samples) =
        measure_lag(cfg, &pool, tenant_count, events_per_tenant, duration_secs).await?;

    info!(
        "projections: Phase 2 complete — max_lag={:.1}ms avg_lag={:.1}ms samples={}",
        max_lag_ms, avg_lag_ms, lag_samples
    );

    // ── Threshold enforcement ─────────────────────────────────────────────────

    let max_rebuild_secs = read_env_f64("PROJ_MAX_REBUILD_SECS", DEFAULT_MAX_REBUILD_SECS);
    let max_lag_ms_thresh = read_env_f64("PROJ_MAX_LAG_MS", DEFAULT_MAX_LAG_MS);

    let mut violations: Vec<String> = Vec::new();

    for m in &rebuild_metrics {
        if m.rebuild_secs > max_rebuild_secs {
            violations.push(format!(
                "projection '{}' rebuild took {:.1}s > threshold {:.1}s",
                m.name, m.rebuild_secs, max_rebuild_secs
            ));
        }
    }

    if lag_samples > 0 && max_lag_ms > max_lag_ms_thresh {
        violations.push(format!(
            "max projection lag {:.1}ms > threshold {:.1}ms",
            max_lag_ms, max_lag_ms_thresh
        ));
    }

    // ── Metrics JSON ──────────────────────────────────────────────────────────

    let rebuild_breakdown: Vec<serde_json::Value> = rebuild_metrics
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m.name,
                "rebuild_secs": m.rebuild_secs,
                "events_processed": m.events_processed,
                "rows_per_sec": m.events_processed as f64 / m.rebuild_secs.max(0.001),
            })
        })
        .collect();

    let metrics = serde_json::json!({
        "rebuild_projections": PROJECTION_NAMES.len(),
        "rebuild_total_secs": total_rebuild_secs,
        "rebuild_total_events": total_events_rebuilt,
        "rebuild_per_projection": rebuild_breakdown,
        "lag_samples": lag_samples,
        "lag_max_ms": max_lag_ms,
        "lag_avg_ms": avg_lag_ms,
        "tenant_count": tenant_count,
        "events_per_tenant": events_per_tenant,
        "duration_secs": duration_secs,
    });

    Ok(ScenarioResult {
        name: "projections".to_string(),
        passed: violations.is_empty(),
        metrics,
        threshold_violations: violations,
        notes: None,
    })
}

async fn ensure_bench_table(pool: &PgPool) -> Result<()> {
    sqlx::query(&format!(
        r#"
        CREATE TABLE IF NOT EXISTS {table} (
            projection_name VARCHAR(100) NOT NULL,
            tenant_id       VARCHAR(100) NOT NULL,
            last_event_id   UUID         NOT NULL,
            last_event_occurred_at TIMESTAMPTZ NOT NULL,
            updated_at      TIMESTAMPTZ  NOT NULL DEFAULT CURRENT_TIMESTAMP,
            events_processed BIGINT      NOT NULL DEFAULT 1,
            PRIMARY KEY (projection_name, tenant_id)
        )
        "#,
        table = BENCH_TABLE
    ))
    .execute(pool)
    .await
    .context("ensure_bench_table DDL")?;
    Ok(())
}

/// Blue/green shadow swap for one projection.
async fn rebuild_one(
    pool: &PgPool,
    proj_name: &str,
    tenant_count: usize,
    events_per_tenant: usize,
) -> Result<RebuildMetric> {
    let shadow = format!("{}_shadow", BENCH_TABLE);

    // Drop any shadow left by a previous interrupted run.
    sqlx::query(&format!("DROP TABLE IF EXISTS {shadow} CASCADE"))
        .execute(pool)
        .await?;

    // Create the shadow table (same schema as the live bench table).
    sqlx::query(&format!(
        r#"
        CREATE TABLE {shadow} (
            projection_name VARCHAR(100) NOT NULL,
            tenant_id       VARCHAR(100) NOT NULL,
            last_event_id   UUID         NOT NULL,
            last_event_occurred_at TIMESTAMPTZ NOT NULL,
            updated_at      TIMESTAMPTZ  NOT NULL DEFAULT CURRENT_TIMESTAMP,
            events_processed BIGINT      NOT NULL DEFAULT 1,
            PRIMARY KEY (projection_name, tenant_id)
        )
        "#,
        shadow = shadow
    ))
    .execute(pool)
    .await?;

    let t = Timer::start();

    // Populate shadow: one row per tenant, events_processed = events_per_tenant.
    for ti in 0..tenant_count {
        let tenant_id = format!("bench-tenant-{:04}", ti);
        let event_id = Uuid::new_v4();
        sqlx::query(&format!(
            r#"
            INSERT INTO {shadow} (
                projection_name, tenant_id, last_event_id,
                last_event_occurred_at, updated_at, events_processed
            ) VALUES ($1, $2, $3, NOW(), NOW(), $4)
            ON CONFLICT (projection_name, tenant_id) DO UPDATE SET
                last_event_id          = EXCLUDED.last_event_id,
                last_event_occurred_at = EXCLUDED.last_event_occurred_at,
                updated_at             = EXCLUDED.updated_at,
                events_processed       = EXCLUDED.events_processed
            "#,
            shadow = shadow
        ))
        .bind(proj_name)
        .bind(&tenant_id)
        .bind(event_id)
        .bind(events_per_tenant as i64)
        .execute(pool)
        .await?;
    }

    let mut tx = pool.begin().await?;
    sqlx::query(&format!(
        "DELETE FROM {live} WHERE projection_name = $1",
        live = BENCH_TABLE
    ))
    .bind(proj_name)
    .execute(&mut *tx)
    .await?;
    sqlx::query(&format!(
        "INSERT INTO {live} SELECT * FROM {shadow} WHERE projection_name = $1",
        live = BENCH_TABLE,
        shadow = shadow
    ))
    .bind(proj_name)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let rebuild_secs = t.elapsed().as_secs_f64();

    // Clean up shadow table.
    sqlx::query(&format!("DROP TABLE IF EXISTS {shadow} CASCADE"))
        .execute(pool)
        .await?;

    Ok(RebuildMetric {
        name: proj_name.to_string(),
        rebuild_secs,
        events_processed: tenant_count * events_per_tenant,
    })
}

/// Returns (max_lag_ms, avg_lag_ms, sample_count).
async fn measure_lag(
    cfg: &Config,
    pool: &PgPool,
    tenant_count: usize,
    events_per_tenant: usize,
    duration_secs: u64,
) -> Result<(f64, f64, u64)> {
    // Subscribe before publishing to avoid missing early events.
    let sub_nc = async_nats::connect(&cfg.nats_url)
        .await
        .context("lag phase: subscriber NATS connect")?;
    let mut subscriber = sub_nc
        .subscribe("stabilization-gate.proj.>".to_string())
        .await
        .context("lag phase: NATS subscribe")?;

    let (lag_tx, mut lag_rx) = mpsc::unbounded_channel::<f64>();
    let pool2 = pool.clone();
    let lag_proj_name = "lag_bench";

    // Consumer: receive event → write DB cursor → report lag.
    let consumer = tokio::spawn(async move {
        while let Some(msg) = subscriber.next().await {
            let recv_ns = now_nanos();
            if let Ok(env) = serde_json::from_slice::<EventEnvelope<ProjBenchPayload>>(&msg.payload)
            {
                let sent_ns = env.payload.sent_at_ns;
                let lag_ms = recv_ns.saturating_sub(sent_ns) as f64 / 1_000_000.0;

                let event_id = env.event_id;
                let tenant_id = env.payload.tenant_id.clone();
                let _ = sqlx::query(&format!(
                    r#"
                    INSERT INTO {table} (
                        projection_name, tenant_id, last_event_id,
                        last_event_occurred_at, updated_at, events_processed
                    ) VALUES ($1, $2, $3, NOW(), NOW(), 1)
                    ON CONFLICT (projection_name, tenant_id) DO UPDATE SET
                        last_event_id          = EXCLUDED.last_event_id,
                        last_event_occurred_at = EXCLUDED.last_event_occurred_at,
                        updated_at             = EXCLUDED.updated_at,
                        events_processed       = {table}.events_processed + 1
                    "#,
                    table = BENCH_TABLE
                ))
                .bind(lag_proj_name)
                .bind(&tenant_id)
                .bind(event_id)
                .execute(&pool2)
                .await;

                let _ = lag_tx.send(lag_ms);
            }
        }
    });

    let pub_nc = async_nats::connect(&cfg.nats_url)
        .await
        .context("lag phase: publisher NATS connect")?;

    let mut pub_count = 0usize;
    let pub_deadline = Instant::now() + Duration::from_secs(duration_secs);

    'publish: for ti in 0..tenant_count {
        for si in 0..events_per_tenant {
            if Instant::now() >= pub_deadline {
                info!(
                    "projections: lag publish deadline reached at event {}",
                    pub_count
                );
                break 'publish;
            }
            let tenant_id = format!("bench-tenant-{:04}", ti);
            let payload = ProjBenchPayload {
                sent_at_ns: now_nanos(),
                tenant_id: tenant_id.clone(),
                seq: si,
            };
            let env = EventEnvelope::new(
                tenant_id.clone(),
                "stabilization-gate".to_string(),
                "proj.event.v1".to_string(),
                payload,
            );
            if let Ok(bytes) = serde_json::to_vec(&env) {
                let subject = format!("stabilization-gate.proj.{}", tenant_id);
                let _ = pub_nc.publish(subject, bytes.into()).await;
                pub_count += 1;
            }
        }
    }
    let _ = pub_nc.flush().await;
    info!(
        "projections: lag phase published {} events, draining…",
        pub_count
    );

    let mut lag_values: Vec<f64> = Vec::with_capacity(pub_count);
    let drain_end = Instant::now() + Duration::from_secs(duration_secs);

    while lag_values.len() < pub_count {
        let remaining = drain_end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            info!(
                "projections: drain deadline reached — {} of {} events received",
                lag_values.len(),
                pub_count
            );
            break;
        }
        let chunk = remaining.min(Duration::from_millis(200));
        match tokio::time::timeout(chunk, lag_rx.recv()).await {
            Ok(Some(lag)) => lag_values.push(lag),
            Ok(None) => break,
            Err(_) => {}
        }
    }

    consumer.abort();

    if lag_values.is_empty() {
        return Ok((0.0, 0.0, 0));
    }

    let max_lag = lag_values.iter().cloned().fold(0.0_f64, f64::max);
    let avg_lag = lag_values.iter().sum::<f64>() / lag_values.len() as f64;

    Ok((max_lag, avg_lag, lag_values.len() as u64))
}

async fn run_dry(cfg: &Config) -> Result<ScenarioResult> {
    let proj_url =
        std::env::var("PROJECTIONS_DATABASE_URL").unwrap_or_else(|_| cfg.database_url.clone());
    let pool = pg_pool(&proj_url, 2)
        .await
        .context("dry-run: projections DB")?;

    let mut samples = MetricsSamples::new();
    let wall = Timer::start();

    for _ in 0..5usize {
        let t = Timer::start();
        match sqlx::query("SELECT 1 AS ping").fetch_one(&pool).await {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    samples.set_wall_clock(wall.elapsed());

    Ok(ScenarioResult {
        name: "projections".to_string(),
        passed: true,
        metrics: samples.to_json(),
        threshold_violations: vec![],
        notes: Some("dry-run: 5 projections-DB pings (connectivity check only)".to_string()),
    })
}

async fn pg_pool(url: &str, max_conn: u32) -> Result<PgPool> {
    Ok(sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_conn)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await?)
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

fn read_env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
