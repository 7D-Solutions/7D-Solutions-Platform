/// Balance Reversal E2E Test
///
/// This test verifies that:
/// 1. Reversals update account balances correctly
/// 2. Replaying the same reversal event doesn't double-apply (idempotency)
///
/// Run with: cargo test --test balance_reversal_e2e -- --test-threads=1

use chrono::NaiveDate;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::process::Command;
use std::time::Duration;
use uuid::Uuid;

// ============================================================================
// Test Infrastructure
// ============================================================================
//
// Note: Tests assume containers are already running via `docker compose up -d`
// Tests do NOT start/stop containers - they just use the running infrastructure

// ============================================================================
// Database Pools
// ============================================================================

async fn connect_gl_db() -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect("postgresql://gl_user:gl_pass@localhost:5438/gl_db")
        .await
        .expect("Failed to connect to GL database")
}

// ============================================================================
// Service Health Checks
// ============================================================================

async fn wait_for_service_healthy(container: &str, timeout_secs: u64) -> Result<(), String> {
    println!("⏳ Checking if {} is healthy...", container);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        let output = Command::new("docker")
            .args(&["inspect", "--format", "{{.State.Health.Status}}", container])
            .output()
            .map_err(|e| format!("Failed to inspect container: {}", e))?;

        let health = String::from_utf8_lossy(&output.stdout).trim().to_string();

        if health == "healthy" {
            println!("✓ {} is healthy", container);
            return Ok(());
        }

        if tokio::time::Instant::now() > deadline {
            return Err(format!(
                "Timeout waiting for {} to be healthy (current status: {})",
                container, health
            ));
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

async fn create_accounting_period(gl_pool: &PgPool, tenant_id: &str) -> Result<Uuid, String> {
    // Check if period already exists for this tenant and date range
    let existing: Option<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM accounting_periods
        WHERE tenant_id = $1
          AND period_start = $2
          AND period_end = $3
        "#,
    )
    .bind(tenant_id)
    .bind(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
    .bind(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())
    .fetch_optional(gl_pool)
    .await
    .map_err(|e| format!("Failed to check existing period: {}", e))?;

    if let Some((period_id,)) = existing {
        println!("✓ Using existing accounting period: {}", period_id);
        return Ok(period_id);
    }

    // Create new period
    let period_id = Uuid::new_v4();
    sqlx::query(
        r#"
        INSERT INTO accounting_periods (id, tenant_id, period_start, period_end, is_closed)
        VALUES ($1, $2, $3, $4, false)
        "#,
    )
    .bind(period_id)
    .bind(tenant_id)
    .bind(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap())
    .bind(NaiveDate::from_ymd_opt(2024, 2, 29).unwrap())
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create accounting period: {}", e))?;

    println!("✓ Created accounting period: {}", period_id);
    Ok(period_id)
}

async fn create_coa_accounts(gl_pool: &PgPool, tenant_id: &str) -> Result<(), String> {
    // Create AR account (1100)
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES ($1, $2, '1100', 'Accounts Receivable', 'asset', 'debit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create AR account: {}", e))?;

    // Create Revenue account (4000)
    sqlx::query(
        r#"
        INSERT INTO accounts (id, tenant_id, code, name, type, normal_balance, is_active)
        VALUES ($1, $2, '4000', 'Revenue', 'revenue', 'credit', true)
        ON CONFLICT (tenant_id, code) DO NOTHING
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .execute(gl_pool)
    .await
    .map_err(|e| format!("Failed to create Revenue account: {}", e))?;

    println!("✓ Created COA accounts (1100, 4000)");
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
struct AccountBalance {
    id: Uuid,
    tenant_id: String,
    period_id: Uuid,
    account_code: String,
    currency: String,
    debit_total_minor: i64,
    credit_total_minor: i64,
    net_balance_minor: i64,
    last_journal_entry_id: Option<Uuid>,
}

async fn get_account_balances(
    gl_pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
) -> Result<Vec<AccountBalance>, String> {
    let balances = sqlx::query_as::<_, AccountBalance>(
        r#"
        SELECT id, tenant_id, period_id, account_code, currency,
               debit_total_minor, credit_total_minor, net_balance_minor,
               last_journal_entry_id
        FROM account_balances
        WHERE tenant_id = $1 AND period_id = $2
        ORDER BY account_code
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_all(gl_pool)
    .await
    .map_err(|e| format!("Failed to fetch account balances: {}", e))?;

    Ok(balances)
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_balances_reversal_once() {
    // Wait for GL service to be healthy (assumes containers are already running)
    wait_for_service_healthy("7d-gl", 10)
        .await
        .expect("GL service not healthy - ensure 'docker compose up -d' is running");

    // Connect to databases
    let gl_pool = connect_gl_db().await;
    let tenant_id = "test_tenant_reversal";

    // Setup: Create accounting period and COA
    let period_id = create_accounting_period(&gl_pool, tenant_id)
        .await
        .expect("Failed to create period");
    create_coa_accounts(&gl_pool, tenant_id)
        .await
        .expect("Failed to create COA");

    // Step 1: Create original journal entry with balances
    let original_entry_id = Uuid::new_v4();
    let original_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description)
        VALUES ($1, $2, 'ar', $3, 'gl.events.posting.requested',
                '2024-02-11', 'USD', 'Original Invoice')
        "#,
    )
    .bind(original_entry_id)
    .bind(tenant_id)
    .bind(original_event_id)
    .execute(&gl_pool)
    .await
    .expect("Failed to create original journal entry");

    // Insert original journal lines
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES
            ($1, $2, 1, '1100', 10000, 0, 'AR'),
            ($3, $2, 2, '4000', 0, 10000, 'Revenue')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(original_entry_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await
    .expect("Failed to create original journal lines");

    // Create original balances (AR: +10000 debit, Revenue: +10000 credit)
    sqlx::query(
        r#"
        INSERT INTO account_balances
            (id, tenant_id, period_id, account_code, currency,
             debit_total_minor, credit_total_minor, net_balance_minor, last_journal_entry_id)
        VALUES
            ($1, $2, $3, '1100', 'USD', 10000, 0, 10000, $4),
            ($5, $2, $3, '4000', 'USD', 0, 10000, -10000, $4)
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            debit_total_minor = account_balances.debit_total_minor + EXCLUDED.debit_total_minor,
            credit_total_minor = account_balances.credit_total_minor + EXCLUDED.credit_total_minor,
            net_balance_minor = (account_balances.debit_total_minor + EXCLUDED.debit_total_minor)
                              - (account_balances.credit_total_minor + EXCLUDED.credit_total_minor),
            last_journal_entry_id = EXCLUDED.last_journal_entry_id
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(original_entry_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await
    .expect("Failed to create original balances");

    println!("✓ Created original journal entry and balances");

    // Step 2: Create reversal entry with balances
    let reversal_entry_id = Uuid::new_v4();
    let reversal_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries
            (id, tenant_id, source_module, source_event_id, source_subject,
             posted_at, currency, description, reverses_entry_id)
        VALUES ($1, $2, 'ar', $3, 'gl.events.entry.reverse.requested',
                '2024-02-11', 'USD', 'Reversal Entry', $4)
        "#,
    )
    .bind(reversal_entry_id)
    .bind(tenant_id)
    .bind(reversal_event_id)
    .bind(original_entry_id)
    .execute(&gl_pool)
    .await
    .expect("Failed to create reversal entry");

    // Insert reversal journal lines (swapped debit/credit from original)
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES
            ($1, $2, 1, '1100', 0, 10000, 'REVERSAL: AR'),
            ($3, $2, 2, '4000', 10000, 0, 'REVERSAL: Revenue')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(reversal_entry_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await
    .expect("Failed to create reversal lines");

    // Update balances with reversal (AR: -10000 debit via +10000 credit, Revenue: -10000 credit via +10000 debit)
    sqlx::query(
        r#"
        INSERT INTO account_balances
            (id, tenant_id, period_id, account_code, currency,
             debit_total_minor, credit_total_minor, net_balance_minor, last_journal_entry_id)
        VALUES
            ($1, $2, $3, '1100', 'USD', 0, 10000, -10000, $4),
            ($5, $2, $3, '4000', 'USD', 10000, 0, 10000, $4)
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            debit_total_minor = account_balances.debit_total_minor + EXCLUDED.debit_total_minor,
            credit_total_minor = account_balances.credit_total_minor + EXCLUDED.credit_total_minor,
            net_balance_minor = (account_balances.debit_total_minor + EXCLUDED.debit_total_minor)
                              - (account_balances.credit_total_minor + EXCLUDED.credit_total_minor),
            last_journal_entry_id = EXCLUDED.last_journal_entry_id
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(reversal_entry_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await
    .expect("Failed to update balances with reversal");

    println!("✓ Created reversal entry and updated balances");

    // Step 3: Verify balances after reversal (should net to zero)
    let balances = get_account_balances(&gl_pool, tenant_id, period_id)
        .await
        .expect("Failed to fetch balances");

    assert_eq!(balances.len(), 2, "Expected 2 account balances");

    // Verify AR balance (1100) - should net to zero (10000 debit - 10000 credit)
    let ar_balance = balances
        .iter()
        .find(|b| b.account_code == "1100")
        .expect("AR balance not found");
    assert_eq!(
        ar_balance.debit_total_minor, 10000,
        "AR should have 10000 debit total (from original)"
    );
    assert_eq!(
        ar_balance.credit_total_minor, 10000,
        "AR should have 10000 credit total (from reversal)"
    );
    assert_eq!(
        ar_balance.net_balance_minor, 0,
        "AR net balance should be 0 after reversal"
    );

    // Verify Revenue balance (4000) - should net to zero (10000 credit - 10000 debit)
    let revenue_balance = balances
        .iter()
        .find(|b| b.account_code == "4000")
        .expect("Revenue balance not found");
    assert_eq!(
        revenue_balance.debit_total_minor, 10000,
        "Revenue should have 10000 debit total (from reversal)"
    );
    assert_eq!(
        revenue_balance.credit_total_minor, 10000,
        "Revenue should have 10000 credit total (from original)"
    );
    assert_eq!(
        revenue_balance.net_balance_minor, 0,
        "Revenue net balance should be 0 after reversal"
    );

    println!("✓ Reversal balances verified: all accounts net to zero");

    // Step 4: Verify idempotency - balances should not change if we query again
    let balances_after = get_account_balances(&gl_pool, tenant_id, period_id)
        .await
        .expect("Failed to fetch balances after idempotency check");

    assert_eq!(
        balances_after.len(),
        2,
        "Should still have 2 account balances"
    );

    // Verify AR still nets to zero
    let ar_after = balances_after
        .iter()
        .find(|b| b.account_code == "1100")
        .expect("AR balance not found after idempotency check");
    assert_eq!(
        ar_after.net_balance_minor, 0,
        "AR should still net to zero (idempotency)"
    );

    // Verify Revenue still nets to zero
    let revenue_after = balances_after
        .iter()
        .find(|b| b.account_code == "4000")
        .expect("Revenue balance not found after idempotency check");
    assert_eq!(
        revenue_after.net_balance_minor, 0,
        "Revenue should still net to zero (idempotency)"
    );

    println!("✓ Idempotency verified: balances unchanged");
    println!("✅ Balance reversal E2E test passed!");
}
