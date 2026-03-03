//! Performance baseline benchmark for core numbering operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for number allocation and policy upsert.
//!
//! Usage:
//!   cargo run --manifest-path modules/numbering/Cargo.toml --bin bench -- --duration 30
//!
//! Environment:
//!   DATABASE_URL — numbering database connection string

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ── CLI args ─────────────────────────────────────────────────────────

struct Args {
    duration_secs: u64,
    iterations: Option<usize>,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut duration_secs = 30u64;
    let mut iterations = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--duration" => {
                i += 1;
                duration_secs = args[i].parse().expect("--duration must be a number");
            }
            "--iterations" => {
                i += 1;
                iterations = Some(args[i].parse().expect("--iterations must be a number"));
            }
            _ => {}
        }
        i += 1;
    }
    Args {
        duration_secs,
        iterations,
    }
}

// ── Benchmark harness ────────────────────────────────────────────────

struct BenchResult {
    name: String,
    samples: Vec<Duration>,
}

impl BenchResult {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            samples: Vec::new(),
        }
    }

    fn record(&mut self, d: Duration) {
        self.samples.push(d);
    }

    fn report(&self) {
        if self.samples.is_empty() {
            println!("  {}: no samples", self.name);
            return;
        }
        let mut sorted: Vec<f64> = self
            .samples
            .iter()
            .map(|d| d.as_secs_f64() * 1000.0)
            .collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        let p50 = sorted[n / 2];
        let p95 = sorted[(n as f64 * 0.95) as usize];
        let p99 = sorted[(n as f64 * 0.99) as usize];
        let mean = sorted.iter().sum::<f64>() / n as f64;
        let total_secs: f64 = self.samples.iter().map(|d| d.as_secs_f64()).sum();
        let throughput = n as f64 / total_secs;

        println!(
            "  {} | n={} | mean={:.2}ms p50={:.2}ms p95={:.2}ms p99={:.2}ms | {:.1} ops/s",
            self.name, n, mean, p50, p95, p99, throughput
        );

        let threshold_p95_ms = 100.0;
        if p95 > threshold_p95_ms {
            println!(
                "    WARNING: p95 ({:.2}ms) exceeds threshold ({:.0}ms)",
                p95, threshold_p95_ms
            );
        }
    }
}

// ── Setup helpers ────────────────────────────────────────────────────

async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for benchmarks");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to numbering DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_allocate(pool: &PgPool, tenant_id: Uuid, entity: &str, bench: &mut BenchResult) {
    let idem_key = format!("bench-alloc-{}", Uuid::new_v4());

    let start = Instant::now();

    let mut tx = pool.begin().await.expect("begin tx");

    // Advance sequence (or create)
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO sequences (tenant_id, entity, current_value) \
         VALUES ($1, $2, 1) \
         ON CONFLICT (tenant_id, entity) \
         DO UPDATE SET current_value = sequences.current_value + 1, updated_at = NOW() \
         RETURNING current_value",
    )
    .bind(tenant_id)
    .bind(entity)
    .fetch_one(&mut *tx)
    .await
    .expect("advance sequence");

    let number_value = row.0;

    // Record issued number
    sqlx::query(
        "INSERT INTO issued_numbers (tenant_id, entity, number_value, idempotency_key, status) \
         VALUES ($1, $2, $3, $4, 'confirmed')",
    )
    .bind(tenant_id)
    .bind(entity)
    .bind(number_value)
    .bind(&idem_key)
    .execute(&mut *tx)
    .await
    .expect("insert issued");

    // Enqueue outbox event
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "number_value": number_value,
        "idempotency_key": &idem_key,
        "status": "confirmed"
    });
    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.number.allocated")
    .bind("number")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(&payload)
    .execute(&mut *tx)
    .await
    .expect("insert outbox");

    tx.commit().await.expect("commit");

    bench.record(start.elapsed());
}

async fn bench_policy_upsert(
    pool: &PgPool,
    tenant_id: Uuid,
    entity: &str,
    bench: &mut BenchResult,
) {
    let start = Instant::now();

    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "INSERT INTO numbering_policies (tenant_id, entity, pattern, prefix, padding) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (tenant_id, entity) \
         DO UPDATE SET pattern = EXCLUDED.pattern, \
                       prefix = EXCLUDED.prefix, \
                       padding = EXCLUDED.padding, \
                       version = numbering_policies.version + 1, \
                       updated_at = NOW()",
    )
    .bind(tenant_id)
    .bind(entity)
    .bind("{prefix}-{YYYY}-{number}")
    .bind("BENCH")
    .bind(5i32)
    .execute(&mut *tx)
    .await
    .expect("upsert policy");

    // Outbox event for policy update
    let event_id = Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "entity": entity,
        "pattern": "{prefix}-{YYYY}-{number}",
        "prefix": "BENCH",
        "padding": 5,
        "version": 1
    });
    sqlx::query(
        "INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(event_id)
    .bind("numbering.events.policy.updated")
    .bind("policy")
    .bind(format!("{}:{}", tenant_id, entity))
    .bind(&payload)
    .execute(&mut *tx)
    .await
    .expect("insert outbox");

    tx.commit().await.expect("commit");

    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = Uuid::new_v4();

    println!("=== Numbering Performance Baseline ===");
    println!("Tenant:     {}", tenant_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", chrono::Utc::now());
    println!();

    let mut alloc_bench = BenchResult::new("allocate");
    let mut policy_bench = BenchResult::new("policy_upsert");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        bench_allocate(&pool, tenant_id, "bench_entity", &mut alloc_bench).await;
        bench_policy_upsert(&pool, tenant_id, "bench_entity", &mut policy_bench).await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    alloc_bench.report();
    policy_bench.report();
    println!();
    println!("Finished: {}", chrono::Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");

    // Clean up bench data
    sqlx::query("DELETE FROM events_outbox WHERE aggregate_id LIKE $1")
        .bind(format!("{}:%", tenant_id))
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM issued_numbers WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM sequences WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM numbering_policies WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await
        .ok();
}
