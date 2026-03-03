use chrono::{NaiveDate, Utc};
use reporting::domain::{
    aging::{ap_aging, ar_aging},
    forecast::cash_forecast,
    kpis,
    statements::{balance_sheet, cashflow, pl},
};
use sqlx::postgres::PgPoolOptions;
use std::time::{Duration, Instant};
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgres://ap_user:ap_pass@localhost:5443/reporting_test";

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
    let db_url = std::env::var("REPORTING_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| DEFAULT_DB_URL.to_string());

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db_url)
        .await?;

    sqlx::migrate!("./db/migrations").run(&pool).await?;

    let tenant_id = format!("bench-rpt-{}", Uuid::new_v4());

    println!("reporting benchmark starting");
    println!(
        "duration={}s tenant={} db={}",
        args.duration_secs, tenant_id, db_url
    );

    // Seed test data
    seed_data(&pool, &tenant_id).await?;

    let mut ap_aging_times = Vec::new();
    let mut ar_aging_times = Vec::new();
    let mut balance_sheet_times = Vec::new();
    let mut pl_times = Vec::new();
    let mut cashflow_times = Vec::new();
    let mut kpi_times = Vec::new();
    let mut forecast_times = Vec::new();

    let deadline = Instant::now() + Duration::from_secs(args.duration_secs);
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 15).expect("valid date");
    let from = NaiveDate::from_ymd_opt(2026, 2, 1).expect("valid date");
    let to = NaiveDate::from_ymd_opt(2026, 2, 28).expect("valid date");

    while Instant::now() < deadline {
        // AP aging
        let t = Instant::now();
        let _ = ap_aging::query_ap_aging(&pool, &tenant_id, as_of).await?;
        ap_aging_times.push(elapsed_ms(t));

        // AR aging
        let t = Instant::now();
        let _ = ar_aging::get_aging_for_tenant(&pool, &tenant_id, as_of).await?;
        ar_aging_times.push(elapsed_ms(t));

        // Balance sheet
        let t = Instant::now();
        let _ = balance_sheet::compute_balance_sheet(&pool, &tenant_id, as_of).await?;
        balance_sheet_times.push(elapsed_ms(t));

        // P&L
        let t = Instant::now();
        let _ = pl::compute_pl(&pool, &tenant_id, from, to).await?;
        pl_times.push(elapsed_ms(t));

        // Cashflow
        let t = Instant::now();
        let _ = cashflow::compute_cashflow(&pool, &tenant_id, from, to).await?;
        cashflow_times.push(elapsed_ms(t));

        // KPIs
        let t = Instant::now();
        let _ = kpis::compute_kpis(&pool, &tenant_id, as_of).await?;
        kpi_times.push(elapsed_ms(t));

        // Cash forecast
        let t = Instant::now();
        let _ = cash_forecast::compute_cash_forecast(&pool, &tenant_id, &[7, 30, 60]).await?;
        forecast_times.push(elapsed_ms(t));
    }

    print_stats("ap_aging", &ap_aging_times);
    print_stats("ar_aging", &ar_aging_times);
    print_stats("balance_sheet", &balance_sheet_times);
    print_stats("pl_statement", &pl_times);
    print_stats("cashflow", &cashflow_times);
    print_stats("kpis", &kpi_times);
    print_stats("cash_forecast", &forecast_times);

    // Cleanup
    cleanup(&pool, &tenant_id).await;

    Ok(())
}

async fn seed_data(
    pool: &sqlx::PgPool,
    tenant_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let as_of = NaiveDate::from_ymd_opt(2026, 2, 15).expect("valid date");

    // Seed AP aging: 10 vendors, 2 currencies
    for i in 0..10 {
        let vendor = format!("bench-v-{:03}", i);
        let currency = if i % 3 == 0 { "EUR" } else { "USD" };
        let current = (i as i64 + 1) * 10000;
        let total = current * 2;
        sqlx::query(
            r#"INSERT INTO rpt_ap_aging_cache
               (tenant_id, as_of, vendor_id, currency, current_minor,
                bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
                bucket_over_90_minor, total_minor, computed_at)
               VALUES ($1, $2, $3, $4, $5, $5, 0, 0, 0, $6, NOW())
               ON CONFLICT (tenant_id, as_of, vendor_id, currency) DO NOTHING"#,
        )
        .bind(tenant_id)
        .bind(as_of)
        .bind(&vendor)
        .bind(currency)
        .bind(current)
        .bind(total)
        .execute(pool)
        .await?;
    }

    // Seed AR aging: 10 customers
    for i in 0..10 {
        let customer = format!("bench-c-{:03}", i);
        let current = (i as i64 + 1) * 15000;
        let total = current * 2;
        sqlx::query(
            r#"INSERT INTO rpt_ar_aging_cache
               (tenant_id, as_of, customer_id, currency, current_minor,
                bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
                bucket_over_90_minor, total_minor, computed_at)
               VALUES ($1, $2, $3, 'USD', $4, $4, 0, 0, 0, $5, NOW())
               ON CONFLICT (tenant_id, as_of, customer_id, currency) DO NOTHING"#,
        )
        .bind(tenant_id)
        .bind(as_of)
        .bind(&customer)
        .bind(current)
        .bind(total)
        .execute(pool)
        .await?;
    }

    // Seed trial balance: representative chart of accounts
    let accounts = [
        ("1000", "Cash", "USD", 500000_i64, 0_i64),
        ("1100", "Accounts Receivable", "USD", 300000, 0),
        ("1200", "Inventory", "USD", 200000, 0),
        ("2000", "Accounts Payable", "USD", 0, 150000),
        ("2100", "Accrued Liabilities", "USD", 0, 50000),
        ("3000", "Retained Earnings", "USD", 0, 400000),
        ("4000", "Product Revenue", "USD", 0, 600000),
        ("4100", "Service Revenue", "USD", 0, 200000),
        ("5000", "Cost of Goods Sold", "USD", 300000, 0),
        ("6000", "Salaries Expense", "USD", 150000, 0),
        ("6100", "Rent Expense", "USD", 50000, 0),
        ("6200", "Utilities Expense", "USD", 10000, 0),
    ];

    for (code, name, currency, debit, credit) in &accounts {
        sqlx::query(
            r#"INSERT INTO rpt_trial_balance_cache
               (tenant_id, as_of, account_code, account_name, currency,
                debit_minor, credit_minor, net_minor, computed_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
               ON CONFLICT (tenant_id, as_of, account_code, currency) DO NOTHING"#,
        )
        .bind(tenant_id)
        .bind(as_of)
        .bind(code)
        .bind(name)
        .bind(currency)
        .bind(debit)
        .bind(credit)
        .bind(debit - credit)
        .execute(pool)
        .await?;
    }

    // Seed cashflow cache
    sqlx::query(
        r#"INSERT INTO rpt_cashflow_cache
           (tenant_id, period_start, period_end, activity_type,
            line_code, line_label, currency, amount_minor, computed_at)
           VALUES ($1, '2026-02-01', '2026-02-28', 'operating',
                   'cash_collections', 'Customer collections', 'USD', 450000, NOW())
           ON CONFLICT (tenant_id, period_start, period_end,
                        activity_type, line_code, currency) DO NOTHING"#,
    )
    .bind(tenant_id)
    .execute(pool)
    .await?;

    // Seed KPI cache
    sqlx::query(
        r#"INSERT INTO rpt_kpi_cache
           (tenant_id, as_of, kpi_name, currency, amount_minor, computed_at)
           VALUES ($1, $2, 'mrr', 'USD', 80000, NOW())
           ON CONFLICT (tenant_id, as_of, kpi_name, currency) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"INSERT INTO rpt_kpi_cache
           (tenant_id, as_of, kpi_name, currency, amount_minor, computed_at)
           VALUES ($1, $2, 'inventory_value', 'USD', 200000, NOW())
           ON CONFLICT (tenant_id, as_of, kpi_name, currency) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .execute(pool)
    .await?;

    // Seed payment history + open invoices for forecast
    let now = Utc::now();
    for i in 0..5 {
        let inv_id = format!("bench-hist-{}", i);
        let days = 10 + i * 10; // 10, 20, 30, 40, 50
        let issued = now - chrono::Duration::days(90);
        let paid = issued + chrono::Duration::days(days as i64);
        sqlx::query(
            r#"INSERT INTO rpt_payment_history
               (tenant_id, customer_id, invoice_id, currency, amount_cents,
                issued_at, paid_at, days_to_pay, created_at)
               VALUES ($1, 'bench-cust', $2, 'USD', 50000, $3, $4, $5, NOW())
               ON CONFLICT (tenant_id, invoice_id) DO NOTHING"#,
        )
        .bind(tenant_id)
        .bind(&inv_id)
        .bind(issued)
        .bind(paid)
        .bind(days)
        .execute(pool)
        .await?;
    }

    for i in 0..3 {
        let inv_id = format!("bench-open-{}", i);
        let issued = now - chrono::Duration::days((i * 5 + 5) as i64);
        sqlx::query(
            r#"INSERT INTO rpt_open_invoices_cache
               (tenant_id, invoice_id, customer_id, currency, amount_cents,
                issued_at, status, created_at, updated_at)
               VALUES ($1, $2, 'bench-cust', 'USD', 75000, $3, 'open', NOW(), NOW())
               ON CONFLICT (tenant_id, invoice_id) DO NOTHING"#,
        )
        .bind(tenant_id)
        .bind(&inv_id)
        .bind(issued)
        .execute(pool)
        .await?;
    }

    Ok(())
}

async fn cleanup(pool: &sqlx::PgPool, tenant_id: &str) {
    for table in &[
        "rpt_ap_aging_cache",
        "rpt_ar_aging_cache",
        "rpt_trial_balance_cache",
        "rpt_cashflow_cache",
        "rpt_kpi_cache",
        "rpt_statement_cache",
        "rpt_payment_history",
        "rpt_open_invoices_cache",
    ] {
        sqlx::query(&format!("DELETE FROM {} WHERE tenant_id = $1", table))
            .bind(tenant_id)
            .execute(pool)
            .await
            .ok();
    }
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
