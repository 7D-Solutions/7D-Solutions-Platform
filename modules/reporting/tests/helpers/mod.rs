//! Shared test helpers for reporting integration tests.

use axum::{
    body::Body,
    extract::Request,
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Router,
};
use http_body_util::BodyExt;
use reporting::{metrics::ReportingMetrics, AppState};
use security::{ActorType, VerifiedClaims};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use uuid::Uuid;

// ── DB setup ────────────────────────────────────────────────────────────────

pub async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("REPORTING_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://ap_user:ap_pass@localhost:5443/reporting_test".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to reporting test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run reporting migrations");

    pool
}

pub fn unique_tenant() -> Uuid {
    Uuid::new_v4()
}

// ── Claims injection middleware ──────────────────────────────────────────────

/// Injects `VerifiedClaims` from `X-Tenant-Id` header (UUID). Test-only.
async fn inject_claims(req: Request, next: Next) -> Response {
    let tenant_id = req
        .headers()
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok());

    match tenant_id {
        Some(tid) => {
            let claims = VerifiedClaims {
                user_id: Uuid::new_v4(),
                tenant_id: tid,
                app_id: None,
                roles: vec!["admin".to_string()],
                perms: vec![],
                actor_type: ActorType::User,
                issued_at: chrono::Utc::now(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                token_id: Uuid::new_v4(),
                version: "1".to_string(),
            };
            let mut req = req;
            req.extensions_mut().insert(claims);
            next.run(req).await
        }
        None => next.run(req).await,
    }
}

// ── Test app builder ────────────────────────────────────────────────────────

/// Build a test router with claims-injection middleware.
/// Pass tenant via `X-Tenant-Id: <uuid>` header.
pub fn build_test_app(pool: sqlx::PgPool) -> Router {
    let metrics = Arc::new(ReportingMetrics::new().expect("metrics init"));
    let app_state = Arc::new(AppState {
        pool: pool.clone(),
        metrics,
    });

    Router::new()
        .route("/api/reporting/pl", get(reporting::http::statements::get_pl))
        .route(
            "/api/reporting/balance-sheet",
            get(reporting::http::statements::get_balance_sheet),
        )
        .route(
            "/api/reporting/cashflow",
            get(reporting::http::cashflow::get_cashflow),
        )
        .route(
            "/api/reporting/ar-aging",
            get(reporting::http::aging::get_ar_aging),
        )
        .route(
            "/api/reporting/ap-aging",
            get(reporting::http::aging::get_ap_aging),
        )
        .route(
            "/api/reporting/kpis",
            get(reporting::http::kpis::get_kpis),
        )
        .route(
            "/api/reporting/forecast",
            get(reporting::http::forecast::get_forecast),
        )
        .route(
            "/api/reporting/rebuild",
            post(reporting::http::admin::rebuild),
        )
        .layer(middleware::from_fn(inject_claims))
        .with_state(app_state)
}

// ── Response helpers ────────────────────────────────────────────────────────

pub async fn body_json(response: Response<Body>) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
}

// ── Seed helpers ────────────────────────────────────────────────────────────

pub async fn seed_trial_balance(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    as_of: &str,
    account_code: &str,
    account_name: &str,
    currency: &str,
    debit_minor: i64,
    credit_minor: i64,
) {
    let net = debit_minor - credit_minor;
    sqlx::query(
        r#"INSERT INTO rpt_trial_balance_cache
            (tenant_id, as_of, account_code, account_name, currency,
             debit_minor, credit_minor, net_minor)
        VALUES ($1, $2::DATE, $3, $4, $5, $6, $7, $8)
        ON CONFLICT (tenant_id, as_of, account_code, currency) DO UPDATE
            SET debit_minor = EXCLUDED.debit_minor,
                credit_minor = EXCLUDED.credit_minor,
                net_minor = EXCLUDED.net_minor"#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(account_code)
    .bind(account_name)
    .bind(currency)
    .bind(debit_minor)
    .bind(credit_minor)
    .bind(net)
    .execute(pool)
    .await
    .expect("seed trial balance");
}

pub async fn seed_ar_aging(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    as_of: &str,
    customer_id: &str,
) {
    sqlx::query(
        r#"INSERT INTO rpt_ar_aging_cache
            (tenant_id, as_of, customer_id, currency, current_minor,
             bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
             bucket_over_90_minor, total_minor)
        VALUES ($1, $2::DATE, $3, 'USD', 10000, 5000, 2000, 1000, 500, 18500)
        ON CONFLICT (tenant_id, as_of, customer_id, currency) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(customer_id)
    .execute(pool)
    .await
    .expect("seed ar aging");
}

pub async fn seed_ap_aging(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    as_of: &str,
    vendor_id: &str,
) {
    sqlx::query(
        r#"INSERT INTO rpt_ap_aging_cache
            (tenant_id, as_of, vendor_id, currency, current_minor,
             bucket_1_30_minor, bucket_31_60_minor, bucket_61_90_minor,
             bucket_over_90_minor, total_minor)
        VALUES ($1, $2::DATE, $3, 'USD', 8000, 3000, 1000, 500, 200, 12700)
        ON CONFLICT (tenant_id, as_of, vendor_id, currency) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(vendor_id)
    .execute(pool)
    .await
    .expect("seed ap aging");
}

pub async fn seed_cashflow(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    period_start: &str,
    period_end: &str,
) {
    sqlx::query(
        r#"INSERT INTO rpt_cashflow_cache
            (tenant_id, period_start, period_end, activity_type,
             line_code, line_label, currency, amount_minor)
        VALUES ($1, $2::DATE, $3::DATE, 'operating',
                'cash_collections', 'Cash Collections', 'USD', 50000)
        ON CONFLICT (tenant_id, period_start, period_end,
                     activity_type, line_code, currency) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(period_start)
    .bind(period_end)
    .execute(pool)
    .await
    .expect("seed cashflow");
}

pub async fn seed_kpi_cache(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    as_of: &str,
    kpi_name: &str,
    currency: &str,
    amount_minor: i64,
) {
    sqlx::query(
        r#"INSERT INTO rpt_kpi_cache
            (tenant_id, as_of, kpi_name, currency, amount_minor, computed_at)
        VALUES ($1, $2::DATE, $3, $4, $5, NOW())
        ON CONFLICT (tenant_id, as_of, kpi_name, currency) DO UPDATE
            SET amount_minor = EXCLUDED.amount_minor"#,
    )
    .bind(tenant_id)
    .bind(as_of)
    .bind(kpi_name)
    .bind(currency)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("seed kpi cache");
}

pub async fn seed_open_invoice(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    invoice_id: &str,
    customer_id: &str,
    currency: &str,
    amount_cents: i64,
    age_days: i64,
) {
    let issued = chrono::Utc::now() - chrono::Duration::days(age_days);
    sqlx::query(
        r#"INSERT INTO rpt_open_invoices_cache
            (tenant_id, invoice_id, customer_id, currency, amount_cents,
             issued_at, status, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, 'open', NOW(), NOW())
        ON CONFLICT (tenant_id, invoice_id) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(invoice_id)
    .bind(customer_id)
    .bind(currency)
    .bind(amount_cents)
    .bind(issued)
    .execute(pool)
    .await
    .expect("seed open invoice");
}

pub async fn seed_payment_history(
    pool: &sqlx::PgPool,
    tenant_id: &str,
    customer_id: &str,
    invoice_id: &str,
    currency: &str,
    amount_cents: i64,
    days_to_pay: i32,
) {
    let issued = chrono::Utc::now() - chrono::Duration::days(90);
    let paid = issued + chrono::Duration::days(days_to_pay as i64);
    sqlx::query(
        r#"INSERT INTO rpt_payment_history
            (tenant_id, customer_id, invoice_id, currency, amount_cents,
             issued_at, paid_at, days_to_pay, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW())
        ON CONFLICT (tenant_id, invoice_id) DO NOTHING"#,
    )
    .bind(tenant_id)
    .bind(customer_id)
    .bind(invoice_id)
    .bind(currency)
    .bind(amount_cents)
    .bind(issued)
    .bind(paid)
    .bind(days_to_pay)
    .execute(pool)
    .await
    .expect("seed payment history");
}
