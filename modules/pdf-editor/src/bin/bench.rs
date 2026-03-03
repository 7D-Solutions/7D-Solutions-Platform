//! Performance baseline benchmark for core pdf-editor operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for template creation and form submission (create draft → submit).
//!
//! Usage:
//!   cargo run --manifest-path modules/pdf-editor/Cargo.toml --bin bench -- --duration 30
//!
//! Environment:
//!   DATABASE_URL — pdf-editor database connection string

use pdf_editor_rs::domain::forms::{
    CreateFieldRequest, CreateTemplateRequest, FieldRepo, TemplateRepo,
};
use pdf_editor_rs::domain::submissions::{CreateSubmissionRequest, SubmissionRepo};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ── CLI args ─────────────────────────────────────────────────────────

struct Args {
    duration_secs: u64,
}

fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();
    let mut duration_secs = 30u64;

    let mut i = 1;
    while i < args.len() {
        if args[i] == "--duration" {
            i += 1;
            if i < args.len() {
                duration_secs = args[i].parse().expect("--duration must be a number");
            }
        }
        i += 1;
    }
    Args { duration_secs }
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
        .expect("Failed to connect to pdf-editor DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// ── Benchmark functions ──────────────────────────────────────────────

async fn bench_template_create(pool: &PgPool, tenant_id: &str, bench: &mut BenchResult) -> Uuid {
    let req = CreateTemplateRequest {
        tenant_id: tenant_id.to_string(),
        name: format!("Bench Template {}", Uuid::new_v4()),
        description: Some("Benchmark template".to_string()),
        created_by: "bench-user".to_string(),
    };
    let start = Instant::now();
    let tmpl = TemplateRepo::create(pool, &req)
        .await
        .expect("template create");
    bench.record(start.elapsed());
    tmpl.id
}

async fn bench_submit_flow(
    pool: &PgPool,
    tenant_id: &str,
    template_id: Uuid,
    bench: &mut BenchResult,
) {
    // Create draft → submit (the full happy path including validation + event enqueue)
    let create_req = CreateSubmissionRequest {
        tenant_id: tenant_id.to_string(),
        template_id,
        submitted_by: "bench-user".to_string(),
        field_data: Some(serde_json::json!({
            "company_name": "Acme Corp",
            "inspection_date": "2026-03-01"
        })),
    };

    let start = Instant::now();
    let sub = SubmissionRepo::create(pool, &create_req)
        .await
        .expect("submission create");
    SubmissionRepo::submit(pool, sub.id, tenant_id)
        .await
        .expect("submission submit");
    bench.record(start.elapsed());
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = format!("bench-{}", Uuid::new_v4());

    println!("=== PDF Editor Performance Baseline ===");
    println!("Tenant:   {}", tenant_id);
    println!("Duration: {}s", args.duration_secs);
    println!("Started:  {}", chrono::Utc::now());
    println!();

    // Pre-create a template with two fields for the submit flow bench
    let tmpl_req = CreateTemplateRequest {
        tenant_id: tenant_id.clone(),
        name: "Bench Submit Template".to_string(),
        description: None,
        created_by: "bench-user".to_string(),
    };
    let submit_tmpl = TemplateRepo::create(&pool, &tmpl_req)
        .await
        .expect("create submit template");

    for (key, label, ft) in [
        ("company_name", "Company Name", "text"),
        ("inspection_date", "Inspection Date", "date"),
    ] {
        FieldRepo::create(
            &pool,
            submit_tmpl.id,
            &tenant_id,
            &CreateFieldRequest {
                field_key: key.to_string(),
                field_label: label.to_string(),
                field_type: ft.to_string(),
                validation_rules: None,
                pdf_position: None,
            },
        )
        .await
        .expect("create bench field");
    }

    let mut template_bench = BenchResult::new("template_create");
    let mut submit_bench = BenchResult::new("submit_flow");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let mut iter = 0;

    while Instant::now() < deadline {
        bench_template_create(&pool, &tenant_id, &mut template_bench).await;
        bench_submit_flow(&pool, &tenant_id, submit_tmpl.id, &mut submit_bench).await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    template_bench.report();
    submit_bench.report();
    println!();
    println!("Finished: {}", chrono::Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");

    // Cleanup
    sqlx::query("DELETE FROM form_submissions WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM form_fields WHERE template_id IN (SELECT id FROM form_templates WHERE tenant_id = $1)")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM form_templates WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
    sqlx::query("DELETE FROM events_outbox WHERE tenant_id = $1")
        .bind(&tenant_id)
        .execute(&pool)
        .await
        .ok();
}
