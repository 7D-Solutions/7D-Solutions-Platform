//! Performance baseline benchmark for core maintenance operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for work order create, transition, meter reading, and plan assignment.
//!
//! Usage:
//!   cargo run --manifest-path modules/maintenance/Cargo.toml --bin bench -- --duration 30
//!   cargo run --manifest-path modules/maintenance/Cargo.toml --bin bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL — maintenance database connection string

use chrono::Utc;
use maintenance_rs::domain::{
    assets::{AssetRepo, CreateAssetRequest},
    meters::{CreateMeterTypeRequest, MeterReadingRepo, MeterTypeRepo, RecordReadingRequest},
    plans::{AssignPlanRequest, AssignmentRepo, CreatePlanRequest, PlanRepo},
    work_orders::{
        CreateWorkOrderRequest, TransitionRequest, WorkOrderRepo,
    },
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
        .expect("Failed to connect to maintenance DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

async fn create_test_asset(pool: &PgPool, tenant_id: &str) -> Uuid {
    let req = CreateAssetRequest {
        tenant_id: tenant_id.to_string(),
        asset_tag: format!("BENCH-{}", Uuid::new_v4()),
        name: "Bench Asset".to_string(),
        description: None,
        asset_type: "equipment".to_string(),
        location: None,
        department: None,
        responsible_person: None,
        serial_number: None,
        fixed_asset_ref: None,
        metadata: None,
        maintenance_schedule: None,
        idempotency_key: None,
    };
    let asset = AssetRepo::create(pool, &req).await.expect("create bench asset");
    asset.id
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_wo_create(
    pool: &PgPool,
    tenant_id: &str,
    asset_id: Uuid,
    bench: &mut BenchResult,
) {
    let req = CreateWorkOrderRequest {
        tenant_id: tenant_id.to_string(),
        asset_id,
        plan_assignment_id: None,
        title: format!("Bench WO {}", Uuid::new_v4()),
        description: None,
        wo_type: "corrective".to_string(),
        priority: Some("medium".to_string()),
        assigned_to: None,
        scheduled_date: None,
        checklist: None,
        notes: None,
    };
    let start = Instant::now();
    let wo = WorkOrderRepo::create(pool, &req).await.expect("create wo");
    bench.record(start.elapsed());

    // Transition draft → scheduled so the next iteration can also create fresh
    let _ = WorkOrderRepo::transition(
        pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tenant_id.to_string(),
            status: "scheduled".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await;
}

async fn bench_wo_transition(
    pool: &PgPool,
    tenant_id: &str,
    asset_id: Uuid,
    bench: &mut BenchResult,
) {
    // Create a fresh WO and transition it through: draft → scheduled → in_progress
    let req = CreateWorkOrderRequest {
        tenant_id: tenant_id.to_string(),
        asset_id,
        plan_assignment_id: None,
        title: format!("Bench Transition {}", Uuid::new_v4()),
        description: None,
        wo_type: "corrective".to_string(),
        priority: Some("medium".to_string()),
        assigned_to: None,
        scheduled_date: None,
        checklist: None,
        notes: None,
    };
    let wo = WorkOrderRepo::create(pool, &req).await.expect("create wo for transition");

    // draft → scheduled
    WorkOrderRepo::transition(
        pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tenant_id.to_string(),
            status: "scheduled".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("schedule");

    // Measure scheduled → in_progress
    let start = Instant::now();
    WorkOrderRepo::transition(
        pool,
        wo.id,
        &TransitionRequest {
            tenant_id: tenant_id.to_string(),
            status: "in_progress".to_string(),
            completed_at: None,
            downtime_minutes: None,
            closed_at: None,
            notes: None,
        },
    )
    .await
    .expect("in_progress");
    bench.record(start.elapsed());
}

async fn bench_meter_reading(
    pool: &PgPool,
    tenant_id: &str,
    asset_id: Uuid,
    meter_type_id: Uuid,
    counter: &mut i64,
    bench: &mut BenchResult,
) {
    *counter += 1;
    let req = RecordReadingRequest {
        tenant_id: tenant_id.to_string(),
        meter_type_id,
        reading_value: *counter,
        recorded_at: None,
        recorded_by: Some("bench".to_string()),
    };
    let start = Instant::now();
    MeterReadingRepo::record(pool, asset_id, &req)
        .await
        .expect("record reading");
    bench.record(start.elapsed());
}

async fn bench_plan_assign(
    pool: &PgPool,
    tenant_id: &str,
    plan_id: Uuid,
    bench: &mut BenchResult,
) {
    // Each iteration needs a fresh asset (unique assignment per plan+asset)
    let asset_id = create_test_asset(pool, tenant_id).await;
    let req = AssignPlanRequest {
        tenant_id: tenant_id.to_string(),
        asset_id,
    };
    let start = Instant::now();
    AssignmentRepo::assign(pool, plan_id, &req)
        .await
        .expect("assign plan");
    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = format!("bench-{}", Uuid::new_v4());

    println!("=== Maintenance Performance Baseline ===");
    println!("Tenant:     {}", tenant_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    // Setup: create assets for each benchmark
    let wo_create_asset = create_test_asset(&pool, &tenant_id).await;
    let wo_transition_asset = create_test_asset(&pool, &tenant_id).await;
    let meter_asset = create_test_asset(&pool, &tenant_id).await;

    // Create a meter type for meter reading benchmark
    let meter_type = MeterTypeRepo::create(
        &pool,
        &CreateMeterTypeRequest {
            tenant_id: tenant_id.clone(),
            name: format!("bench-hours-{}", Uuid::new_v4()),
            unit_label: "hours".to_string(),
            rollover_value: None,
        },
    )
    .await
    .expect("create meter type");

    // Create a maintenance plan for plan assignment benchmark
    let plan = PlanRepo::create(
        &pool,
        &CreatePlanRequest {
            tenant_id: tenant_id.clone(),
            name: format!("Bench Plan {}", Uuid::new_v4()),
            description: None,
            asset_type_filter: None,
            schedule_type: "calendar".to_string(),
            calendar_interval_days: Some(30),
            meter_type_id: None,
            meter_interval: None,
            priority: Some("medium".to_string()),
            estimated_duration_minutes: None,
            estimated_cost_minor: None,
            task_checklist: None,
        },
    )
    .await
    .expect("create plan");

    let mut wo_create_bench = BenchResult::new("wo_create");
    let mut wo_transition_bench = BenchResult::new("wo_transition");
    let mut meter_bench = BenchResult::new("meter_reading");
    let mut plan_assign_bench = BenchResult::new("plan_assign");

    let mut meter_counter: i64 = 0;

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        bench_wo_create(&pool, &tenant_id, wo_create_asset, &mut wo_create_bench).await;
        bench_wo_transition(&pool, &tenant_id, wo_transition_asset, &mut wo_transition_bench).await;
        bench_meter_reading(
            &pool,
            &tenant_id,
            meter_asset,
            meter_type.id,
            &mut meter_counter,
            &mut meter_bench,
        )
        .await;
        bench_plan_assign(&pool, &tenant_id, plan.id, &mut plan_assign_bench).await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    wo_create_bench.report();
    wo_transition_bench.report();
    meter_bench.report();
    plan_assign_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");
}
