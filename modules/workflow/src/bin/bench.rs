//! Performance baseline benchmark for workflow definition and instance operations.
//!
//! Runs against real Postgres. Measures latency (p50/p95/p99) and throughput
//! for create_definition and start_instance + advance operations.
//!
//! Usage:
//!   cargo run --manifest-path modules/workflow/Cargo.toml --bin bench -- --duration 30
//!   cargo run --manifest-path modules/workflow/Cargo.toml --bin bench -- --duration 10 --iterations 50
//!
//! Environment:
//!   DATABASE_URL - workflow database connection string

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
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("Failed to connect to workflow DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// -- Benchmark functions -----------------------------------------------------

async fn bench_create_definition(pool: &PgPool, tenant_id: &str, bench: &mut BenchResult) -> Uuid {
    let def_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let name = format!("bench-def-{}", &def_id.to_string()[..8]);
    let now = Utc::now();

    let steps = serde_json::json!([
        {"step_id": "draft", "name": "Draft", "step_type": "action", "position": 1},
        {"step_id": "review", "name": "Review", "step_type": "approval", "position": 2},
        {"step_id": "approved", "name": "Approved", "step_type": "terminal", "position": 3}
    ]);

    let start = Instant::now();

    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        r#"
        INSERT INTO workflow_definitions
            (id, tenant_id, name, description, steps, initial_step_id)
        VALUES ($1, $2, $3, 'bench definition', $4, 'draft')
        "#,
    )
    .bind(def_id)
    .bind(tenant_id)
    .bind(&name)
    .bind(&steps)
    .execute(&mut *tx)
    .await
    .expect("insert definition");

    let payload = serde_json::json!({
        "event_id": event_id.to_string(),
        "event_type": "workflow.events.definition.created",
        "occurred_at": now.to_rfc3339(),
        "tenant_id": tenant_id,
        "source_module": "workflow",
        "source_version": "0.1.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {
            "definition_id": def_id.to_string(),
            "tenant_id": tenant_id,
            "name": name,
            "version": 1,
            "initial_step_id": "draft",
            "step_count": 3
        }
    });

    sqlx::query(
        r#"
        INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload)
        VALUES ($1, $2, 'workflow_definition', $3, $4)
        "#,
    )
    .bind(event_id)
    .bind("workflow.events.definition.created")
    .bind(def_id.to_string())
    .bind(&payload)
    .execute(&mut *tx)
    .await
    .expect("insert outbox");

    tx.commit().await.expect("commit");

    bench.record(start.elapsed());
    def_id
}

async fn bench_start_and_advance(
    pool: &PgPool,
    tenant_id: &str,
    definition_id: Uuid,
    bench: &mut BenchResult,
) {
    let instance_id = Uuid::new_v4();
    let start_event_id = Uuid::new_v4();
    let advance_event_id = Uuid::new_v4();
    let transition_id = Uuid::new_v4();
    let now = Utc::now();

    let start = Instant::now();

    let mut tx = pool.begin().await.expect("begin tx");

    // Start instance
    sqlx::query(
        r#"
        INSERT INTO workflow_instances
            (id, tenant_id, definition_id, entity_type, entity_id,
             current_step_id, status, context)
        VALUES ($1, $2, $3, 'bench_entity', $4, 'draft', 'active', '{}')
        "#,
    )
    .bind(instance_id)
    .bind(tenant_id)
    .bind(definition_id)
    .bind(format!("bench-{}", &instance_id.to_string()[..8]))
    .execute(&mut *tx)
    .await
    .expect("insert instance");

    // Initial transition
    sqlx::query(
        r#"
        INSERT INTO workflow_transitions
            (id, tenant_id, instance_id, from_step_id, to_step_id, action)
        VALUES ($1, $2, $3, '__start__', 'draft', 'start')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(instance_id)
    .execute(&mut *tx)
    .await
    .expect("insert initial transition");

    // Start event outbox
    let start_payload = serde_json::json!({
        "event_id": start_event_id.to_string(),
        "event_type": "workflow.events.instance.started",
        "occurred_at": now.to_rfc3339(),
        "tenant_id": tenant_id,
        "source_module": "workflow",
        "source_version": "0.1.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {
            "instance_id": instance_id.to_string(),
            "tenant_id": tenant_id,
            "definition_id": definition_id.to_string(),
            "entity_type": "bench_entity",
            "entity_id": format!("bench-{}", &instance_id.to_string()[..8]),
            "initial_step_id": "draft"
        }
    });

    sqlx::query(
        r#"
        INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload)
        VALUES ($1, $2, 'workflow_instance', $3, $4)
        "#,
    )
    .bind(start_event_id)
    .bind("workflow.events.instance.started")
    .bind(instance_id.to_string())
    .bind(&start_payload)
    .execute(&mut *tx)
    .await
    .expect("insert start event");

    tx.commit().await.expect("commit start");

    // Advance: draft → review
    let mut tx2 = pool.begin().await.expect("begin tx2");

    sqlx::query(
        r#"
        UPDATE workflow_instances
        SET current_step_id = 'review', updated_at = now()
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(instance_id)
    .bind(tenant_id)
    .execute(&mut *tx2)
    .await
    .expect("advance instance");

    sqlx::query(
        r#"
        INSERT INTO workflow_transitions
            (id, tenant_id, instance_id, from_step_id, to_step_id, action)
        VALUES ($1, $2, $3, 'draft', 'review', 'submit')
        "#,
    )
    .bind(transition_id)
    .bind(tenant_id)
    .bind(instance_id)
    .execute(&mut *tx2)
    .await
    .expect("insert advance transition");

    let advance_payload = serde_json::json!({
        "event_id": advance_event_id.to_string(),
        "event_type": "workflow.events.instance.advanced",
        "occurred_at": Utc::now().to_rfc3339(),
        "tenant_id": tenant_id,
        "source_module": "workflow",
        "source_version": "0.1.0",
        "schema_version": "1.0.0",
        "replay_safe": true,
        "mutation_class": "DATA_MUTATION",
        "payload": {
            "instance_id": instance_id.to_string(),
            "tenant_id": tenant_id,
            "transition_id": transition_id.to_string(),
            "from_step_id": "draft",
            "to_step_id": "review",
            "action": "submit"
        }
    });

    sqlx::query(
        r#"
        INSERT INTO events_outbox (event_id, event_type, aggregate_type, aggregate_id, payload)
        VALUES ($1, $2, 'workflow_instance', $3, $4)
        "#,
    )
    .bind(advance_event_id)
    .bind("workflow.events.instance.advanced")
    .bind(instance_id.to_string())
    .bind(&advance_payload)
    .execute(&mut *tx2)
    .await
    .expect("insert advance event");

    tx2.commit().await.expect("commit advance");

    bench.record(start.elapsed());
}

// -- Main --------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let args = parse_args();
    let pool = setup_pool().await;
    let tenant_id = format!("bench-{}", Uuid::new_v4());

    println!("=== Workflow Performance Baseline ===");
    println!("Tenant ID:  {}", tenant_id);
    println!("Duration:   {}s", args.duration_secs);
    if let Some(n) = args.iterations {
        println!("Iterations: {}", n);
    }
    println!("Started:    {}", Utc::now());
    println!();

    let mut def_bench = BenchResult::new("create_definition");
    let mut instance_bench = BenchResult::new("start_and_advance_instance");

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let max_iter = args.iterations.unwrap_or(usize::MAX);
    let mut iter = 0;

    while Instant::now() < deadline && iter < max_iter {
        let def_id = bench_create_definition(&pool, &tenant_id, &mut def_bench).await;
        bench_start_and_advance(&pool, &tenant_id, def_id, &mut instance_bench).await;
        iter += 1;
    }

    println!("Results ({} iterations completed):", iter);
    println!();
    def_bench.report();
    instance_bench.report();
    println!();
    println!("Finished: {}", Utc::now());
    println!();
    println!("Thresholds: p95 < 100ms per operation (warn-only)");
    println!("To update baselines, re-run and compare.");
}
