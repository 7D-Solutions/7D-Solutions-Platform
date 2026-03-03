//! Performance baseline benchmark for party CRUD and contact management.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for create_company and create_contact operations.
//!
//! Usage:
//!   cargo run --bin party_bench -- --duration 30
//!   cargo run --bin party_bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL - party database connection string

use chrono::Utc;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::time::{Duration, Instant};
use uuid::Uuid;

// -- CLI args ----------------------------------------------------------------

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

// -- Benchmark harness -------------------------------------------------------

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

// -- Setup -------------------------------------------------------------------

async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for benchmarks");
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to party DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// -- Benchmark functions -----------------------------------------------------

async fn bench_create_company(pool: &PgPool, app_id: &str, bench: &mut BenchResult) {
    let party_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let now = Utc::now();
    let name = format!("Bench-Co-{}", &party_id.to_string()[..8]);

    let start = Instant::now();

    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        r#"
        INSERT INTO party_parties (
            id, app_id, party_type, status, display_name, email,
            country, created_at, updated_at
        )
        VALUES ($1, $2, 'company', 'active', $3, $4, 'US', $5, $5)
        "#,
    )
    .bind(party_id)
    .bind(app_id)
    .bind(&name)
    .bind(format!("{}@bench.test", &party_id.to_string()[..8]))
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert party");

    sqlx::query(
        r#"
        INSERT INTO party_companies (
            party_id, legal_name, currency, created_at, updated_at
        )
        VALUES ($1, $2, 'usd', $3, $3)
        "#,
    )
    .bind(party_id)
    .bind(format!("{} LLC", name))
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert company");

    let payload = serde_json::json!({
        "event_id": event_id.to_string(),
        "event_type": "party.created",
        "occurred_at": now.to_rfc3339(),
        "tenant_id": app_id,
        "source_module": "party",
        "source_version": "0.1.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {
            "party_id": party_id.to_string(),
            "app_id": app_id,
            "party_type": "company",
            "display_name": name,
            "created_at": now.to_rfc3339()
        }
    });

    sqlx::query(
        r#"
        INSERT INTO party_outbox (
            event_id, event_type, aggregate_type, aggregate_id,
            app_id, payload, schema_version
        )
        VALUES ($1, $2, 'party', $3, $4, $5, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind("party.created")
    .bind(party_id.to_string())
    .bind(app_id)
    .bind(&payload)
    .execute(&mut *tx)
    .await
    .expect("insert outbox");

    tx.commit().await.expect("commit");

    bench.record(start.elapsed());
}

async fn bench_create_contact(
    pool: &PgPool,
    app_id: &str,
    party_id: Uuid,
    bench: &mut BenchResult,
) {
    let contact_id = Uuid::new_v4();
    let now = Utc::now();

    let start = Instant::now();

    sqlx::query(
        r#"
        INSERT INTO party_contacts (
            id, party_id, app_id, first_name, last_name, email,
            role, is_primary, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'engineer', false, $7, $7)
        "#,
    )
    .bind(contact_id)
    .bind(party_id)
    .bind(app_id)
    .bind(format!("First-{}", &contact_id.to_string()[..6]))
    .bind(format!("Last-{}", &contact_id.to_string()[..6]))
    .bind(format!("{}@bench.test", &contact_id.to_string()[..8]))
    .bind(now)
    .execute(pool)
    .await
    .expect("insert contact");

    bench.record(start.elapsed());
}

// -- Main --------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let app_id = format!("bench-{}", Uuid::new_v4());

    println!("=== Party Performance Baseline ===");
    println!("App ID:     {}", app_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    // Create a seed party for contact benchmarks
    let seed_party_id = Uuid::new_v4();
    let now = Utc::now();
    sqlx::query(
        r#"
        INSERT INTO party_parties (
            id, app_id, party_type, status, display_name,
            country, created_at, updated_at
        )
        VALUES ($1, $2, 'company', 'active', 'Bench Seed Corp', 'US', $3, $3)
        "#,
    )
    .bind(seed_party_id)
    .bind(&app_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("seed party");

    sqlx::query(
        "INSERT INTO party_companies (party_id, legal_name, currency, created_at, updated_at) \
         VALUES ($1, 'Bench Seed Corp LLC', 'usd', $2, $2)",
    )
    .bind(seed_party_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("seed company");

    let mut company_bench = BenchResult::new("create_company");
    let mut contact_bench = BenchResult::new("create_contact");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        bench_create_company(&pool, &app_id, &mut company_bench).await;
        bench_create_contact(&pool, &app_id, seed_party_id, &mut contact_bench).await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    company_bench.report();
    contact_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");
}
