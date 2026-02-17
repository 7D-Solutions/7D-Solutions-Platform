//! Event bus throughput benchmark (bd-3oio, Wave 1).
//!
//! Measures NATS publish/consume throughput and end-to-end delivery latency
//! under concurrent multi-tenant load with real EventEnvelope serialization.
//!
//! Thresholds (all configurable via env vars):
//!   EVENTBUS_MIN_THROUGHPUT  — minimum events/sec (default 500)
//!   EVENTBUS_MAX_P99_MS      — maximum P99 end-to-end latency ms (default 500)
//!   EVENTBUS_MAX_DROP_COUNT  — maximum allowed drops (default 0)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use event_bus::EventEnvelope;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};
use tracing::info;
use uuid::Uuid;

use crate::config::Config;
use crate::metrics::{MetricsSamples, Timer};
use crate::report::ScenarioResult;

// ── Default thresholds ────────────────────────────────────────────────────────

const DEFAULT_MIN_THROUGHPUT: f64 = 500.0; // events/sec sustained publish rate
const DEFAULT_MAX_P99_MS: f64 = 500.0; // ms end-to-end delivery latency
const DEFAULT_MAX_DROP_COUNT: u64 = 0; // zero drop tolerance

// ── Payload ───────────────────────────────────────────────────────────────────

/// Payload for each benchmark event.
///
/// Unique `idempotency_key` per event prevents NATS dedup from hiding load.
/// `sent_at_ns` enables nanosecond-resolution latency measurement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchPayload {
    /// Unique per-event key — prevents dedup from hiding load.
    pub idempotency_key: String,
    /// Nanoseconds since UNIX_EPOCH at publish time.
    pub sent_at_ns: u64,
    /// Tenant owning this event.
    pub tenant_id: String,
    /// Sequence within tenant's batch.
    pub seq: usize,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run the event bus throughput benchmark.
///
/// In dry-run mode: 5 raw NATS publishes to verify connectivity.
/// In production mode: full concurrent publish/consume with metric collection.
pub async fn run(cfg: &Config, dry_run: bool) -> Result<ScenarioResult> {
    if dry_run {
        return run_dry(cfg).await;
    }

    let tenant_count = cfg.tenant_count;
    let events_per_tenant = cfg.events_per_tenant;
    let concurrency = cfg.concurrency;
    let drain_timeout_secs = cfg.duration_secs;
    let total_events = tenant_count * events_per_tenant;

    info!(
        "eventbus: {} tenants × {} events = {} total, concurrency={}",
        tenant_count, events_per_tenant, total_events, concurrency
    );

    // ── 1. Subscribe BEFORE publishers to avoid message loss ─────────────────

    let sub_nc = async_nats::connect(&cfg.nats_url)
        .await
        .context("consumer: NATS connect")?;
    let mut subscriber = sub_nc
        .subscribe("stabilization-gate.bench.>".to_string())
        .await
        .context("consumer: subscribe")?;

    // Channel: consumer task → main thread (idempotency_key, latency_ms).
    let (tx, mut rx) = mpsc::unbounded_channel::<(String, f64)>();

    // Consumer task drains the subscriber into the channel until aborted.
    let consumer_task = tokio::spawn(async move {
        while let Some(msg) = subscriber.next().await {
            let now_ns = now_nanos();
            if let Ok(env) =
                serde_json::from_slice::<EventEnvelope<BenchPayload>>(&msg.payload)
            {
                let latency_ms =
                    now_ns.saturating_sub(env.payload.sent_at_ns) as f64 / 1_000_000.0;
                let _ = tx.send((env.payload.idempotency_key, latency_ms));
            }
        }
    });

    // ── 2. Build per-event work items ─────────────────────────────────────────

    let tenant_ids: Vec<String> = (0..tenant_count)
        .map(|i| format!("bench-tenant-{:04}", i))
        .collect();

    // Pre-generate idempotency keys so we know exactly what was sent.
    let work: Vec<(usize, usize, String)> = (0..tenant_count)
        .flat_map(|ti| {
            (0..events_per_tenant)
                .map(move |si| (ti, si, Uuid::new_v4().to_string()))
        })
        .collect();

    let work_queue = Arc::new(Mutex::new(work.into_iter()));
    let publish_ok = Arc::new(AtomicU64::new(0));
    let publish_err = Arc::new(AtomicU64::new(0));

    // ── 3. Spawn concurrent publishers ────────────────────────────────────────

    let wall = Timer::start();
    let mut handles = Vec::with_capacity(concurrency);

    for _ in 0..concurrency {
        let nats_url = cfg.nats_url.clone();
        let tenant_ids = tenant_ids.clone();
        let queue = work_queue.clone();
        let ok_ctr = publish_ok.clone();
        let err_ctr = publish_err.clone();

        handles.push(tokio::spawn(async move {
            let nc = match async_nats::connect(&nats_url).await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("publisher NATS connect failed: {}", e);
                    return;
                }
            };

            loop {
                let item = { queue.lock().await.next() };
                let (ti, seq, key) = match item {
                    Some(v) => v,
                    None => break,
                };

                let tenant_id = &tenant_ids[ti];
                let payload = BenchPayload {
                    idempotency_key: key,
                    sent_at_ns: now_nanos(),
                    tenant_id: tenant_id.clone(),
                    seq,
                };

                let envelope = EventEnvelope::new(
                    tenant_id.clone(),
                    "stabilization-gate".to_string(),
                    "bench.event.v1".to_string(),
                    payload,
                );

                let subject = format!("stabilization-gate.bench.{}", tenant_id);
                match serde_json::to_vec(&envelope) {
                    Ok(bytes) => match nc.publish(subject, bytes.into()).await {
                        Ok(_) => {
                            ok_ctr.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(_) => {
                            err_ctr.fetch_add(1, Ordering::Relaxed);
                        }
                    },
                    Err(_) => {
                        err_ctr.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            let _ = nc.flush().await;
        }));
    }

    // Wait for all publishers to finish.
    for h in handles {
        let _ = h.await;
    }

    let publish_elapsed = wall.elapsed();
    let pub_sent = publish_ok.load(Ordering::Relaxed) as usize;
    let pub_errors = publish_err.load(Ordering::Relaxed) as usize;

    info!(
        "eventbus: published {} events in {:.3}s ({} errors)",
        pub_sent,
        publish_elapsed.as_secs_f64(),
        pub_errors
    );

    // ── 4. Drain consumer with deadline ──────────────────────────────────────

    let mut received: HashMap<String, f64> = HashMap::with_capacity(pub_sent);
    let drain_end = Instant::now() + Duration::from_secs(drain_timeout_secs);

    while received.len() < pub_sent {
        let remaining = drain_end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            info!(
                "eventbus: drain deadline reached, {} messages outstanding",
                pub_sent - received.len()
            );
            break;
        }
        let chunk = remaining.min(Duration::from_millis(200));
        match tokio::time::timeout(chunk, rx.recv()).await {
            Ok(Some((key, latency))) => {
                received.entry(key).or_insert(latency);
            }
            Ok(None) => break, // channel closed
            Err(_) => {}       // chunk timeout — keep trying
        }
    }

    consumer_task.abort();

    // ── 5. Compute metrics ────────────────────────────────────────────────────

    let recv_count = received.len();
    let drop_count = pub_sent.saturating_sub(recv_count);
    let throughput = pub_sent as f64 / publish_elapsed.as_secs_f64().max(0.001);

    let mut samples = MetricsSamples::new();
    for lat in received.values() {
        samples.record_latency(Duration::from_secs_f64(lat / 1000.0));
    }
    for _ in 0..pub_errors {
        samples.record_error();
    }
    samples.set_wall_clock(publish_elapsed);

    let consumer_lag_peak_ms: f64 = received.values().cloned().fold(0.0_f64, f64::max);

    info!(
        "eventbus: sent={} recv={} drop={} throughput={:.1}ops/s \
         p50={:.1}ms p95={:.1}ms p99={:.1}ms lag_peak={:.1}ms",
        pub_sent,
        recv_count,
        drop_count,
        throughput,
        samples.p50(),
        samples.p95(),
        samples.p99(),
        consumer_lag_peak_ms,
    );

    // ── 6. Threshold enforcement ──────────────────────────────────────────────

    let min_throughput = read_env_f64("EVENTBUS_MIN_THROUGHPUT", DEFAULT_MIN_THROUGHPUT);
    let max_p99_ms = read_env_f64("EVENTBUS_MAX_P99_MS", DEFAULT_MAX_P99_MS);
    let max_drop = read_env_u64("EVENTBUS_MAX_DROP_COUNT", DEFAULT_MAX_DROP_COUNT);

    let mut violations: Vec<String> = Vec::new();

    if throughput < min_throughput {
        violations.push(format!(
            "publish throughput {:.1} events/sec < threshold {:.1} events/sec",
            throughput, min_throughput
        ));
    }

    if recv_count > 0 && samples.p99() > max_p99_ms {
        violations.push(format!(
            "P99 end-to-end latency {:.1}ms > threshold {:.1}ms",
            samples.p99(),
            max_p99_ms
        ));
    }

    if drop_count as u64 > max_drop {
        violations.push(format!(
            "drop count {} > allowed {} (received {} of {} sent)",
            drop_count, max_drop, recv_count, pub_sent
        ));
    }

    // Explicit sent/recv mismatch per acceptance criteria.
    if pub_sent != recv_count && drop_count as u64 <= max_drop {
        violations.push(format!(
            "received_count ({}) != sent_count ({}) — {} events unaccounted",
            recv_count, pub_sent, drop_count
        ));
    }

    let metrics = serde_json::json!({
        "p50_ms": samples.p50(),
        "p95_ms": samples.p95(),
        "p99_ms": samples.p99(),
        "total_ops": samples.total_ops,
        "errors": pub_errors,
        "wall_clock_ms": publish_elapsed.as_secs_f64() * 1000.0,
        "throughput_ops_per_sec": throughput,
        "sent_count": pub_sent,
        "recv_count": recv_count,
        "drop_count": drop_count,
        "consumer_lag_peak_ms": consumer_lag_peak_ms,
        "tenant_count": tenant_count,
        "events_per_tenant": events_per_tenant,
        "concurrency": concurrency,
    });

    Ok(ScenarioResult {
        name: "eventbus".to_string(),
        passed: violations.is_empty(),
        metrics,
        threshold_violations: violations,
        notes: None,
    })
}

// ── Dry-run ───────────────────────────────────────────────────────────────────

async fn run_dry(cfg: &Config) -> Result<ScenarioResult> {
    let nc = async_nats::connect(&cfg.nats_url)
        .await
        .context("dry-run: NATS connect")?;

    let mut samples = MetricsSamples::new();
    let wall = Timer::start();

    for _ in 0..5usize {
        let t = Timer::start();
        match nc
            .publish("stabilization-gate.bench.dry-run", b"ping".as_ref().into())
            .await
        {
            Ok(_) => samples.record_latency(t.elapsed()),
            Err(_) => samples.record_error(),
        }
    }
    let _ = nc.flush().await;
    samples.set_wall_clock(wall.elapsed());

    Ok(ScenarioResult {
        name: "eventbus".to_string(),
        passed: true,
        metrics: samples.to_json(),
        threshold_violations: vec![],
        notes: Some("dry-run: 5 NATS publishes (connectivity check only)".to_string()),
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

fn read_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
