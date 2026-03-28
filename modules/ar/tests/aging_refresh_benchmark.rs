use chrono::{Duration, Utc};
use serial_test::serial;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
struct ComputedAging {
    currency: String,
    current_minor: i64,
    days_1_30_minor: i64,
    days_31_60_minor: i64,
    days_61_90_minor: i64,
    days_over_90_minor: i64,
    total_outstanding_minor: i64,
    invoice_count: i64,
}

async fn setup_pool() -> PgPool {
    let database_url = std::env::var("DATABASE_URL_AR")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5436/ar_db".to_string());

    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(60))
        .connect(&database_url)
        .await
        .expect("connect AR benchmark db");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run AR migrations");

    pool
}

async fn cleanup_app(pool: &PgPool, app_id: &str) {
    sqlx::query("DELETE FROM ar_payment_allocations WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_credit_notes WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoice_write_offs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_charges WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_invoices WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM ar_customers WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await
        .ok();
}

async fn seed_dataset(pool: &PgPool, app_id: &str, invoice_count: usize) -> i32 {
    let customer_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_customers (
            app_id, email, external_customer_id, status, name,
            default_payment_method_id, payment_method_type,
            retry_attempt_count, created_at, updated_at
        ) VALUES ($1, $2, $3, 'active', 'Benchmark Customer', 'pm_bench', 'card', 0, NOW(), NOW())
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(format!("bench-{}@example.com", Uuid::new_v4()))
    .bind(format!("ext-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert benchmark customer");

    let mut tx = pool.begin().await.expect("seed transaction");

    for idx in 0..invoice_count {
        let due_at = Utc::now() - Duration::days((idx % 120) as i64);
        let invoice_id: i32 = sqlx::query_scalar(
            r#"INSERT INTO ar_invoices (
                app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
                due_at, updated_at
            ) VALUES ($1, $2, $3, 'open', $4, 'usd', $5, NOW())
            RETURNING id"#,
        )
        .bind(app_id)
        .bind(format!("inv-{}-{}", app_id, idx))
        .bind(customer_id)
        .bind(10_000 + (idx as i32 % 500))
        .bind(due_at.naive_utc())
        .fetch_one(&mut *tx)
        .await
        .expect("insert benchmark invoice");

        if idx % 2 == 0 {
            sqlx::query(
                r#"INSERT INTO ar_charges (
                    app_id, tilled_charge_id, invoice_id, ar_customer_id, status, amount_cents,
                    currency, charge_type, reference_id, updated_at
                ) VALUES ($1, $2, $3, $4, 'succeeded', $5, 'usd', 'one_time', $6, NOW())"#,
            )
            .bind(app_id)
            .bind(format!("ch-{}-{}", app_id, idx))
            .bind(invoice_id)
            .bind(customer_id)
            .bind(2_500 + (idx as i32 % 100))
            .bind(format!("ref-charge-{}", idx))
            .execute(&mut *tx)
            .await
            .expect("insert benchmark charge");
        }

        if idx % 3 == 0 {
            sqlx::query(
                r#"INSERT INTO ar_payment_allocations (
                    app_id, payment_id, invoice_id, amount_cents, idempotency_key
                ) VALUES ($1, $2, $3, $4, $5)"#,
            )
            .bind(app_id)
            .bind(format!("pay-{}", idx))
            .bind(invoice_id)
            .bind(1_000 + (idx as i32 % 50))
            .bind(format!("alloc-{}-{}", app_id, idx))
            .execute(&mut *tx)
            .await
            .expect("insert benchmark allocation");
        }

        if idx % 5 == 0 {
            sqlx::query(
                r#"INSERT INTO ar_credit_notes (
                    credit_note_id, app_id, customer_id, invoice_id, amount_minor, currency, reason
                ) VALUES ($1, $2, $3, $4, $5, 'usd', 'benchmark')"#,
            )
            .bind(Uuid::new_v4())
            .bind(app_id)
            .bind(customer_id.to_string())
            .bind(invoice_id)
            .bind(700_i64)
            .execute(&mut *tx)
            .await
            .expect("insert benchmark credit note");
        }

        if idx % 7 == 0 {
            sqlx::query(
                r#"INSERT INTO ar_invoice_write_offs (
                    write_off_id, app_id, invoice_id, customer_id, written_off_amount_minor, currency, reason
                ) VALUES ($1, $2, $3, $4, $5, 'usd', 'benchmark')"#,
            )
            .bind(Uuid::new_v4())
            .bind(app_id)
            .bind(invoice_id)
            .bind(customer_id.to_string())
            .bind(400_i64)
            .execute(&mut *tx)
            .await
            .expect("insert benchmark write-off");
        }
    }

    tx.commit().await.expect("commit benchmark seed");
    customer_id
}

async fn run_legacy_query(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
) -> Result<ComputedAging, sqlx::Error> {
    sqlx::query_as::<_, ComputedAging>(
        r#"
        WITH open_invoices AS (
            SELECT
                i.id,
                i.amount_cents,
                i.currency,
                i.due_at,
                COALESCE(
                    (SELECT SUM(c.amount_cents)
                     FROM ar_charges c
                     WHERE c.invoice_id = i.id
                       AND c.status = 'succeeded'),
                    0
                ) AS paid_cents,
                COALESCE(
                    (SELECT SUM(a.amount_cents)
                     FROM ar_payment_allocations a
                     WHERE a.invoice_id = i.id),
                    0
                ) AS allocated_cents,
                COALESCE(
                    (SELECT SUM(cn.amount_minor)
                     FROM ar_credit_notes cn
                     WHERE cn.invoice_id = i.id
                       AND cn.status = 'issued'),
                    0
                ) AS credit_note_cents,
                COALESCE(
                    (SELECT SUM(wo.written_off_amount_minor)
                     FROM ar_invoice_write_offs wo
                     WHERE wo.invoice_id = i.id
                       AND wo.status = 'written_off'),
                    0
                ) AS written_off_cents
            FROM ar_invoices i
            WHERE i.app_id = $1
              AND i.ar_customer_id = $2
              AND i.status NOT IN ('paid', 'void', 'draft')
        ),
        open_balances AS (
            SELECT
                currency,
                GREATEST(0, amount_cents - paid_cents - allocated_cents - credit_note_cents - written_off_cents) AS open_balance,
                due_at,
                CASE
                    WHEN due_at IS NULL OR due_at >= NOW() THEN 'current'
                    WHEN due_at >= NOW() - INTERVAL '30 days' THEN 'days_1_30'
                    WHEN due_at >= NOW() - INTERVAL '60 days' THEN 'days_31_60'
                    WHEN due_at >= NOW() - INTERVAL '90 days' THEN 'days_61_90'
                    ELSE 'days_over_90'
                END AS bucket
            FROM open_invoices
            WHERE GREATEST(0, amount_cents - paid_cents - allocated_cents - credit_note_cents - written_off_cents) > 0
        )
        SELECT
            COALESCE(MAX(currency), 'usd') AS currency,
            COALESCE(SUM(CASE WHEN bucket = 'current'    THEN open_balance ELSE 0 END), 0)::BIGINT AS current_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_1_30'  THEN open_balance ELSE 0 END), 0)::BIGINT AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_31_60' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_61_90' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_over_90' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_over_90_minor,
            COALESCE(SUM(open_balance), 0)::BIGINT AS total_outstanding_minor,
            COUNT(*)::BIGINT AS invoice_count
        FROM open_balances
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .fetch_one(pool)
    .await
}

async fn run_optimized_query(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
) -> Result<ComputedAging, sqlx::Error> {
    sqlx::query_as::<_, ComputedAging>(
        r#"
        WITH invoice_base AS (
            SELECT
                i.id,
                i.amount_cents,
                i.currency,
                i.due_at
            FROM ar_invoices i
            WHERE i.app_id = $1
              AND i.ar_customer_id = $2
              AND i.status NOT IN ('paid', 'void', 'draft')
        ),
        charges AS (
            SELECT
                c.invoice_id,
                SUM(c.amount_cents)::BIGINT AS paid_cents
            FROM ar_charges c
            JOIN invoice_base i ON i.id = c.invoice_id
            WHERE c.status = 'succeeded'
            GROUP BY c.invoice_id
        ),
        allocations AS (
            SELECT
                a.invoice_id,
                SUM(a.amount_cents)::BIGINT AS allocated_cents
            FROM ar_payment_allocations a
            JOIN invoice_base i ON i.id = a.invoice_id
            GROUP BY a.invoice_id
        ),
        credit_notes AS (
            SELECT
                cn.invoice_id,
                SUM(cn.amount_minor)::BIGINT AS credit_note_cents
            FROM ar_credit_notes cn
            JOIN invoice_base i ON i.id = cn.invoice_id
            WHERE cn.status = 'issued'
            GROUP BY cn.invoice_id
        ),
        write_offs AS (
            SELECT
                wo.invoice_id,
                SUM(wo.written_off_amount_minor)::BIGINT AS written_off_cents
            FROM ar_invoice_write_offs wo
            JOIN invoice_base i ON i.id = wo.invoice_id
            WHERE wo.status = 'written_off'
            GROUP BY wo.invoice_id
        ),
        open_balances AS (
            SELECT
                i.currency,
                GREATEST(
                    0::BIGINT,
                    i.amount_cents::BIGINT
                        - COALESCE(ch.paid_cents, 0)
                        - COALESCE(al.allocated_cents, 0)
                        - COALESCE(cn.credit_note_cents, 0)
                        - COALESCE(wo.written_off_cents, 0)
                ) AS open_balance,
                i.due_at,
                CASE
                    WHEN i.due_at IS NULL OR i.due_at >= NOW() THEN 'current'
                    WHEN i.due_at >= NOW() - INTERVAL '30 days' THEN 'days_1_30'
                    WHEN i.due_at >= NOW() - INTERVAL '60 days' THEN 'days_31_60'
                    WHEN i.due_at >= NOW() - INTERVAL '90 days' THEN 'days_61_90'
                    ELSE 'days_over_90'
                END AS bucket
            FROM invoice_base i
            LEFT JOIN charges ch ON ch.invoice_id = i.id
            LEFT JOIN allocations al ON al.invoice_id = i.id
            LEFT JOIN credit_notes cn ON cn.invoice_id = i.id
            LEFT JOIN write_offs wo ON wo.invoice_id = i.id
            WHERE GREATEST(
                0::BIGINT,
                i.amount_cents::BIGINT
                    - COALESCE(ch.paid_cents, 0)
                    - COALESCE(al.allocated_cents, 0)
                    - COALESCE(cn.credit_note_cents, 0)
                    - COALESCE(wo.written_off_cents, 0)
            ) > 0
        )
        SELECT
            COALESCE(MAX(currency), 'usd') AS currency,
            COALESCE(SUM(CASE WHEN bucket = 'current'    THEN open_balance ELSE 0 END), 0)::BIGINT AS current_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_1_30'  THEN open_balance ELSE 0 END), 0)::BIGINT AS days_1_30_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_31_60' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_31_60_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_61_90' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_61_90_minor,
            COALESCE(SUM(CASE WHEN bucket = 'days_over_90' THEN open_balance ELSE 0 END), 0)::BIGINT AS days_over_90_minor,
            COALESCE(SUM(open_balance), 0)::BIGINT AS total_outstanding_minor,
            COUNT(*)::BIGINT AS invoice_count
        FROM open_balances
        "#,
    )
    .bind(app_id)
    .bind(customer_id)
    .fetch_one(pool)
    .await
}

#[tokio::test]
#[serial]
#[ignore]
async fn benchmark_aging_query_against_legacy_correlated_subqueries() {
    let pool = setup_pool().await;
    let app_id = "bench-aging-refresh";
    cleanup_app(&pool, app_id).await;
    let customer_id = seed_dataset(&pool, app_id, 1_000).await;

    let warmup_legacy = run_legacy_query(&pool, app_id, customer_id)
        .await
        .expect("warmup legacy");
    let warmup_optimized = run_optimized_query(&pool, app_id, customer_id)
        .await
        .expect("warmup optimized");
    assert_eq!(
        warmup_legacy.total_outstanding_minor,
        warmup_optimized.total_outstanding_minor
    );
    assert_eq!(warmup_legacy.invoice_count, warmup_optimized.invoice_count);

    let legacy_start = Instant::now();
    let legacy = run_legacy_query(&pool, app_id, customer_id)
        .await
        .expect("legacy query");
    let legacy_elapsed = legacy_start.elapsed();

    let optimized_start = Instant::now();
    let optimized = run_optimized_query(&pool, app_id, customer_id)
        .await
        .expect("optimized query");
    let optimized_elapsed = optimized_start.elapsed();

    assert_eq!(legacy.currency, optimized.currency);
    assert_eq!(legacy.current_minor, optimized.current_minor);
    assert_eq!(legacy.days_1_30_minor, optimized.days_1_30_minor);
    assert_eq!(legacy.days_31_60_minor, optimized.days_31_60_minor);
    assert_eq!(legacy.days_61_90_minor, optimized.days_61_90_minor);
    assert_eq!(legacy.days_over_90_minor, optimized.days_over_90_minor);
    assert_eq!(
        legacy.total_outstanding_minor,
        optimized.total_outstanding_minor
    );
    assert_eq!(legacy.invoice_count, optimized.invoice_count);

    println!(
        "legacy={:?} optimized={:?} speedup={:.2}x",
        legacy_elapsed,
        optimized_elapsed,
        legacy_elapsed.as_secs_f64() / optimized_elapsed.as_secs_f64()
    );

    cleanup_app(&pool, app_id).await;
    pool.close().await;
}
