//! Performance baseline benchmark for core timekeeping operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for time entry creation and approval workflow flows.
//!
//! Usage:
//!   cargo run --manifest-path modules/timekeeping/Cargo.toml --bin bench -- --duration 30
//!   cargo run --manifest-path modules/timekeeping/Cargo.toml --bin bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL — timekeeping database connection string

use chrono::{NaiveDate, Utc};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use timekeeping::domain::{
    approvals::{
        models::{ReviewApprovalRequest, SubmitApprovalRequest},
        service as approval_svc,
    },
    employees::{
        models::CreateEmployeeRequest,
        service::EmployeeRepo,
    },
    entries::{
        models::CreateEntryRequest,
        service as entry_svc,
    },
};
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
        .expect("Failed to connect to timekeeping DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn create_test_employee(pool: &PgPool, app_id: &str) -> Uuid {
    let req = CreateEmployeeRequest {
        app_id: app_id.to_string(),
        employee_code: format!("BENCH-{}", Uuid::new_v4()),
        first_name: "Bench".to_string(),
        last_name: "Worker".to_string(),
        email: None,
        department: None,
        external_payroll_id: None,
        hourly_rate_minor: Some(5000),
        currency: Some("USD".to_string()),
    };
    let emp = EmployeeRepo::create(pool, &req)
        .await
        .expect("create bench employee");
    emp.id
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_create_entry(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    bench: &mut BenchResult,
) {
    let req = CreateEntryRequest {
        app_id: app_id.to_string(),
        employee_id,
        project_id: None,
        task_id: None,
        work_date: NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date")
            + chrono::Duration::days(bench.samples.len() as i64),
        minutes: 480,
        description: Some("Bench work".to_string()),
        created_by: Some(employee_id),
    };
    let start = Instant::now();
    entry_svc::create_entry(pool, &req, None)
        .await
        .expect("create entry");
    bench.record(start.elapsed());
}

async fn bench_approval_flow(
    pool: &PgPool,
    app_id: &str,
    employee_id: Uuid,
    reviewer_id: Uuid,
    bench: &mut BenchResult,
) {
    let offset = bench.samples.len() as i64;
    let period_start =
        NaiveDate::from_ymd_opt(2027, 1, 1).expect("valid date")
            + chrono::Duration::days(offset * 7);
    let period_end = period_start + chrono::Duration::days(6);

    // Create an entry in the period first
    let entry_req = CreateEntryRequest {
        app_id: app_id.to_string(),
        employee_id,
        project_id: None,
        task_id: None,
        work_date: period_start,
        minutes: 480,
        description: Some("Approval bench".to_string()),
        created_by: Some(employee_id),
    };
    entry_svc::create_entry(pool, &entry_req, None)
        .await
        .expect("create entry for approval bench");

    let submit_req = SubmitApprovalRequest {
        app_id: app_id.to_string(),
        employee_id,
        period_start,
        period_end,
        actor_id: employee_id,
    };

    let start = Instant::now();

    let approval = approval_svc::submit(pool, &submit_req)
        .await
        .expect("submit approval");

    let approve_req = ReviewApprovalRequest {
        app_id: app_id.to_string(),
        approval_id: approval.id,
        actor_id: reviewer_id,
        notes: None,
    };
    approval_svc::approve(pool, &approve_req)
        .await
        .expect("approve");

    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let app_id = format!("bench-{}", Uuid::new_v4());

    println!("=== Timekeeping Performance Baseline ===");
    println!("App ID:     {}", app_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    let entry_employee = create_test_employee(&pool, &app_id).await;
    let approval_employee = create_test_employee(&pool, &app_id).await;
    let reviewer = create_test_employee(&pool, &app_id).await;

    let mut entry_bench = BenchResult::new("create_entry");
    let mut approval_bench = BenchResult::new("submit+approve");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        bench_create_entry(&pool, &app_id, entry_employee, &mut entry_bench).await;
        bench_approval_flow(
            &pool,
            &app_id,
            approval_employee,
            reviewer,
            &mut approval_bench,
        )
        .await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    entry_bench.report();
    approval_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");
}
