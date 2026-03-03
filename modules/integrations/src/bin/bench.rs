use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db";

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
    let db_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;

    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let app_id = format!("bench-app-{}", Uuid::new_v4());

    println!("integrations benchmark starting");
    println!(
        "duration={}s app_id={} db={}",
        args.duration_secs, app_id, db_url
    );

    let mut create_ref_times = Vec::new();
    let mut webhook_ingest_times = Vec::new();
    let mut webhook_duplicate_times = Vec::new();
    let mut ref_lookup_times = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);

    while Instant::now() < deadline {
        let create_ms = bench_create_external_ref(&pool, &app_id).await?;
        create_ref_times.push(create_ms);

        let ingest_ms = bench_webhook_ingest(&pool, &app_id).await?;
        webhook_ingest_times.push(ingest_ms);

        let dup_ms = bench_webhook_duplicate(&pool, &app_id).await?;
        webhook_duplicate_times.push(dup_ms);

        let lookup_ms = bench_ref_lookup(&pool, &app_id).await?;
        ref_lookup_times.push(lookup_ms);
    }

    print_stats("create_external_ref", &create_ref_times);
    print_stats("webhook_ingest", &webhook_ingest_times);
    print_stats("webhook_duplicate", &webhook_duplicate_times);
    print_stats("ref_lookup_by_entity", &ref_lookup_times);

    // Cleanup benchmark data
    cleanup(&pool, &app_id).await;

    Ok(())
}

async fn bench_create_external_ref(
    pool: &sqlx::PgPool,
    app_id: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let ext_id = format!("bench-ext-{}", Uuid::new_v4());
    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, label, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO UPDATE SET
            label = EXCLUDED.label, updated_at = NOW()
        "#,
    )
    .bind(app_id)
    .bind("invoice")
    .bind(format!("inv-{}", Uuid::new_v4()))
    .bind("stripe")
    .bind(&ext_id)
    .bind("Bench Label")
    .execute(&mut *tx)
    .await?;

    // Outbox event (mirrors real code path)
    let payload = serde_json::json!({
        "ref_id": 0,
        "app_id": app_id,
        "entity_type": "invoice",
        "entity_id": "inv-bench",
        "system": "stripe",
        "external_id": ext_id,
        "created_at": chrono::Utc::now()
    });

    sqlx::query(
        r#"
        INSERT INTO integrations_outbox
            (event_id, event_type, aggregate_type, aggregate_id, app_id, payload, schema_version)
        VALUES ($1, $2, $3, $4, $5, $6, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind("external_ref.created")
    .bind("external_ref")
    .bind(&ext_id)
    .bind(app_id)
    .bind(&payload)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(elapsed_ms(started))
}

async fn bench_webhook_ingest(
    pool: &sqlx::PgPool,
    app_id: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let idem_key = format!("bench-wh-{}", Uuid::new_v4());
    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    let raw_payload = serde_json::json!({ "data": "benchmark", "id": idem_key });
    let headers = serde_json::json!({});

    let ingest_result = sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO integrations_webhook_ingest
            (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, NOW(), $6)
        ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind("internal")
    .bind("bench.event")
    .bind(&raw_payload)
    .bind(&headers)
    .bind(&idem_key)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some((ingest_id,)) = ingest_result {
        // Outbox event
        let envelope = serde_json::json!({
            "ingest_id": ingest_id,
            "system": "internal",
            "event_type": "bench.event",
            "received_at": chrono::Utc::now()
        });

        sqlx::query(
            r#"
            INSERT INTO integrations_outbox
                (event_id, event_type, aggregate_type, aggregate_id, app_id, payload, schema_version)
            VALUES ($1, $2, $3, $4, $5, $6, '1.0.0')
            "#,
        )
        .bind(event_id)
        .bind("webhook.received")
        .bind("webhook")
        .bind(ingest_id.to_string())
        .bind(app_id)
        .bind(&envelope)
        .execute(&mut *tx)
        .await?;

        // Mark processed
        sqlx::query("UPDATE integrations_webhook_ingest SET processed_at = NOW() WHERE id = $1")
            .bind(ingest_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(elapsed_ms(started))
}

async fn bench_webhook_duplicate(
    pool: &sqlx::PgPool,
    app_id: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let idem_key = format!("bench-dup-{}", Uuid::new_v4());

    // First insert
    let raw_payload = serde_json::json!({ "data": "dup-test" });
    let headers = serde_json::json!({});

    sqlx::query(
        r#"
        INSERT INTO integrations_webhook_ingest
            (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, NOW(), $6)
        ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
        "#,
    )
    .bind(app_id)
    .bind("internal")
    .bind("bench.dup")
    .bind(&raw_payload)
    .bind(&headers)
    .bind(&idem_key)
    .execute(pool)
    .await?;

    // Duplicate attempt — should be rejected by constraint
    let dup_result = sqlx::query_as::<_, (i64,)>(
        r#"
        INSERT INTO integrations_webhook_ingest
            (app_id, system, event_type, raw_payload, headers, received_at, idempotency_key)
        VALUES ($1, $2, $3, $4, $5, NOW(), $6)
        ON CONFLICT ON CONSTRAINT integrations_webhook_ingest_dedup DO NOTHING
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind("internal")
    .bind("bench.dup")
    .bind(&raw_payload)
    .bind(&headers)
    .bind(&idem_key)
    .fetch_optional(pool)
    .await?;

    assert!(dup_result.is_none(), "duplicate should be rejected");
    Ok(elapsed_ms(started))
}

async fn bench_ref_lookup(
    pool: &sqlx::PgPool,
    app_id: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();

    let _rows: Vec<(i64,)> = sqlx::query_as(
        r#"
        SELECT id FROM integrations_external_refs
        WHERE app_id = $1 AND entity_type = 'invoice'
        ORDER BY created_at DESC
        LIMIT 10
        "#,
    )
    .bind(app_id)
    .fetch_all(pool)
    .await?;

    Ok(elapsed_ms(started))
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_webhook_ingest WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
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

    println!(
        "{name}: n={count} avg={avg:.2}ms p50={p50:.2}ms p95={p95:.2}ms p99={p99:.2}ms"
    );
}

fn percentile(sorted: &[f64], pct: f64) -> f64 {
    let max_idx = (sorted.len() - 1) as f64;
    let idx = ((pct / 100.0) * max_idx).round() as usize;
    sorted[idx]
}
