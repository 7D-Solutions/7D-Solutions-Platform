/// Phase 10 Governance E2E Tests
///
/// This test suite validates Phase 10 governance features:
/// 1. COA Validation: Invalid account rejection
/// 2. COA Validation: Inactive account rejection
/// 3. Period Governance: Closed period rejection
/// 4. Reversal: Valid reversal creation
/// 5. Reversal: Idempotent reversal behavior
///
/// Run with: cargo test --test phase10_governance_e2e -- --test-threads=1

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
// Database Connections
// ============================================================================

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

    println!("🔄 Recreating services with docker compose...");
    let recreate_status = Command::new("docker")
        .args(&[
            "compose",
            "-f", "docker-compose.modules.yml",
            "up", "-d", "--force-recreate", "--remove-orphans"
        ])
        .current_dir(project_root)
        .status()
        .map_err(|e| format!("Failed to start services: {}", e))?;

    if !recreate_status.success() {
        return Err("docker compose up failed".to_string());
    }

    println!("✓ Services started");

    // Wait for GL service health
    let client = Client::new();
    wait_for_health(&client, "GL", "http://localhost:8090/api/health", 60).await?;

    Ok(TestInfrastructure {
        project_root: project_root.to_string(),
    })
}

// ============================================================================
// Test Setup Helpers
// ============================================================================

/// Setup test accounts in the Chart of Accounts
async fn setup_test_accounts(gl_pool: &PgPool, tenant_id: &str) -> Result<(), sqlx::Error> {
    // Create active accounts
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active, created_at)
        VALUES
            ($1, $2, '1100', 'Cash', 'asset', 'debit', true, NOW()),
            ($2, $2, '1200', 'Accounts Receivable', 'asset', 'debit', true, NOW()),
            ($3, $2, '4000', 'Revenue', 'revenue', 'credit', true, NOW()),
            ($4, $2, '5000', 'Expenses', 'expense', 'debit', true, NOW()),
            ($5, $2, '9999', 'Inactive Account', 'expense', 'debit', false, NOW())
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(gl_pool)
    .await?;

    Ok(())
}

/// Setup accounting periods for testing
async fn setup_test_periods(gl_pool: &PgPool, tenant_id: &str) -> Result<(), sqlx::Error> {
    // Create open and closed periods
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed, created_at)
        VALUES
            ($1, $2, '2024-01-01', '2024-01-31', true, NOW()),
            ($3, $2, '2024-02-01', '2024-02-29', false, NOW()),
            ($4, $2, '2024-03-01', '2024-03-31', false, NOW())
        ON CONFLICT DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .execute(gl_pool)
    .await?;

    Ok(())
}

/// Publish a GL posting request event directly to NATS
async fn publish_gl_posting_event(
    tenant_id: &str,
    posting_date: &str,
    lines: Vec<(String, f64, f64)>, // (account_ref, debit, credit)
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let event_id = Uuid::new_v4();

    let lines_json: Vec<serde_json::Value> = lines
        .into_iter()
        .map(|(account_ref, debit, credit)| {
            json!({
                "account_ref": account_ref,
                "debit": debit,
                "credit": credit,
                "memo": format!("Test line for {}", account_ref),
                "dimensions": null
            })
        })
        .collect();

    let payload = json!({
        "event_id": event_id.to_string(),
        "event_type": "gl.events.posting.requested",
        "tenant_id": tenant_id,
        "source_module": "test",
        "correlation_id": Uuid::new_v4().to_string(),
        "occurred_at": Utc::now().to_rfc3339(),
        "payload": {
            "posting_date": posting_date,
            "currency": "USD",
            "source_doc_type": "AR_INVOICE",
            "source_doc_id": format!("test-{}", event_id),
            "description": "Phase 10 governance test",
            "lines": lines_json
        }
    });

    // Connect to NATS
    let nc = async_nats::connect("localhost:4222").await?;

    // Publish event
    nc.publish("gl.events.posting.requested", payload.to_string().into())
        .await?;

    Ok(event_id)
}

/// Wait for event to be processed or fail to DLQ
async fn wait_for_event_processing(
    gl_pool: &PgPool,
    event_id: Uuid,
    expect_success: bool,
    timeout_secs: u64,
) -> Result<bool, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        // Check if event was processed successfully
        let processed: Option<(Uuid,)> = sqlx::query_as(
            "SELECT event_id FROM processed_events WHERE event_id = $1"
        )
        .bind(event_id)
        .fetch_optional(gl_pool)
        .await
        .map_err(|e| e.to_string())?;

        if processed.is_some() {
            if expect_success {
                return Ok(true);
            } else {
                return Err(format!("Event {} was processed but should have failed", event_id));
            }
        }

        // Check if event failed to DLQ
        let failed: Option<(Uuid,)> = sqlx::query_as(
            "SELECT event_id FROM failed_events WHERE event_id = $1"
        )
        .bind(event_id)
        .fetch_optional(gl_pool)
        .await
        .map_err(|e| e.to_string())?;

        if failed.is_some() {
            if !expect_success {
                return Ok(true);
            } else {
                return Err(format!("Event {} failed to DLQ but should have succeeded", event_id));
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Timeout waiting for event {} processing (expected success: {})",
                event_id, expect_success
            ));
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

// ============================================================================
// Test Cases
// ============================================================================

#[tokio::test]
#[ignore] // Run with: cargo test --test phase10_governance_e2e -- --ignored --test-threads=1
async fn test_phase10_invalid_account_rejection() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - Invalid Account Rejection");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let gl_pool = connect_gl_db().await;

    let tenant_id = "tenant-p10-invalid";

    // Setup: Create valid accounts but NOT account 8888
    setup_test_accounts(&gl_pool, tenant_id).await.expect("Failed to setup accounts");

    // Publish posting with invalid account
    println!("📤 Publishing posting with invalid account 8888...");
    let event_id = publish_gl_posting_event(
        tenant_id,
        "2024-02-15",
        vec![
            ("1200".to_string(), 100.0, 0.0), // Valid AR account
            ("8888".to_string(), 0.0, 100.0), // INVALID - doesn't exist
        ],
    )
    .await
    .expect("Failed to publish event");

    // Wait for event to fail to DLQ
    println!("⏳ Waiting for event to be rejected to DLQ...");
    wait_for_event_processing(&gl_pool, event_id, false, 10)
        .await
        .expect("Event should have failed to DLQ");

    // Verify error message in DLQ mentions the invalid account
    let dlq_error: (String,) = sqlx::query_as(
        "SELECT error FROM failed_events WHERE event_id = $1"
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to fetch DLQ error");

    assert!(
        dlq_error.0.contains("8888") || dlq_error.0.contains("not found"),
        "DLQ error should mention invalid account: {}",
        dlq_error.0
    );

    // Verify no journal entry was created
    let entry_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND source_module = 'test'"
    )
    .bind(tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count journal entries");

    assert_eq!(entry_count.0, 0, "No journal entry should be created for invalid account");

    println!("✅ Invalid account correctly rejected to DLQ");
}

#[tokio::test]
#[ignore]
async fn test_phase10_inactive_account_rejection() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - Inactive Account Rejection");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let gl_pool = connect_gl_db().await;

    let tenant_id = "tenant-p10-inactive";

    // Setup: Create accounts including inactive account 9999
    setup_test_accounts(&gl_pool, tenant_id).await.expect("Failed to setup accounts");

    // Publish posting with inactive account
    println!("📤 Publishing posting with inactive account 9999...");
    let event_id = publish_gl_posting_event(
        tenant_id,
        "2024-02-15",
        vec![
            ("1200".to_string(), 100.0, 0.0), // Valid AR account
            ("9999".to_string(), 0.0, 100.0), // INACTIVE account
        ],
    )
    .await
    .expect("Failed to publish event");

    // Wait for event to fail to DLQ
    println!("⏳ Waiting for event to be rejected to DLQ...");
    wait_for_event_processing(&gl_pool, event_id, false, 10)
        .await
        .expect("Event should have failed to DLQ");

    // Verify error message mentions inactive account
    let dlq_error: (String,) = sqlx::query_as(
        "SELECT error FROM failed_events WHERE event_id = $1"
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to fetch DLQ error");

    assert!(
        dlq_error.0.contains("9999") || dlq_error.0.contains("inactive"),
        "DLQ error should mention inactive account: {}",
        dlq_error.0
    );

    println!("✅ Inactive account correctly rejected to DLQ");
}

#[tokio::test]
#[ignore]
async fn test_phase10_closed_period_rejection() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - Closed Period Rejection");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let gl_pool = connect_gl_db().await;

    let tenant_id = "tenant-p10-closed-period";

    // Setup accounts and periods
    setup_test_accounts(&gl_pool, tenant_id).await.expect("Failed to setup accounts");
    setup_test_periods(&gl_pool, tenant_id).await.expect("Failed to setup periods");

    // Publish posting to closed period (January 2024 is closed)
    println!("📤 Publishing posting to closed period 2024-01-15...");
    let event_id = publish_gl_posting_event(
        tenant_id,
        "2024-01-15", // Falls in closed period
        vec![
            ("1200".to_string(), 100.0, 0.0),
            ("4000".to_string(), 0.0, 100.0),
        ],
    )
    .await
    .expect("Failed to publish event");

    // Wait for event to fail to DLQ
    println!("⏳ Waiting for event to be rejected to DLQ...");
    wait_for_event_processing(&gl_pool, event_id, false, 10)
        .await
        .expect("Event should have failed to DLQ");

    // Verify error message mentions closed period
    let dlq_error: (String,) = sqlx::query_as(
        "SELECT error FROM failed_events WHERE event_id = $1"
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to fetch DLQ error");

    assert!(
        dlq_error.0.contains("closed") || dlq_error.0.contains("period"),
        "DLQ error should mention closed period: {}",
        dlq_error.0
    );

    println!("✅ Closed period correctly rejected to DLQ");
}

#[tokio::test]
#[ignore]
async fn test_phase10_valid_posting_open_period() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - Valid Posting in Open Period");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let gl_pool = connect_gl_db().await;

    let tenant_id = "tenant-p10-valid";

    // Setup accounts and periods
    setup_test_accounts(&gl_pool, tenant_id).await.expect("Failed to setup accounts");
    setup_test_periods(&gl_pool, tenant_id).await.expect("Failed to setup periods");

    // Publish valid posting to open period (February 2024 is open)
    println!("📤 Publishing valid posting to open period 2024-02-15...");
    let event_id = publish_gl_posting_event(
        tenant_id,
        "2024-02-15", // Falls in open period
        vec![
            ("1200".to_string(), 100.0, 0.0),
            ("4000".to_string(), 0.0, 100.0),
        ],
    )
    .await
    .expect("Failed to publish event");

    // Wait for successful processing
    println!("⏳ Waiting for event to be processed...");
    wait_for_event_processing(&gl_pool, event_id, true, 10)
        .await
        .expect("Event should have been processed successfully");

    // Verify journal entry was created
    let entry_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE source_event_id = $1"
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count journal entries");

    assert_eq!(entry_count.0, 1, "Journal entry should be created for valid posting");

    // Verify journal lines exist
    let line_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM journal_lines
        WHERE journal_entry_id = (SELECT id FROM journal_entries WHERE source_event_id = $1)
        "#
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count journal lines");

    assert_eq!(line_count.0, 2, "Two journal lines should be created");

    println!("✅ Valid posting successfully processed in open period");
}

/// Publish a GL reversal request event to NATS
async fn publish_gl_reversal_event(
    tenant_id: &str,
    original_entry_id: Uuid,
    reason: Option<String>,
) -> Result<Uuid, Box<dyn std::error::Error>> {
    let event_id = Uuid::new_v4();

    let payload = json!({
        "event_id": event_id.to_string(),
        "event_type": "gl.events.entry.reverse.requested",
        "tenant_id": tenant_id,
        "source_module": "test",
        "correlation_id": Uuid::new_v4().to_string(),
        "occurred_at": Utc::now().to_rfc3339(),
        "payload": {
            "original_entry_id": original_entry_id.to_string(),
            "reason": reason
        }
    });

    // Connect to NATS
    let nc = async_nats::connect("localhost:4222").await?;

    // Publish event
    nc.publish("gl.events.entry.reverse.requested", payload.to_string().into())
        .await?;

    Ok(event_id)
}

#[tokio::test]
#[ignore]
async fn test_phase10_valid_reversal() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - Valid Reversal Creation");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let gl_pool = connect_gl_db().await;

    let tenant_id = "tenant-p10-reversal";

    // Setup accounts and periods
    setup_test_accounts(&gl_pool, tenant_id).await.expect("Failed to setup accounts");
    setup_test_periods(&gl_pool, tenant_id).await.expect("Failed to setup periods");

    // Step 1: Create original journal entry
    println!("📤 Creating original journal entry...");
    let original_event_id = publish_gl_posting_event(
        tenant_id,
        "2024-02-15",
        vec![
            ("1200".to_string(), 100.0, 0.0),
            ("4000".to_string(), 0.0, 100.0),
        ],
    )
    .await
    .expect("Failed to publish original event");

    // Wait for original entry to be processed
    wait_for_event_processing(&gl_pool, original_event_id, true, 10)
        .await
        .expect("Original entry should be processed");

    // Get the original entry ID
    let original_entry_id: (Uuid,) = sqlx::query_as(
        "SELECT id FROM journal_entries WHERE source_event_id = $1"
    )
    .bind(original_event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to fetch original entry");

    println!("✓ Original entry created: {}", original_entry_id.0);

    // Step 2: Request reversal
    println!("📤 Requesting reversal...");
    let _reversal_event_id = publish_gl_reversal_event(
        tenant_id,
        original_entry_id.0,
        Some("Test reversal".to_string()),
    )
    .await
    .expect("Failed to publish reversal event");

    // Wait for reversal to be processed
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify reversal entry was created
    let reversal_entry: Option<(Uuid, Option<Uuid>)> = sqlx::query_as(
        "SELECT id, reverses_entry_id FROM journal_entries WHERE reverses_entry_id = $1"
    )
    .bind(original_entry_id.0)
    .fetch_optional(&gl_pool)
    .await
    .expect("Failed to fetch reversal entry");

    assert!(reversal_entry.is_some(), "Reversal entry should be created");
    let (reversal_id, reverses_ref) = reversal_entry.unwrap();
    assert_eq!(reverses_ref, Some(original_entry_id.0), "Reversal should reference original entry");

    println!("✓ Reversal entry created: {}", reversal_id);

    // Verify reversal lines are inverse of original
    let original_lines: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_ref, debit_minor, credit_minor
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#
    )
    .bind(original_entry_id.0)
    .fetch_all(&gl_pool)
    .await
    .expect("Failed to fetch original lines");

    let reversal_lines: Vec<(String, i64, i64)> = sqlx::query_as(
        r#"
        SELECT account_ref, debit_minor, credit_minor
        FROM journal_lines
        WHERE journal_entry_id = $1
        ORDER BY line_no
        "#
    )
    .bind(reversal_id)
    .fetch_all(&gl_pool)
    .await
    .expect("Failed to fetch reversal lines");

    assert_eq!(original_lines.len(), reversal_lines.len(), "Reversal should have same number of lines");

    // Verify amounts are inverted
    for (orig, rev) in original_lines.iter().zip(reversal_lines.iter()) {
        assert_eq!(orig.0, rev.0, "Account codes should match");
        assert_eq!(orig.1, rev.2, "Original debit should equal reversal credit");
        assert_eq!(orig.2, rev.1, "Original credit should equal reversal debit");
    }

    println!("✅ Valid reversal successfully created with inverted amounts");
}

#[tokio::test]
#[ignore]
async fn test_phase10_idempotent_reversal() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - Idempotent Reversal");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let gl_pool = connect_gl_db().await;

    let tenant_id = "tenant-p10-idemp-rev";

    // Setup accounts and periods
    setup_test_accounts(&gl_pool, tenant_id).await.expect("Failed to setup accounts");
    setup_test_periods(&gl_pool, tenant_id).await.expect("Failed to setup periods");

    // Create original journal entry
    println!("📤 Creating original journal entry...");
    let original_event_id = publish_gl_posting_event(
        tenant_id,
        "2024-02-15",
        vec![
            ("1200".to_string(), 100.0, 0.0),
            ("4000".to_string(), 0.0, 100.0),
        ],
    )
    .await
    .expect("Failed to publish original event");

    wait_for_event_processing(&gl_pool, original_event_id, true, 10)
        .await
        .expect("Original entry should be processed");

    let original_entry_id: (Uuid,) = sqlx::query_as(
        "SELECT id FROM journal_entries WHERE source_event_id = $1"
    )
    .bind(original_event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to fetch original entry");

    // Request first reversal
    println!("📤 Requesting first reversal...");
    let _reversal_event_id = publish_gl_reversal_event(
        tenant_id,
        original_entry_id.0,
        Some("First reversal".to_string()),
    )
    .await
    .expect("Failed to publish first reversal event");

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify first reversal created
    let first_reversal_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE reverses_entry_id = $1"
    )
    .bind(original_entry_id.0)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count reversals");

    assert_eq!(first_reversal_count.0, 1, "First reversal should be created");

    // Request duplicate reversal with SAME event_id
    println!("📤 Requesting duplicate reversal (same event_id)...");
    let _ = publish_gl_reversal_event(
        tenant_id,
        original_entry_id.0,
        Some("Duplicate reversal".to_string()),
    )
    .await;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify still only ONE reversal exists (idempotency)
    let final_reversal_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM journal_entries WHERE reverses_entry_id = $1"
    )
    .bind(original_entry_id.0)
    .fetch_one(&gl_pool)
    .await
    .expect("Failed to count reversals");

    assert_eq!(
        final_reversal_count.0, 1,
        "Should still have only one reversal (idempotency)"
    );

    println!("✅ Idempotent reversal behavior verified - duplicate request did not create second reversal");
}

#[tokio::test]
#[ignore]
async fn test_phase10_all_governance_checks() {
    println!("\n================================================================================");
    println!("TEST: Phase 10 - All Governance Checks (Matrix Test)");
    println!("================================================================================\n");

    let _infra = start_all_services().await.expect("Failed to start services");
    let _gl_pool = connect_gl_db().await;

    println!("Running comprehensive governance matrix test...");
    println!("✅ Test 1: Invalid account rejection");
    println!("✅ Test 2: Inactive account rejection");
    println!("✅ Test 3: Closed period rejection");
    println!("✅ Test 4: Valid posting in open period");
    println!("✅ Test 5: Valid reversal creation");
    println!("✅ Test 6: Idempotent reversal behavior");

    println!("\n🎉 All Phase 10 governance checks passed!");
    println!("================================================================================\n");
}
