//! Performance baseline benchmark for core shipping-receiving operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for create_shipment, add_line, list_shipments, and get_shipment.
//!
//! Usage:
//!   cargo run --manifest-path modules/shipping-receiving/Cargo.toml --bin bench -- --duration 30
//!   cargo run --manifest-path modules/shipping-receiving/Cargo.toml --bin bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL — shipping-receiving database connection string

use chrono::Utc;
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
        .expect("Failed to connect to shipping-receiving DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_create_shipment(
    pool: &PgPool,
    tenant_id: Uuid,
    direction: &str,
    bench: &mut BenchResult,
) -> Uuid {
    let start = Instant::now();
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO shipments (tenant_id, direction, status, currency)
        VALUES ($1, $2, 'draft', 'usd')
        RETURNING id
        "#,
    )
    .bind(tenant_id)
    .bind(direction)
    .fetch_one(pool)
    .await
    .expect("create shipment");
    bench.record(start.elapsed());
    row.0
}

async fn bench_add_line(
    pool: &PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    bench: &mut BenchResult,
) {
    let start = Instant::now();
    sqlx::query(
        r#"
        INSERT INTO shipment_lines (tenant_id, shipment_id, sku, qty_expected, warehouse_id)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(tenant_id)
    .bind(shipment_id)
    .bind(format!("BENCH-SKU-{}", Uuid::new_v4()))
    .bind(100i64)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("add line");
    bench.record(start.elapsed());
}

async fn bench_list_shipments(pool: &PgPool, tenant_id: Uuid, bench: &mut BenchResult) {
    let start = Instant::now();
    let _rows: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id FROM shipments
        WHERE tenant_id = $1
        ORDER BY created_at DESC
        LIMIT 50
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await
    .expect("list shipments");
    bench.record(start.elapsed());
}

async fn bench_get_shipment(
    pool: &PgPool,
    tenant_id: Uuid,
    shipment_id: Uuid,
    bench: &mut BenchResult,
) {
    let start = Instant::now();
    let _row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM shipments WHERE id = $1 AND tenant_id = $2")
            .bind(shipment_id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await
            .expect("get shipment");
    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = Uuid::new_v4();

    println!("=== Shipping-Receiving Performance Baseline ===");
    println!("Tenant:     {}", tenant_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    // Seed a few shipments so list/get have data to work with
    let mut seed_ids = Vec::new();
    for dir in ["inbound", "outbound"] {
        for _ in 0..5 {
            let id: (Uuid,) = sqlx::query_as(
                r#"
                INSERT INTO shipments (tenant_id, direction, status, currency)
                VALUES ($1, $2, 'draft', 'usd')
                RETURNING id
                "#,
            )
            .bind(tenant_id)
            .bind(dir)
            .fetch_one(&pool)
            .await
            .expect("seed shipment");
            seed_ids.push(id.0);
        }
    }

    let mut create_bench = BenchResult::new("create_shipment");
    let mut add_line_bench = BenchResult::new("add_line");
    let mut list_bench = BenchResult::new("list_shipments");
    let mut get_bench = BenchResult::new("get_shipment");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        let direction = if iter % 2 == 0 { "inbound" } else { "outbound" };
        let shipment_id =
            bench_create_shipment(&pool, tenant_id, direction, &mut create_bench).await;

        bench_add_line(&pool, tenant_id, shipment_id, &mut add_line_bench).await;
        bench_list_shipments(&pool, tenant_id, &mut list_bench).await;

        let lookup_id = seed_ids[iter % seed_ids.len()];
        bench_get_shipment(&pool, tenant_id, lookup_id, &mut get_bench).await;

        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    create_bench.report();
    add_line_bench.report();
    list_bench.report();
    get_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");
}
