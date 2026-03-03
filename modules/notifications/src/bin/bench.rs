use chrono::Utc;
use notifications_rs::inbox::{create_inbox_message, list_messages, InboxListParams};
use notifications_rs::scheduled::{dispatch_once, insert_pending, LoggingSender, RetryPolicy};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db";

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
    let user_id = format!("user-{}", Uuid::new_v4());
    let sender: Arc<dyn notifications_rs::scheduled::NotificationSender> = Arc::new(LoggingSender);

    println!("notifications benchmark starting");
    println!(
        "duration={}s tenant={} db={}",
        args.duration_secs, tenant_id, db_url
    );

    let mut dispatch_times = Vec::new();
    let mut inbox_list_times = Vec::new();
    let mut dlq_list_times = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);

    while Instant::now() < deadline {
        let recipient_ref = format!("{}:{}", tenant_id, user_id);
        insert_pending(
            &pool,
            &recipient_ref,
            "email",
            "invoice_due_soon",
            serde_json::json!({"invoice_id":"INV-BENCH","amount_due_minor":12345}),
            Utc::now() - chrono::Duration::seconds(1),
        )
        .await?;

        let started = Instant::now();
        let _ = dispatch_once(&pool, sender.clone(), RetryPolicy::default()).await?;
        dispatch_times.push(elapsed_ms(started));

        let notification_id = insert_pending(
            &pool,
            &recipient_ref,
            "email",
            "invoice_due_soon",
            serde_json::json!({"invoice_id":"INV-INBOX","amount_due_minor":999}),
            Utc::now(),
        )
        .await?;

        let _ = create_inbox_message(
            &pool,
            &tenant_id,
            &user_id,
            notification_id,
            "Benchmark Inbox",
            Some("bench body"),
            Some("ops"),
        )
        .await?;

        let started = Instant::now();
        let _ = list_messages(
            &pool,
            &InboxListParams {
                tenant_id: tenant_id.clone(),
                user_id: user_id.clone(),
                unread_only: false,
                include_dismissed: true,
                category: None,
                limit: 50,
                offset: 0,
            },
        )
        .await?;
        inbox_list_times.push(elapsed_ms(started));

        let started = Instant::now();
        let _: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT id, status FROM scheduled_notifications \
             WHERE status = 'dead_lettered' AND tenant_id = $1 \
             ORDER BY dead_lettered_at DESC NULLS LAST \
             LIMIT 50",
        )
        .bind(&tenant_id)
        .fetch_all(&pool)
        .await?;
        dlq_list_times.push(elapsed_ms(started));
    }

    print_stats("dispatch_once", &dispatch_times);
    print_stats("inbox_list", &inbox_list_times);
    print_stats("dlq_list", &dlq_list_times);

    Ok(())
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
