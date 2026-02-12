/// Real NATS-Based E2E Integration Test - Sad Path (Payment Failure)
///
/// This test:
/// 1. Starts NATS + Postgres via docker-compose.infrastructure.yml
/// 2. Builds and starts all services via docker-compose.modules.yml (Docker containers)
/// 3. Triggers a bill run with a payment method that will fail
/// 4. Asserts payment failure is handled correctly:
///    - AR invoice remains OPEN (not paid)
///    - Notifications processes the payment.failed event
///    - DLQ tables stay EMPTY (expected failures don't go to DLQ)
///    - No payment.succeeded event is emitted
///
/// Run with: cargo test --test real_e2e_sad_path -- --test-threads=1

use chrono::Utc;
use reqwest::Client;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Test Infrastructure
// ============================================================================

struct TestInfrastructure {
    project_root: String,
}

impl Drop for TestInfrastructure {
    fn drop(&mut self) {
        if std::env::var("E2E_KEEP_CONTAINERS").unwrap_or_default() == "1" {
            println!("E2E_KEEP_CONTAINERS=1 → skipping docker compose down for debugging.");
            println!("Inspect logs with: docker logs 7d-<service>");
            return;
        }
        println!("🛑 Shutting down services...");
        let _ = Command::new("docker")
            .args(&["compose", "-f", "docker-compose.modules.yml", "down"])
            .current_dir(&self.project_root)
            .status();
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

async fn wait_for_log_line(container: &str, needle: &str, timeout_secs: u64) -> Result<(), String> {
    println!("⏳ Waiting for '{}' in {} logs...", needle, container);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        let output = Command::new("docker")
            .args(&["logs", container])
            .output()
            .map_err(|e| format!("Failed to get logs for {}: {}", container, e))?;

        let logs = String::from_utf8_lossy(&output.stdout).to_string()
            + &String::from_utf8_lossy(&output.stderr);

        if logs.contains(needle) {
            println!("✓ Found '{}' in {} logs", needle, container);
            return Ok(());
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Timed out waiting for '{}' in {} logs after {}s",
                needle, container, timeout_secs
            ));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
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
    let project_root = "/Users/james/Projects/7D-Solutions Platform";

    // Recreate services with docker compose (picks up rebuilt images)
    println!("🔄 Recreating services with docker compose...");
    let recreate_status = Command::new("docker")
        .args(&["compose", "-f", "docker-compose.modules.yml", "up", "-d", "--force-recreate"])
        .current_dir(project_root)
        .status()
        .expect("Failed to recreate services");

    if !recreate_status.success() {
        return Err("Failed to recreate services with docker compose".to_string());
    }

    println!("✓ Services recreated");

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

    // Wait for NATS consumers to be subscribed (readiness gate)
    wait_for_log_line("7d-payments", "Subscribed to ar.events.payment.collection.requested", 30).await?;
    wait_for_log_line("7d-payments", "Starting outbox publisher", 30).await?;
    wait_for_log_line("7d-ar", "Subscribed to payments.events.payment.succeeded", 30).await?;
    wait_for_log_line("7d-ar", "Publisher tick", 30).await?;
    wait_for_log_line("7d-notifications", "Subscribed to payments.events.payment.succeeded", 30).await?;
    wait_for_log_line("7d-notifications", "Subscribed to payments.events.payment.failed", 30).await?;

    println!("✓ All NATS consumers ready");

    Ok(TestInfrastructure {
        project_root: project_root.to_string(),
    })
}

// ============================================================================
// Test Data Setup
// ============================================================================

async fn setup_test_data_with_failing_payment(
    ar_pool: &PgPool,
    subscriptions_pool: &PgPool,
) -> Result<(i32, Uuid), Box<dyn std::error::Error>> {
    let tenant_id = "test-app";

    // Create AR customer with failing payment method
    let email = format!("e2e-sad-path-{}@example.com", Uuid::new_v4());
    let external_id = format!("ext-sad-{}", Uuid::new_v4());
    let failing_payment_method = "fail_insufficient_funds";

    let customer_id: i32 = sqlx::query_scalar(
        "INSERT INTO ar_customers (app_id, email, name, external_customer_id, default_payment_method_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW())
         RETURNING id"
    )
    .bind(tenant_id)
    .bind(&email)
    .bind("E2E Sad Path Customer")
    .bind(&external_id)
    .bind(failing_payment_method)
    .fetch_one(ar_pool)
    .await?;

    println!("✓ Created AR customer with failing payment method: {}", customer_id);

    // Create subscription plan
    let plan_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO subscription_plans
         (id, tenant_id, name, description, schedule, price_minor, currency, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'monthly', 2999, 'usd', NOW(), NOW())"
    )
    .bind(plan_id)
    .bind(tenant_id)
    .bind("E2E Sad Path Plan")
    .bind("Test plan for E2E sad path testing")
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

    println!("✓ Created subscription (will use customer's default payment method): {}", subscription_id);

    Ok((customer_id, subscription_id))
}

// ============================================================================
// Main E2E Sad Path Test
// ============================================================================

#[tokio::test]
#[ignore] // Run explicitly with: cargo test --test real_e2e_sad_path -- --ignored
async fn test_payment_failure_sad_path() {
    println!("\n🚀 Starting E2E Sad Path Test (Payment Failure)\n");

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
    sqlx::query("TRUNCATE TABLE ar_invoices, ar_customers, events_outbox, processed_events CASCADE")
        .execute(&ar_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE subscriptions, subscription_plans, bill_runs CASCADE")
        .execute(&subscriptions_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE payments, payments_events_outbox, payments_processed_events, failed_events CASCADE")
        .execute(&payments_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE notifications, events_outbox, processed_events, failed_events CASCADE")
        .execute(&notifications_pool)
        .await
        .ok();
    println!("✓ Databases cleaned");

    // Setup test data with failing payment method
    let (ar_customer_id, _subscription_id) = setup_test_data_with_failing_payment(&ar_pool, &subscriptions_pool)
        .await
        .expect("Failed to setup test data");

    // Start all services
    let _infra = start_all_services().await.expect("Failed to start services");

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

    // Poll for event propagation with timeout
    println!("\n⏳ Waiting for sad path event chain to complete...");
    let poll_deadline = tokio::time::Instant::now() + Duration::from_secs(15);

    // 1. Wait for invoice to be created
    let invoice_id: i32;
    loop {
        let invoice: Option<(i32, String, i32, String, i32)> = sqlx::query_as(
            "SELECT id, status, amount_cents, currency, ar_customer_id
             FROM ar_invoices WHERE ar_customer_id = $1
             ORDER BY created_at DESC LIMIT 1"
        )
        .bind(ar_customer_id)
        .fetch_optional(&ar_pool)
        .await
        .expect("Failed to query AR invoices");

        if let Some((id, status, amount, currency, cust_id)) = invoice {
            invoice_id = id;
            println!("\n✓ AR Invoice created:");
            println!("  - ID: {}", id);
            println!("  - Status: {}", status);
            println!("  - Amount: {} {}", amount, currency);
            println!("  - Customer: {}", cust_id);
            assert_eq!(amount, 2999, "Invoice amount should be 2999");
            assert_eq!(currency, "usd", "Invoice currency should be USD");
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            panic!("❌ No invoice found in AR database for customer {}", ar_customer_id);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 2. Wait for payment.failed event in Payments outbox
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM payments_events_outbox WHERE event_type = 'payment.failed'"
        )
        .fetch_one(&payments_pool)
        .await
        .expect("Failed to query payments outbox");

        if count > 0 {
            println!("✓ Payments outbox: {} payment.failed event(s)", count);
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            panic!("❌ No payment.failed event in payments outbox within timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 3. Wait for Notifications to process the payment.failed event
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM processed_events WHERE subject = 'payments.events.payment.failed'"
        )
        .fetch_one(&notifications_pool)
        .await
        .expect("Failed to query notifications processed_events");

        if count > 0 {
            println!("✓ Notifications: {} payment.failed event(s) processed", count);
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            // Dump full container logs before failing
            for container in &["7d-payments", "7d-notifications"] {
                println!("\n📊 === Full logs: {} ===", container);
                if let Ok(output) = Command::new("docker").args(&["logs", container]).output() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stdout.is_empty() {
                        println!("[stdout]");
                        for line in stdout.lines() {
                            println!("  {}", line);
                        }
                    }
                    if !stderr.is_empty() {
                        println!("[stderr]");
                        for line in stderr.lines() {
                            println!("  {}", line);
                        }
                    }
                } else {
                    println!("  (failed to get logs)");
                }
            }
            panic!("❌ Notifications did not process payment.failed event within timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 4. CRITICAL ASSERTION: Invoice should remain OPEN (not paid)
    let invoice_status: String = sqlx::query_scalar(
        "SELECT status FROM ar_invoices WHERE id = $1"
    )
    .bind(invoice_id)
    .fetch_one(&ar_pool)
    .await
    .expect("Failed to query invoice status");

    println!("✓ AR Invoice status after payment failure: {}", invoice_status);
    assert_eq!(invoice_status, "open", "Invoice should remain OPEN after payment failure");

    // 5. NEGATIVE ASSERTION: No payment.succeeded events should exist
    let succeeded_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM payments_events_outbox WHERE event_type = 'payment.succeeded'"
    )
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to query payments outbox for succeeded events");

    println!("✓ Verified no payment.succeeded events: {} (expected 0)", succeeded_count);
    assert_eq!(succeeded_count, 0, "No payment.succeeded events should be emitted for failed payments");

    // 6. CRITICAL ASSERTION: DLQ tables should be EMPTY
    // Expected failures (like payment declined) should NOT go to DLQ
    // DLQ is only for unexpected processing errors
    let payments_dlq_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM failed_events"
    )
    .fetch_one(&payments_pool)
    .await
    .expect("Failed to query payments DLQ");

    let notifications_dlq_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM failed_events"
    )
    .fetch_one(&notifications_pool)
    .await
    .expect("Failed to query notifications DLQ");

    let ar_dlq_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM failed_events"
    )
    .fetch_one(&ar_pool)
    .await
    .expect("Failed to query AR DLQ");

    println!("✓ DLQ tables empty:");
    println!("  - Payments DLQ: {} events", payments_dlq_count);
    println!("  - Notifications DLQ: {} events", notifications_dlq_count);
    println!("  - AR DLQ: {} events", ar_dlq_count);

    assert_eq!(payments_dlq_count, 0, "Payments DLQ should be empty for expected failures");
    assert_eq!(notifications_dlq_count, 0, "Notifications DLQ should be empty for expected failures");
    assert_eq!(ar_dlq_count, 0, "AR DLQ should be empty for expected failures");

    println!("\n🎉 Sad Path E2E Test Passed! All assertions successful:");
    println!("  ✓ Invoice created");
    println!("  ✓ Payment failed as expected");
    println!("  ✓ payment.failed event emitted");
    println!("  ✓ Notifications processed failure");
    println!("  ✓ Invoice remains OPEN (not paid)");
    println!("  ✓ No payment.succeeded events");
    println!("  ✓ DLQ tables empty (expected failure, not processing error)\n");

    // Services will be stopped when _infra is dropped
}
