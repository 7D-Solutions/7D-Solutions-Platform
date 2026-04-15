use chrono::NaiveDate;
use gl_rs::contracts::gl_posting_request_v1::{GlPostingRequestV1, JournalLine, SourceDocType};
use gl_rs::services::income_statement_service::get_income_statement;
use gl_rs::services::journal_service::process_gl_posting_request;
use gl_rs::services::trial_balance_service::get_trial_balance;
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgresql://gl_user:gl_pass@localhost:5438/gl_db";

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

    let tenant_id = format!("bench-tenant-{}", Uuid::new_v4());
    let period_id = setup_tenant_fixtures(&pool, &tenant_id).await?;

    println!("gl benchmark starting");
    println!(
        "duration={}s tenant={} period_id={} db={}",
        args.duration_secs, tenant_id, period_id, db_url
    );

    let mut post_times = Vec::new();
    let mut trial_balance_times = Vec::new();
    let mut income_statement_times = Vec::new();
    let mut post_duplicate_times = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);

    while Instant::now() < deadline {
        let posting_ms = bench_post_journal(&pool, &tenant_id).await?;
        post_times.push(posting_ms);

        let trial_balance_ms = bench_trial_balance(&pool, &tenant_id, period_id).await?;
        trial_balance_times.push(trial_balance_ms);

        let income_statement_ms = bench_income_statement(&pool, &tenant_id, period_id).await?;
        income_statement_times.push(income_statement_ms);

        let post_duplicate_ms = bench_post_duplicate(&pool, &tenant_id).await?;
        post_duplicate_times.push(post_duplicate_ms);
    }

    print_stats("post_journal", &post_times);
    print_stats("trial_balance", &trial_balance_times);
    print_stats("income_statement", &income_statement_times);
    print_stats("post_duplicate", &post_duplicate_times);

    Ok(())
}

async fn setup_tenant_fixtures(pool: &sqlx::PgPool, tenant_id: &str) -> Result<Uuid, sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
          ($1, $2, '1100', 'Accounts Receivable', 'asset', 'debit', true, NOW()),
          ($3, $2, '4000', 'Product Revenue', 'revenue', 'credit', true, NOW())
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await?;

    let period_id = Uuid::new_v4();
    let period_start = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date");
    let period_end = NaiveDate::from_ymd_opt(2026, 1, 31).expect("valid date");

    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES ($1, $2, $3, $4, false, NOW())
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(period_id)
}

async fn bench_post_journal(
    pool: &sqlx::PgPool,
    tenant_id: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let event_id = Uuid::new_v4();

    let payload = GlPostingRequestV1 {
        posting_date: "2026-01-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("bench-inv-{}", event_id),
        description: "GL benchmark posting".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 100.0,
                credit: 0.0,
                memo: Some("AR debit".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 100.0,
                memo: Some("Revenue credit".to_string()),
                dimensions: None,
            },
        ],
    };

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        "bench",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await?;

    Ok(elapsed_ms(started))
}

async fn bench_trial_balance(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _ = get_trial_balance(pool, tenant_id, period_id, "USD").await?;
    Ok(elapsed_ms(started))
}

async fn bench_income_statement(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _ = get_income_statement(pool, tenant_id, period_id, "USD").await?;
    Ok(elapsed_ms(started))
}

async fn bench_post_duplicate(
    pool: &sqlx::PgPool,
    tenant_id: &str,
) -> Result<f64, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let event_id = Uuid::new_v4();
    let payload = GlPostingRequestV1 {
        posting_date: "2026-01-15".to_string(),
        currency: "USD".to_string(),
        source_doc_type: SourceDocType::ArInvoice,
        source_doc_id: format!("bench-dup-{}", event_id),
        description: "GL benchmark duplicate posting probe".to_string(),
        lines: vec![
            JournalLine {
                account_ref: "1100".to_string(),
                debit: 50.0,
                credit: 0.0,
                memo: Some("AR debit".to_string()),
                dimensions: None,
            },
            JournalLine {
                account_ref: "4000".to_string(),
                debit: 0.0,
                credit: 50.0,
                memo: Some("Revenue credit".to_string()),
                dimensions: None,
            },
        ],
    };

    process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        "bench",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await?;

    let _duplicate = process_gl_posting_request(
        pool,
        event_id,
        tenant_id,
        "bench",
        "gl.events.posting.requested",
        &payload,
        None,
    )
    .await;

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
