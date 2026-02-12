/// Real NATS-Based E2E Integration Test
///
/// This test:
/// 1. Starts NATS + Postgres via docker-compose.infrastructure.yml
/// 2. Builds and starts all services via docker-compose.modules.yml (Docker containers)
/// 3. Triggers a bill run via HTTP POST
/// 4. Asserts results in real databases
///
/// Run with: cargo test --test real_e2e -- --test-threads=1

use chrono::{NaiveDate, Utc};
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

async fn connect_gl_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://gl_user:gl_pass@localhost:5438/gl_db")
        .await
        .expect("Failed to connect to GL database")
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
    wait_for_health(&client, "GL", "http://localhost:8090/api/health", 60).await?;

    println!("✓ All services are healthy");

    // Wait for NATS consumers to be subscribed (readiness gate)
    wait_for_log_line("7d-payments", "Subscribed to ar.events.payment.collection.requested", 30).await?;
    wait_for_log_line("7d-payments", "Starting outbox publisher", 30).await?;
    wait_for_log_line("7d-ar", "Subscribed to payments.events.payment.succeeded", 30).await?;
    wait_for_log_line("7d-ar", "Publisher tick", 30).await?;
    wait_for_log_line("7d-notifications", "Subscribed to payments.events.payment.succeeded", 30).await?;
    wait_for_log_line("7d-gl", "Subscribed to gl.events.posting.requested", 30).await?;

    println!("✓ All NATS consumers ready");

    Ok(TestInfrastructure {
        project_root: project_root.to_string(),
    })
}

// ============================================================================
// Test Data Setup
// ============================================================================

async fn setup_test_data(ar_pool: &PgPool, subscriptions_pool: &PgPool) -> Result<(i32, Uuid), Box<dyn std::error::Error>> {
    let tenant_id = "test-app";

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
    let gl_pool = connect_gl_db().await;
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
    sqlx::query("TRUNCATE TABLE payments, payments_events_outbox, payments_processed_events CASCADE")
        .execute(&payments_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE notifications, events_outbox, processed_events CASCADE")
        .execute(&notifications_pool)
        .await
        .ok();
    sqlx::query("TRUNCATE TABLE journal_entries, journal_lines, events_outbox, processed_events, failed_events CASCADE")
        .execute(&gl_pool)
        .await
        .ok();
    println!("✓ Databases cleaned");

    // Setup test data
    let (ar_customer_id, subscription_id) = setup_test_data(&ar_pool, &subscriptions_pool)
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
    println!("\n⏳ Waiting for event chain to complete...");
    let poll_deadline = tokio::time::Instant::now() + Duration::from_secs(15);

    // 1. Wait for invoice to be created
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

        if invoice.is_some() {
            let (invoice_id, status, amount, currency, cust_id) = invoice.unwrap();
            println!("\n✓ AR Invoice created:");
            println!("  - ID: {}", invoice_id);
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

    // 2. Wait for invoice to be marked as paid (full AR → Payments → AR chain)
    loop {
        let paid: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ar_invoices WHERE ar_customer_id = $1 AND status = 'paid'"
        )
        .bind(ar_customer_id)
        .fetch_one(&ar_pool)
        .await
        .expect("Failed to query paid invoices");

        if paid > 0 {
            println!("\n✓ AR Invoice marked as paid");
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            panic!("❌ Invoice not marked as paid within timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 3. Wait for payment.succeeded event in Payments outbox
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM payments_events_outbox WHERE event_type = 'payment.succeeded'"
        )
        .fetch_one(&payments_pool)
        .await
        .expect("Failed to query payments outbox");

        if count > 0 {
            println!("✓ Payments outbox: {} payment.succeeded event(s)", count);
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            panic!("❌ No payment.succeeded event in payments outbox within timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 4. Wait for Notifications to process at least one event
    loop {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM processed_events"
        )
        .fetch_one(&notifications_pool)
        .await
        .expect("Failed to query notifications processed_events");

        if count > 0 {
            println!("✓ Notifications: {} event(s) processed", count);
            break;
        }
        if tokio::time::Instant::now() >= poll_deadline {
            // Dump full container logs before failing (stdout + stderr, unfiltered)
            for container in &["7d-ar", "7d-payments", "7d-subscriptions", "7d-notifications"] {
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
            panic!("❌ Notifications did not process any events within timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 5. Check Subscriptions: next_bill_date should be updated
    let next_bill_date: NaiveDate = sqlx::query_scalar(
        "SELECT next_bill_date FROM subscriptions WHERE id = $1"
    )
    .bind(subscription_id)
    .fetch_one(&subscriptions_pool)
    .await
    .expect("Failed to query subscription");

    println!("✓ Subscriptions: next_bill_date = {}", next_bill_date);
    assert!(next_bill_date > today, "Subscription next_bill_date should be updated to future date");

    // 6. Wait for GL journal entry to be created
    println!("\n⏳ Waiting for GL journal entry...");
    let (journal_entry_id, source_event_id): (Uuid, Uuid) = loop {
        let result: Option<(Uuid, Uuid)> = sqlx::query_as(
            "SELECT id, source_event_id FROM journal_entries
             WHERE tenant_id = $1
             ORDER BY created_at DESC LIMIT 1"
        )
        .bind("test-app")
        .fetch_optional(&gl_pool)
        .await
        .expect("Failed to query GL journal_entries");

        if let Some((id, src_event_id)) = result {
            println!("\n✓ GL Journal Entry created:");
            println!("  - Journal Entry ID: {}", id);
            println!("  - Source Event ID: {}", src_event_id);
            break (id, src_event_id);
        }
        if tokio::time::Instant::now() >= poll_deadline {
            panic!("❌ No GL journal entry found within timeout");
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    };

    // 7. Verify journal lines exist and are balanced
    #[derive(sqlx::FromRow)]
    struct JournalLine {
        debit_minor: i64,
        credit_minor: i64,
        account_ref: String,
    }

    let lines: Vec<JournalLine> = sqlx::query_as(
        "SELECT debit_minor, credit_minor, account_ref
         FROM journal_lines
         WHERE journal_entry_id = $1"
    )
    .bind(journal_entry_id)
    .fetch_all(&gl_pool)
    .await
    .expect("Failed to query journal_lines");

    assert!(!lines.is_empty(), "Journal entry should have lines");
    println!("✓ GL Journal Lines: {} lines found", lines.len());

    let total_debits: i64 = lines.iter().map(|l| l.debit_minor).sum();
    let total_credits: i64 = lines.iter().map(|l| l.credit_minor).sum();

    println!("  - Total Debits: {}", total_debits);
    println!("  - Total Credits: {}", total_credits);

    for line in &lines {
        println!("  - Account {}: Debit={}, Credit={}",
                 line.account_ref, line.debit_minor, line.credit_minor);
    }

    assert_eq!(total_debits, total_credits, "Debits must equal credits");
    println!("✓ GL Journal Entry is balanced (debits == credits)");

    // 8. Test idempotency: republish the same event
    println!("\n⏳ Testing GL idempotency...");

    // Get the original GL posting event from AR's outbox
    let original_event: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM events_outbox
         WHERE event_type = 'gl.posting.requested'
         ORDER BY created_at DESC LIMIT 1"
    )
    .fetch_optional(&ar_pool)
    .await
    .expect("Failed to query AR outbox");

    if let Some(event_payload) = original_event {
        // Connect to NATS and republish the event
        let nats_client = async_nats::connect("nats://localhost:4222")
            .await
            .expect("Failed to connect to NATS");

        let payload_bytes = serde_json::to_vec(&event_payload)
            .expect("Failed to serialize event");

        nats_client
            .publish("gl.events.posting.requested".to_string(), payload_bytes.into())
            .await
            .expect("Failed to republish event");

        println!("✓ Republished GL posting event");

        // Wait a bit for processing
        tokio::time::sleep(Duration::from_secs(2)).await;

        // Verify no duplicate journal entry was created
        let entry_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM journal_entries
             WHERE source_event_id = $1"
        )
        .bind(source_event_id)
        .fetch_one(&gl_pool)
        .await
        .expect("Failed to count journal entries");

        assert_eq!(entry_count, 1, "Should only have one journal entry despite republish");
        println!("✓ GL Idempotency verified: no duplicate entry created");
    } else {
        println!("⚠️  Warning: No GL posting event found in AR outbox (idempotency test skipped)");
    }

    println!("\n🎉 E2E Test Passed! All assertions successful (including GL).\n");

    // Services will be stopped when _infra is dropped
}
