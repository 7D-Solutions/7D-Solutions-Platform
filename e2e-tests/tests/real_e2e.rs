/// Real NATS-Based E2E Integration Test
///
/// This test:
/// 1. Starts NATS + Postgres via docker-compose
/// 2. Builds and starts all services as separate processes (ar-rs, subscriptions-rs, payments-rs, notifications-rs)
/// 3. Triggers a bill run via HTTP POST
/// 4. Asserts results in real databases
///
/// Run with: cargo test --test real_e2e -- --test-threads=1

use chrono::{NaiveDate, Utc};
use reqwest::Client;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Test Infrastructure
// ============================================================================

struct TestInfrastructure {
    ar_process: Child,
    subscriptions_process: Child,
    payments_process: Child,
    notifications_process: Child,
}

impl Drop for TestInfrastructure {
    fn drop(&mut self) {
        println!("🛑 Shutting down services...");
        let _ = self.ar_process.kill();
        let _ = self.subscriptions_process.kill();
        let _ = self.payments_process.kill();
        let _ = self.notifications_process.kill();

        // Wait for processes to exit
        let _ = self.ar_process.wait();
        let _ = self.subscriptions_process.wait();
        let _ = self.payments_process.wait();
        let _ = self.notifications_process.wait();

        println!("✓ All services stopped");
    }
}

// ============================================================================
// Database Pools
// ============================================================================

async fn connect_ar_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://ar_user:ar_pass@localhost:5434/ar_db")
        .await
        .expect("Failed to connect to AR database")
}

async fn connect_subscriptions_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db")
        .await
        .expect("Failed to connect to Subscriptions database")
}

async fn connect_payments_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://payments_user:payments_pass@localhost:5436/payments_db")
        .await
        .expect("Failed to connect to Payments database")
}

async fn connect_notifications_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db")
        .await
        .expect("Failed to connect to Notifications database")
}

// ============================================================================
// Service Management
// ============================================================================

fn start_service(
    binary_path: &str,
    service_name: &str,
    database_url: &str,
    port: u16,
) -> Child {
    println!("🚀 Starting {} on port {}...", service_name, port);

    Command::new(binary_path)
        .env("DATABASE_URL", database_url)
        .env("NATS_URL", "nats://localhost:4222")
        .env("BUS_TYPE", "nats")
        .env("HOST", "0.0.0.0")
        .env("PORT", port.to_string())
        .env("RUST_LOG", "info")
        .env("AR_BASE_URL", "http://localhost:8086") // For subscriptions
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect(&format!("Failed to start {}", service_name))
}

async fn wait_for_health(client: &Client, service_name: &str, url: &str, max_attempts: u32) -> Result<(), String> {
    println!("⏳ Waiting for {} to be healthy at {}...", service_name, url);

    for attempt in 1..=max_attempts {
        tokio::time::sleep(Duration::from_millis(500)).await;

        match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                println!("✓ {} is healthy (attempt {}/{})", service_name, attempt, max_attempts);
                return Ok(());
            }
            Ok(resp) => {
                println!("  {} health check returned {} (attempt {}/{})", service_name, resp.status(), attempt, max_attempts);
            }
            Err(e) => {
                println!("  {} health check failed: {} (attempt {}/{})", service_name, e, attempt, max_attempts);
            }
        }
    }

    Err(format!("{} failed to become healthy after {} attempts", service_name, max_attempts))
}

async fn start_all_services() -> Result<TestInfrastructure, String> {
    println!("🔧 Building all services...");

    let project_root = "/Users/james/Projects/7D-Solutions Platform";

    // Build all services
    let build_status = Command::new("cargo")
        .args(&["build", "--release"])
        .current_dir(project_root)
        .status()
        .expect("Failed to run cargo build");

    if !build_status.success() {
        return Err("Failed to build services".to_string());
    }

    println!("✓ All services built");

    // Start services with absolute paths
    let ar_process = start_service(
        &format!("{}/target/release/ar-rs", project_root),
        "ar-rs",
        "postgresql://ar_user:ar_pass@localhost:5434/ar_db",
        8086,
    );

    let subscriptions_process = start_service(
        &format!("{}/target/release/subscriptions-rs", project_root),
        "subscriptions-rs",
        "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db",
        8087,
    );

    let payments_process = start_service(
        &format!("{}/target/release/payments-rs", project_root),
        "payments-rs",
        "postgresql://payments_user:payments_pass@localhost:5436/payments_db",
        8088,
    );

    let notifications_process = start_service(
        &format!("{}/target/release/notifications-rs", project_root),
        "notifications-rs",
        "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db",
        8089,
    );

    // Wait for health checks
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    wait_for_health(&client, "AR", "http://localhost:8086/api/health", 60).await?;
    wait_for_health(&client, "Subscriptions", "http://localhost:8087/api/health", 60).await?;
    wait_for_health(&client, "Payments", "http://localhost:8088/api/health", 60).await?;
    wait_for_health(&client, "Notifications", "http://localhost:8089/api/health", 60).await?;

    println!("✓ All services are healthy");

    Ok(TestInfrastructure {
        ar_process,
        subscriptions_process,
        payments_process,
        notifications_process,
    })
}

// ============================================================================
// Test Data Setup
// ============================================================================

async fn setup_test_data(ar_pool: &PgPool, subscriptions_pool: &PgPool) -> Result<(i32, Uuid), Box<dyn std::error::Error>> {
    let tenant_id = "test-tenant";

    // Create AR customer
    let email = format!("e2e-test-{}@example.com", Uuid::new_v4());
    let external_id = format!("ext-{}", Uuid::new_v4());

    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, external_customer_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, NOW(), NOW())
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&email)
    .bind("E2E Test Customer")
    .bind(&external_id)
    .fetch_one(ar_pool)
    .await?;

    println!("✓ Created AR customer: {}", customer_id);

    // Create subscription plan
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscription_plans
         (id, tenant_id, name, description, schedule, price_minor, currency, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'usd', NOW(), NOW())"
    )
    .bind(plan_id)
    .bind(tenant_id)
    .bind("E2E Test Plan")
    .bind("Test plan for E2E testing")
    .execute(subscriptions_pool)
    .await?;

    // Create subscription due today
    let subscription_id = Uuid::new_v4();
    let today = Utc::now().date_naive();

    sqlx::query(
        "INSERT INTO subscriptions
         (id, tenant_id, ar_customer_id, plan_id, status, schedule, price_minor, currency, start_date, next_bill_date, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'active', 'monthly', 2999, 'usd', $5, $6, NOW(), NOW())"
    )
    .bind(subscription_id)
    .bind(tenant_id)
    .bind(customer_id.to_string())
    .bind(plan_id)
    .bind(today)
    .bind(today)
    .execute(subscriptions_pool)
    .await?;

    println!("✓ Created subscription: {}", subscription_id);

    Ok((customer_id, subscription_id))
}

// ============================================================================
// Main E2E Test
// ============================================================================

#[tokio::test]
#[ignore] // Run explicitly with: cargo test --test real_e2e -- --ignored
async fn test_real_nats_based_e2e() {
    println!("\n🚀 Starting Real NATS-Based E2E Integration Test\n");

    // Check infrastructure is running
    println!("🔍 Checking infrastructure (NATS + Postgres)...");
    let nats_check = reqwest::get("http://localhost:8222/healthz").await;
    if nats_check.is_err() {
        panic!("❌ NATS is not running. Start infrastructure first:\n  docker compose -f docker-compose.infrastructure.yml up -d");
    }
    println!("✓ NATS is running");

    // Connect to databases
    let ar_pool = connect_ar_db().await;
    let subscriptions_pool = connect_subscriptions_db().await;
    let payments_pool = connect_payments_db().await;
    let notifications_pool = connect_notifications_db().await;
    println!("✓ Connected to all databases");

    // Clean up test data from previous runs
    println!("🧹 Cleaning up previous test data...");
    sqlx::query("TRUNCATE TABLE ar_invoices, ar_customers CASCADE")
        .execute(&ar_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE subscriptions, subscription_plans, bill_runs CASCADE")
        .execute(&subscriptions_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE payments, payments_events_outbox, payments_processed_events CASCADE")
        .execute(&payments_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE notifications, events_outbox, processed_events CASCADE")
        .execute(&notifications_pool)
        .await
        .ok();
    println!("✓ Databases cleaned");

    // Setup test data
    let (ar_customer_id, subscription_id) = setup_test_data(&ar_pool, &subscriptions_pool)
        .await
        .expect("Failed to setup test data");

    // Start all services
    let _infra = start_all_services().await.expect("Failed to start services");

    // Give services a moment to initialize event consumers
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Trigger bill run via HTTP POST
    println!("\n📋 Triggering bill run via HTTP POST...");
    let client = Client::new();
    let today = Utc::now().date_naive();

    let response = client
        .post("http://localhost:8087/api/bill-runs/execute")
        .json(&json!({
            "execution_date": today.format("%Y-%m-%d").to_string()
        }))
        .send()
        .await
        .expect("Failed to trigger bill run");

    println!("Bill run response status: {}", response.status());

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        panic!("❌ Bill run failed: {}", error_text);
    }

    let bill_run_response: serde_json::Value = response.json().await.expect("Failed to parse response");
    println!("✓ Bill run triggered: {:?}", bill_run_response);

    // Wait for event propagation through NATS
    println!("\n⏳ Waiting for events to propagate through NATS...");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // ASSERTIONS: Check results in databases
    println!("\n🔍 Verifying results in databases...\n");

    // 1. Check AR: Invoice should be created and finalized (status = 'open')
    let invoice: Option<(i32, String, i64, String, i32)> = sqlx::query_as(
        "SELECT id, status, amount_cents, currency, ar_customer_id
         FROM ar_invoices
         WHERE ar_customer_id = $1
         ORDER BY created_at DESC
         LIMIT 1"
    )
    .bind(ar_customer_id)
    .fetch_optional(&ar_pool)
    .await
    .expect("Failed to query AR invoices");

    match invoice {
        Some((invoice_id, status, amount, currency, cust_id)) => {
            println!("✓ AR Invoice created:");
            println!("  - ID: {}", invoice_id);
            println!("  - Status: {}", status);
            println!("  - Amount: {} {}", amount, currency);
            println!("  - Customer: {}", cust_id);
            assert_eq!(status, "open", "Invoice should be finalized (status='open')");
            assert_eq!(amount, 2999, "Invoice amount should be 2999");
            assert_eq!(currency, "usd", "Invoice currency should be USD");
        }
        None => {
            panic!("❌ No invoice found in AR database for customer {}", ar_customer_id);
        }
    }

    // 2. Check AR: Invoice should be marked as paid (after payment applied)
    // Give a bit more time for payment to be applied
    tokio::time::sleep(Duration::from_secs(3)).await;

    let paid_invoice_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE ar_customer_id = $1 AND status = 'paid'"
    )
    .bind(ar_customer_id)
    .fetch_one(&ar_pool)
    .await
    .expect("Failed to query paid invoices");

    println!("\n✓ AR Invoice payment status:");
    println!("  - Paid invoices: {}", paid_invoice_count);
    assert!(paid_invoice_count > 0, "Invoice should be marked as paid after payment applied");

    // 3. Check Payments: Payment record should exist
    let payment_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments WHERE status = 'succeeded'"
    )
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to query payments");

    println!("\n✓ Payments DB:");
    println!("  - Successful payments: {}", payment_count);
    assert!(payment_count > 0, "At least one payment should be recorded");

    // 4. Check Notifications: Notification should be sent
    let notification_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE status = 'sent'"
    )
    .fetch_one(&notifications_pool)
    .await
    .expect("Failed to query notifications");

    println!("\n✓ Notifications DB:");
    println!("  - Sent notifications: {}", notification_count);
    assert!(notification_count > 0, "At least one notification should be sent");

    // 5. Check Subscriptions: next_bill_date should be updated
    let next_bill_date: NaiveDate = sqlx::query_scalar(
        "SELECT next_bill_date FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to query subscription");

    println!("\n✓ Subscriptions DB:");
    println!("  - Next bill date: {}", next_bill_date);
    assert!(next_bill_date > today, "Subscription next_bill_date should be updated to future date");

    println!("\n🎉 E2E Test Passed! All assertions successful.\n");

    // Services will be stopped when _infra is dropped
}
