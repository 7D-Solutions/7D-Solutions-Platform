use chrono::Utc;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgresql://doc_mgmt_user:doc_mgmt_pass@localhost:5455/doc_mgmt_db";

#[derive(Debug, Clone)]
struct Args {
    duration_secs: u64,
}

impl Args {
    fn parse() -> Self {
        let mut duration_secs = 30_u64;
        let mut iter = std::env::args().skip(1);
        while let Some(arg) = iter.next() {
            if arg == "--duration" {
                if let Some(v) = iter.next() {
                    duration_secs = v.parse::<u64>().unwrap_or(30);
                }
            }
        }
        Self { duration_secs }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;

    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    println!("doc-mgmt benchmark starting");
    println!(
        "duration={}s tenant={} db={}",
        args.duration_secs, tenant_id, db_url
    );

    let mut create_times = Vec::new();
    let mut list_times = Vec::new();
    let mut release_times = Vec::new();
    let mut distribute_times = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let mut seq: i64 = 0;

    while Instant::now() < deadline {
        seq += 1;
        let (doc_id, rev_id, doc_number, create_ms) =
            bench_create(&pool, tenant_id, actor_id, seq).await?;
        create_times.push(create_ms);

        let list_ms = bench_list(&pool, tenant_id).await?;
        list_times.push(list_ms);

        let release_ms = bench_release(&pool, tenant_id, actor_id, doc_id, &doc_number).await?;
        release_times.push(release_ms);

        let distribute_ms =
            bench_distribute(&pool, tenant_id, actor_id, doc_id, rev_id, &doc_number).await?;
        distribute_times.push(distribute_ms);
    }

    print_stats("create", &create_times);
    print_stats("list", &list_times);
    print_stats("release", &release_times);
    print_stats("distribute", &distribute_times);

    Ok(())
}

async fn bench_create(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_id: Uuid,
    seq: i64,
) -> Result<(Uuid, Uuid, String, f64), sqlx::Error> {
    let started = Instant::now();
    let mut tx = pool.begin().await?;
    let doc_id = Uuid::new_v4();
    let rev_id = Uuid::new_v4();
    let now = Utc::now();
    let doc_number = format!("BENCH-{}", seq);

    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'spec', 'draft', $5, $6, $6)",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&doc_number)
    .bind(format!("Benchmark Doc {}", seq))
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 1, '{}'::jsonb, 'benchmark create', 'draft', $4, $5)",
    )
    .bind(rev_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
        .bind("document.created")
        .bind("doc_mgmt.events.document.created")
        .bind(serde_json::json!({
            "event_type": "document.created",
            "tenant_id": tenant_id,
            "payload": { "document_id": doc_id, "revision_id": rev_id, "doc_number": doc_number }
        }))
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok((doc_id, rev_id, doc_number, elapsed_ms(started)))
}

async fn bench_list(pool: &PgPool, tenant_id: Uuid) -> Result<f64, sqlx::Error> {
    let started = Instant::now();
    let _rows: Vec<(Uuid, String, String)> = sqlx::query_as(
        "SELECT id, doc_number, status
         FROM documents
         WHERE tenant_id = $1
         ORDER BY created_at DESC
         LIMIT 50",
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(elapsed_ms(started))
}

async fn bench_release(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_id: Uuid,
    doc_id: Uuid,
    doc_number: &str,
) -> Result<f64, sqlx::Error> {
    let started = Instant::now();
    let now = Utc::now();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "UPDATE documents SET status = 'released', updated_at = $1
         WHERE id = $2 AND tenant_id = $3 AND status = 'draft'",
    )
    .bind(now)
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "UPDATE revisions SET status = 'released'
         WHERE document_id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
        .bind("document.released")
        .bind("doc_mgmt.events.document.released")
        .bind(serde_json::json!({
            "event_type": "document.released",
            "tenant_id": tenant_id,
            "payload": {
                "document_id": doc_id,
                "doc_number": doc_number,
                "released_by": actor_id
            }
        }))
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(elapsed_ms(started))
}

async fn bench_distribute(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_id: Uuid,
    doc_id: Uuid,
    rev_id: Uuid,
    doc_number: &str,
) -> Result<f64, sqlx::Error> {
    let started = Instant::now();
    let now = Utc::now();
    let dist_id = Uuid::new_v4();
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO document_distributions
         (id, tenant_id, document_id, revision_id, recipient_ref, channel, template_key, payload_json,
          status, requested_by, requested_at, idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'ops@fireproof.test', 'email', 'doc_distribution_notice', '{}'::jsonb,
                 'pending', $5, $6, $7, $6, $6)",
    )
    .bind(dist_id)
    .bind(tenant_id)
    .bind(doc_id)
    .bind(rev_id)
    .bind(actor_id)
    .bind(now)
    .bind(format!("bench-dist-{}", Uuid::new_v4()))
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO document_distribution_status_log
         (distribution_id, tenant_id, previous_status, new_status, idempotency_key, payload_json, changed_by, changed_at)
         VALUES ($1, $2, NULL, 'pending', $3, '{}'::jsonb, $4, $5)",
    )
    .bind(dist_id)
    .bind(tenant_id)
    .bind(format!("bench-log-{}", Uuid::new_v4()))
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
        .bind("document.distribution.requested")
        .bind("doc_mgmt.events.document.distribution.requested")
        .bind(serde_json::json!({
            "event_type": "document.distribution.requested",
            "tenant_id": tenant_id,
            "payload": {
                "distribution_id": dist_id,
                "document_id": doc_id,
                "revision_id": rev_id,
                "doc_number": doc_number,
                "channel": "email"
            }
        }))
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(elapsed_ms(started))
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn print_stats(name: &str, values: &[f64]) {
    if values.is_empty() {
        println!("{name}: no samples");
        return;
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite values"));
    let count = sorted.len();
    let p50 = percentile(&sorted, 50.0);
    let p95 = percentile(&sorted, 95.0);
    let p99 = percentile(&sorted, 99.0);
    let avg = sorted.iter().sum::<f64>() / count as f64;

    println!("{name}: n={count} avg={avg:.2}ms p50={p50:.2}ms p95={p95:.2}ms p99={p99:.2}ms");
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    let max_idx = (sorted.len() - 1) as f64;
    let idx = ((pct / 100.0) * max_idx).round() as usize;
    sorted[idx]
}
