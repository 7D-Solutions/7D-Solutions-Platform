//! Performance baseline benchmark for core AP operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for create_vendor, create_po, create_bill, and approve_bill flows.
//!
//! Usage:
//!   cargo run --bin bench -- --duration 30
//!   cargo run --bin bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL — AP database connection string

use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

use ap::domain::bills::approve::approve_bill;
use ap::domain::bills::service::create_bill;
use ap::domain::bills::{ApproveBillRequest, CreateBillLineRequest, CreateBillRequest};
use ap::domain::po::service::create_po;
use ap::domain::po::{CreatePoLineRequest, CreatePoRequest};
use ap::domain::tax::ZeroTaxProvider;
use ap::domain::vendors::service::create_vendor;
use ap::domain::vendors::CreateVendorRequest;

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
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ap_user:ap_pass@localhost:5443/ap_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to AP database");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run AP migrations");

    pool
}

async fn cleanup(pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM three_way_match WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM ap_tax_snapshots WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM events_outbox WHERE aggregate_type IN ('bill','vendor','po') \
         AND aggregate_id IN ( \
           SELECT bill_id::TEXT FROM vendor_bills WHERE tenant_id = $1 \
           UNION ALL SELECT vendor_id::TEXT FROM vendors WHERE tenant_id = $1 \
           UNION ALL SELECT po_id::TEXT FROM purchase_orders WHERE tenant_id = $1 \
         )",
        "DELETE FROM bill_lines WHERE bill_id IN \
         (SELECT bill_id FROM vendor_bills WHERE tenant_id = $1)",
        "DELETE FROM vendor_bills WHERE tenant_id = $1",
        "DELETE FROM po_status WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM po_lines WHERE po_id IN \
         (SELECT po_id FROM purchase_orders WHERE tenant_id = $1)",
        "DELETE FROM purchase_orders WHERE tenant_id = $1",
        "DELETE FROM vendors WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_create_vendor(pool: &PgPool, tenant_id: &str, bench: &mut BenchResult) -> Uuid {
    let req = CreateVendorRequest {
        name: format!("Bench Vendor {}", Uuid::new_v4()),
        tax_id: Some("12-3456789".to_string()),
        currency: "USD".to_string(),
        payment_terms_days: 30,
        payment_method: Some("ach".to_string()),
        remittance_email: Some("ap@bench.test".to_string()),
        party_id: None,
    };
    let start = Instant::now();
    let vendor = create_vendor(pool, tenant_id, &req, format!("bench-{}", Uuid::new_v4()))
        .await
        .expect("create_vendor");
    bench.record(start.elapsed());
    vendor.vendor_id
}

async fn bench_create_po(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    bench: &mut BenchResult,
) {
    let req = CreatePoRequest {
        vendor_id,
        currency: "USD".to_string(),
        created_by: "bench".to_string(),
        expected_delivery_date: None,
        lines: vec![CreatePoLineRequest {
            item_id: None,
            description: Some("Bench item".to_string()),
            quantity: 10.0,
            unit_of_measure: "each".to_string(),
            unit_price_minor: 5000,
            gl_account_code: "6100".to_string(),
        }],
    };
    let start = Instant::now();
    create_po(pool, tenant_id, &req, format!("bench-{}", Uuid::new_v4()))
        .await
        .expect("create_po");
    bench.record(start.elapsed());
}

async fn bench_create_bill(
    pool: &PgPool,
    tenant_id: &str,
    vendor_id: Uuid,
    bench: &mut BenchResult,
) -> Uuid {
    let req = CreateBillRequest {
        vendor_id,
        vendor_invoice_ref: format!("BENCH-INV-{}", Uuid::new_v4()),
        currency: "USD".to_string(),
        invoice_date: Utc::now(),
        due_date: None,
        tax_minor: None,
        entered_by: "bench".to_string(),
        fx_rate_id: None,
        lines: vec![CreateBillLineRequest {
            description: Some("Bench service".to_string()),
            item_id: None,
            quantity: 5.0,
            unit_price_minor: 10000,
            gl_account_code: Some("6100".to_string()),
            po_line_id: None,
        }],
    };
    let start = Instant::now();
    let result = create_bill(pool, tenant_id, &req, format!("bench-{}", Uuid::new_v4()))
        .await
        .expect("create_bill");
    bench.record(start.elapsed());
    result.bill.bill_id
}

async fn bench_approve_bill(
    pool: &PgPool,
    tenant_id: &str,
    bill_id: Uuid,
    bench: &mut BenchResult,
) {
    let req = ApproveBillRequest {
        approved_by: "bench-approver".to_string(),
        override_reason: Some("bench run — no PO match".to_string()),
    };
    let start = Instant::now();
    approve_bill(
        pool,
        &ZeroTaxProvider,
        tenant_id,
        bill_id,
        &req,
        format!("bench-{}", Uuid::new_v4()),
    )
    .await
    .expect("approve_bill");
    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = format!("bench-{}", Uuid::new_v4());

    println!("=== AP Performance Baseline ===");
    println!("Tenant:     {}", tenant_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    let mut vendor_bench = BenchResult::new("create_vendor");
    let mut po_bench = BenchResult::new("create_po");
    let mut bill_bench = BenchResult::new("create_bill");
    let mut approve_bench = BenchResult::new("approve_bill");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        // Each iteration: vendor → PO → bill → approve
        let vendor_id = bench_create_vendor(&pool, &tenant_id, &mut vendor_bench).await;
        bench_create_po(&pool, &tenant_id, vendor_id, &mut po_bench).await;
        let bill_id = bench_create_bill(&pool, &tenant_id, vendor_id, &mut bill_bench).await;
        bench_approve_bill(&pool, &tenant_id, bill_id, &mut approve_bench).await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    vendor_bench.report();
    po_bench.report();
    bill_bench.report();
    approve_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");

    // Cleanup bench data
    cleanup(&pool, &tenant_id).await;
}
