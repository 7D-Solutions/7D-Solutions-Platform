//! Performance baseline benchmark for core inventory operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for receipt, issue, adjustment, and transfer commands.
//!
//! Usage:
//!   cargo run --bin bench -- --duration 30
//!   cargo run --bin bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL — inventory database connection string

use chrono::Utc;
use inventory_rs::domain::{
    adjust_service::{process_adjustment, AdjustRequest},
    issue_service::{process_issue, IssueRequest},
    items::{CreateItemRequest, ItemRepo, TrackingMode},
    receipt_service::{process_receipt, ReceiptRequest},
    transfer_service::{process_transfer, TransferRequest},
};
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
        let mut sorted: Vec<f64> = self.samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
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

        // Threshold checks (warn, don't fail — first run establishes baseline)
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
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for benchmarks");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to inventory DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn create_test_item(pool: &PgPool, tenant_id: &str) -> Uuid {
    let req = CreateItemRequest {
        tenant_id: tenant_id.to_string(),
        sku: format!("BENCH-{}", Uuid::new_v4()),
        name: "Bench Item".to_string(),
        description: None,
        inventory_account_ref: "1200".to_string(),
        cogs_account_ref: "5000".to_string(),
        variance_account_ref: "5010".to_string(),
        uom: None,
        tracking_mode: TrackingMode::None,
    };
    let item = ItemRepo::create(pool, &req).await.expect("create bench item");
    item.id
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_receipt(pool: &PgPool, tenant_id: &str, item_id: Uuid, bench: &mut BenchResult) {
    let warehouse_id = Uuid::new_v4();
    let req = ReceiptRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity: 100,
        unit_cost_minor: 1000,
        currency: "usd".to_string(),
        purchase_order_id: None,
        idempotency_key: format!("bench-recv-{}", Uuid::new_v4()),
        correlation_id: Some("bench".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    };
    let start = Instant::now();
    process_receipt(pool, &req, None).await.expect("receipt");
    bench.record(start.elapsed());
}

async fn bench_issue(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    bench: &mut BenchResult,
) {
    let req = IssueRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity: 1,
        currency: "usd".to_string(),
        source_module: "bench".to_string(),
        source_type: "benchmark".to_string(),
        source_id: format!("BENCH-{}", Uuid::new_v4()),
        source_line_id: None,
        idempotency_key: format!("bench-iss-{}", Uuid::new_v4()),
        correlation_id: Some("bench".to_string()),
        causation_id: None,
        lot_code: None,
        serial_codes: None,
        location_id: None,
        uom_id: None,
    };
    let start = Instant::now();
    process_issue(pool, &req, None).await.expect("issue");
    bench.record(start.elapsed());
}

async fn bench_adjustment(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    warehouse_id: Uuid,
    bench: &mut BenchResult,
) {
    let req = AdjustRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        warehouse_id,
        quantity_delta: 1,
        reason: "benchmark".to_string(),
        allow_negative: false,
        idempotency_key: format!("bench-adj-{}", Uuid::new_v4()),
        correlation_id: Some("bench".to_string()),
        causation_id: None,
        location_id: None,
    };
    let start = Instant::now();
    process_adjustment(pool, &req, None).await.expect("adjust");
    bench.record(start.elapsed());
}

async fn bench_transfer(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    from_warehouse: Uuid,
    bench: &mut BenchResult,
) {
    let to_warehouse = Uuid::new_v4();
    let req = TransferRequest {
        tenant_id: tenant_id.to_string(),
        item_id,
        from_warehouse_id: from_warehouse,
        to_warehouse_id: to_warehouse,
        quantity: 1,
        currency: "usd".to_string(),
        idempotency_key: format!("bench-xfr-{}", Uuid::new_v4()),
        correlation_id: Some("bench".to_string()),
        causation_id: None,
    };
    let start = Instant::now();
    process_transfer(pool, &req, None).await.expect("transfer");
    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = format!("bench-{}", Uuid::new_v4());

    println!("=== Inventory Performance Baseline ===");
    println!("Tenant:     {}", tenant_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    // Create items for each operation type
    let receipt_item = create_test_item(&pool, &tenant_id).await;
    let issue_item = create_test_item(&pool, &tenant_id).await;
    let adjust_item = create_test_item(&pool, &tenant_id).await;
    let transfer_item = create_test_item(&pool, &tenant_id).await;

    // Pre-stock items that need inventory (issue, adjustment, transfer)
    let issue_warehouse = Uuid::new_v4();
    let adjust_warehouse = Uuid::new_v4();
    let transfer_warehouse = Uuid::new_v4();

    // Seed large stock so we don't run out during bench
    for (item, wh) in [
        (issue_item, issue_warehouse),
        (adjust_item, adjust_warehouse),
        (transfer_item, transfer_warehouse),
    ] {
        let seed = ReceiptRequest {
            tenant_id: tenant_id.clone(),
            item_id: item,
            warehouse_id: wh,
            quantity: 1_000_000,
            unit_cost_minor: 1000,
            currency: "usd".to_string(),
            purchase_order_id: None,
            idempotency_key: format!("seed-{}-{}", item, wh),
            correlation_id: Some("bench-seed".to_string()),
            causation_id: None,
            lot_code: None,
            serial_codes: None,
            location_id: None,
            uom_id: None,
        };
        process_receipt(&pool, &seed, None)
            .await
            .expect("seed receipt");
    }

    let mut receipt_bench = BenchResult::new("receipt");
    let mut issue_bench = BenchResult::new("issue");
    let mut adjust_bench = BenchResult::new("adjustment");
    let mut transfer_bench = BenchResult::new("transfer");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        bench_receipt(&pool, &tenant_id, receipt_item, &mut receipt_bench).await;
        bench_issue(
            &pool,
            &tenant_id,
            issue_item,
            issue_warehouse,
            &mut issue_bench,
        )
        .await;
        bench_adjustment(
            &pool,
            &tenant_id,
            adjust_item,
            adjust_warehouse,
            &mut adjust_bench,
        )
        .await;
        bench_transfer(
            &pool,
            &tenant_id,
            transfer_item,
            transfer_warehouse,
            &mut transfer_bench,
        )
        .await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    receipt_bench.report();
    issue_bench.report();
    adjust_bench.report();
    transfer_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");
}
