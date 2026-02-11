use axum::body::Body;
use axum::Router;
use http_body_util::BodyExt;
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;

/// Connect to the test database and run migrations.
/// Uses a small connection pool with short timeouts for tests.
pub async fn setup_pool() -> PgPool {
    dotenvy::dotenv().ok();

    let url = std::env::var("DATABASE_URL_AR")
        .expect("DATABASE_URL_AR must be set for integration tests");

    // Configure test pool with small size and short timeouts to prevent connection leaks
    let pool = PgPoolOptions::new()
        .max_connections(5) // Small pool for tests
        .idle_timeout(Some(std::time::Duration::from_secs(30))) // Close idle connections after 30s
        .max_lifetime(Some(std::time::Duration::from_secs(300))) // 5 minute max lifetime
        .acquire_timeout(std::time::Duration::from_secs(5)) // Fail fast in tests
        .connect(&url)
        .await
        .expect("Failed to connect to test database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Generate a unique email for test customers.
pub fn unique_email() -> String {
    format!("test-{}@example.com", Uuid::new_v4())
}

/// Generate a unique external customer ID.
pub fn unique_external_id() -> String {
    format!("ext-{}", Uuid::new_v4())
}

/// Generate a unique plan ID.
pub fn unique_plan_id() -> String {
    format!("plan-{}", Uuid::new_v4())
}

/// Generate a unique reference ID.
pub fn unique_reference_id() -> String {
    format!("ref-{}", Uuid::new_v4())
}

/// Build the full AR API router with pool state for testing.
pub fn app(pool: &PgPool) -> Router {
    ar_rs::routes::ar_router(pool.clone())
}

/// Read response body as JSON.
pub async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// Create a test customer with known data.
/// Returns (customer_id, email, external_customer_id).
pub async fn seed_customer(pool: &PgPool, app_id: &str) -> (i32, String, String) {
    let email = unique_email();
    let external_id = unique_external_id();

    let customer_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_customers (
            app_id, email, external_customer_id, status, name,
            default_payment_method_id, payment_method_type,
            retry_attempt_count, created_at, updated_at
        ) VALUES ($1, $2, $3, 'active', 'Test Customer', 'pm_test', 'card', 0, NOW(), NOW())
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(&email)
    .bind(&external_id)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test customer");

    (customer_id, email, external_id)
}

/// Create a test subscription for a customer.
/// Returns subscription_id.
pub async fn seed_subscription(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    status: &str,
) -> i32 {
    let plan_id = unique_plan_id();
    let tilled_sub_id = format!("sub_{}", Uuid::new_v4());

    let subscription_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_subscriptions (
            app_id, ar_customer_id, tilled_subscription_id,
            plan_id, plan_name, price_cents, status, interval_unit, interval_count,
            current_period_start, current_period_end,
            payment_method_id, payment_method_type,
            cancel_at_period_end, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, 'Test Plan', 1000, $5::ar_subscriptions_status,
            'month'::ar_subscriptions_interval, 1,
            NOW(), NOW() + INTERVAL '1 month',
            'pm_test123', 'card',
            false, NOW(), NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(&tilled_sub_id)
    .bind(&plan_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test subscription");

    subscription_id
}

/// Create a test charge for a customer.
/// Returns charge_id.
pub async fn seed_charge(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    amount_cents: i32,
    status: &str,
) -> i32 {
    let reference_id = unique_reference_id();
    let tilled_charge_id = format!("ch_{}", Uuid::new_v4());

    let charge_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_charges (
            app_id, ar_customer_id, tilled_charge_id,
            status, amount_cents, currency, charge_type,
            reason, reference_id,
            created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, 'usd', 'one_time',
            'Test charge', $6,
            NOW(), NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(&tilled_charge_id)
    .bind(status)
    .bind(amount_cents)
    .bind(&reference_id)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test charge");

    charge_id
}

/// Create a test webhook event.
/// Returns webhook_id.
pub async fn seed_webhook(
    pool: &PgPool,
    app_id: &str,
    event_id: &str,
    event_type: &str,
    status: &str,
) -> i32 {
    // Create a valid TilledWebhookEvent payload
    // Note: The webhook processor expects data.id directly, not data.object.id
    let payload = serde_json::json!({
        "id": event_id,
        "type": event_type,
        "data": {
            "id": format!("pi_{}", Uuid::new_v4()),
            "amount": 1000,
            "currency": "usd",
            "customer_id": "cus_test",
            "status": "succeeded"
        },
        "created_at": chrono::Utc::now().timestamp(),
        "livemode": false
    });

    let webhook_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_webhooks (
            app_id, event_id, event_type, status,
            payload, attempt_count, received_at
        ) VALUES (
            $1, $2, $3, $4::ar_webhooks_status,
            $5, 1, NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(event_id)
    .bind(event_type)
    .bind(status)
    .bind(payload)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test webhook");

    webhook_id
}

/// Create a test payment method for a customer.
/// Returns payment_method_id.
pub async fn seed_payment_method(
    pool: &PgPool,
    app_id: &str,
    customer_id: i32,
    is_default: bool,
) -> i32 {
    let tilled_pm_id = format!("pm_{}", Uuid::new_v4());

    let payment_method_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_payment_methods (
            app_id, ar_customer_id, tilled_payment_method_id,
            status, type, brand, last4, exp_month, exp_year,
            is_default, metadata, created_at, updated_at
        ) VALUES (
            $1, $2, $3, 'active', 'card', 'visa', '4242', 12, 2027,
            $4, '{}', NOW(), NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(customer_id)
    .bind(&tilled_pm_id)
    .bind(is_default)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test payment method");

    payment_method_id
}

/// Clean up test customers by ID.
pub async fn cleanup_customers(pool: &PgPool, customer_ids: &[i32]) {
    for &customer_id in customer_ids {
        // Delete related records first (foreign keys)
        sqlx::query("DELETE FROM ar_refunds WHERE ar_customer_id = $1")
            .bind(customer_id)
            .execute(pool)
            .await
            .ok();

        sqlx::query("DELETE FROM ar_charges WHERE ar_customer_id = $1")
            .bind(customer_id)
            .execute(pool)
            .await
            .ok();

        sqlx::query("DELETE FROM ar_subscriptions WHERE ar_customer_id = $1")
            .bind(customer_id)
            .execute(pool)
            .await
            .ok();

        sqlx::query("DELETE FROM ar_payment_methods WHERE ar_customer_id = $1")
            .bind(customer_id)
            .execute(pool)
            .await
            .ok();

        sqlx::query("DELETE FROM ar_invoices WHERE ar_customer_id = $1")
            .bind(customer_id)
            .execute(pool)
            .await
            .ok();

        // Delete customer
        sqlx::query("DELETE FROM ar_customers WHERE id = $1")
            .bind(customer_id)
            .execute(pool)
            .await
            .ok();
    }
}

/// Create a test dispute for a charge.
/// Returns dispute_id.
pub async fn seed_dispute(
    pool: &PgPool,
    app_id: &str,
    charge_id: i32,
    status: &str,
) -> i32 {
    let tilled_dispute_id = format!("dp_{}", Uuid::new_v4());

    let dispute_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_disputes (
            app_id, tilled_dispute_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, 5000, 'usd', 'fraudulent', 'fraud',
            NOW() + INTERVAL '7 days', NOW(), NOW(), NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(&tilled_dispute_id)
    .bind(charge_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test dispute");

    dispute_id
}

/// Clean up test disputes by ID.
pub async fn cleanup_disputes(pool: &PgPool, dispute_ids: &[i32]) {
    for &dispute_id in dispute_ids {
        sqlx::query("DELETE FROM ar_disputes WHERE id = $1")
            .bind(dispute_id)
            .execute(pool)
            .await
            .ok();
    }
}

/// Clean up test webhooks by ID.
pub async fn cleanup_webhooks(pool: &PgPool, webhook_ids: &[i32]) {
    for &webhook_id in webhook_ids {
        sqlx::query("DELETE FROM ar_webhooks WHERE id = $1")
            .bind(webhook_id)
            .execute(pool)
            .await
            .ok();
    }
}

/// Create a test event for audit logging.
/// Returns event_id.
pub async fn seed_event(
    pool: &PgPool,
    app_id: &str,
    event_type: &str,
    source: &str,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
) -> i32 {
    let payload = serde_json::json!({
        "test": true,
        "timestamp": chrono::Utc::now().to_rfc3339()
    });

    let event_id: i32 = sqlx::query_scalar(
        r#"INSERT INTO ar_events (
            app_id, event_type, source, entity_type, entity_id, payload, created_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, NOW()
        )
        RETURNING id"#,
    )
    .bind(app_id)
    .bind(event_type)
    .bind(source)
    .bind(entity_type)
    .bind(entity_id)
    .bind(payload)
    .fetch_one(pool)
    .await
    .expect("Failed to seed test event");

    event_id
}

/// Clean up test events by ID.
pub async fn cleanup_events(pool: &PgPool, event_ids: &[i32]) {
    for &event_id in event_ids {
        sqlx::query("DELETE FROM ar_events WHERE id = $1")
            .bind(event_id)
            .execute(pool)
            .await
            .ok();
    }
}

/// Close the pool and release all connections.
/// Call this after tests to ensure proper cleanup.
pub async fn teardown_pool(pool: PgPool) {
    pool.close().await;
}
