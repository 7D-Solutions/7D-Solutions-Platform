//! E2E Test: Reversal Chain Depth Enforcement (Phase 16: bd-1iok)
//!
//! **Purpose**: Verify max reversal chain depth = 1 invariant enforcement
//!
//! **Invariant**: reversal/supersession chains never exceed depth 1
//!
//! **Test Coverage**:
//! 1. **Valid Reversal**: Entry A reverses original entry B (depth = 1) ✓
//! 2. **Invalid Double Reversal**: Entry C attempts to reverse reversal entry A (depth = 2) ✗
//! 3. **Rejection Behavior**: Invariant check detects and rejects excessive depth
//!
//! **Failure Mode to Avoid**: Recursive reversals that make projections ambiguous

mod common;

use anyhow::Result;
use common::{cleanup_tenant_data, generate_test_tenant, get_ar_pool, get_payments_pool, get_subscriptions_pool, get_gl_pool};
use gl_rs::invariants::{assert_max_reversal_chain_depth, InvariantViolation};
use serial_test::serial;
use uuid::Uuid;

/// Test: Valid single-level reversal (depth = 1) is allowed
#[tokio::test]
#[serial]
async fn test_valid_single_level_reversal() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("\n🔍 Testing valid single-level reversal (depth = 1)\n");

    // Step 1: Create original journal entry (non-reversal)
    let original_entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency)
        VALUES ($1, $2, 'ar', $3, 'test.reversal', NULL, NOW(), 'USD')
"#,
    )
    .bind(original_entry_id)
    .bind(&tenant_id)
    .bind(source_event_id)
    .execute(&gl_pool)
    .await?;

    println!("✓ Created original entry: {}", original_entry_id);

    // Step 2: Create reversal entry (depth = 1)
    let reversal_entry_id = Uuid::new_v4();
    let reversal_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency)
        VALUES ($1, $2, 'ar', $3, 'test.reversal', $4, NOW(), 'USD')
        "#,
    )
    .bind(reversal_entry_id)
    .bind(&tenant_id)
    .bind(reversal_event_id)
    .bind(original_entry_id) // This entry reverses the original
    .execute(&gl_pool)
    .await?;

    println!("✓ Created reversal entry: {} (reverses {})", reversal_entry_id, original_entry_id);

    // Step 3: Assert invariant passes (depth = 1 is valid)
    let result = assert_max_reversal_chain_depth(&gl_pool, &tenant_id).await;

    assert!(
        result.is_ok(),
        "Single-level reversal (depth = 1) should pass invariant check"
    );

    println!("✅ Invariant check passed: single-level reversal is valid\n");

    // Cleanup
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

/// Test: Invalid double reversal (depth = 2) is rejected
#[tokio::test]
#[serial]
async fn test_invalid_double_reversal_rejected() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("\n🔍 Testing invalid double reversal (depth = 2) rejection\n");

    // Step 1: Create original journal entry (non-reversal)
    let original_entry_id = Uuid::new_v4();
    let source_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency)
        VALUES ($1, $2, 'ar', $3, 'test.reversal', NULL, NOW(), 'USD')
"#,
    )
    .bind(original_entry_id)
    .bind(&tenant_id)
    .bind(source_event_id)
    .execute(&gl_pool)
    .await?;

    println!("✓ Created original entry: {}", original_entry_id);

    // Step 2: Create first reversal entry (depth = 1) - VALID
    let first_reversal_id = Uuid::new_v4();
    let first_reversal_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency)
        VALUES ($1, $2, 'ar', $3, 'test.reversal', $4, NOW(), 'USD')
        "#,
    )
    .bind(first_reversal_id)
    .bind(&tenant_id)
    .bind(first_reversal_event_id)
    .bind(original_entry_id)
    .execute(&gl_pool)
    .await?;

    println!("✓ Created first reversal: {} (reverses {})", first_reversal_id, original_entry_id);

    // Step 3: Create second reversal (depth = 2) - INVALID (reverses a reversal)
    let second_reversal_id = Uuid::new_v4();
    let second_reversal_event_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency)
        VALUES ($1, $2, 'ar', $3, 'test.reversal', $4, NOW(), 'USD')
        "#,
    )
    .bind(second_reversal_id)
    .bind(&tenant_id)
    .bind(second_reversal_event_id)
    .bind(first_reversal_id) // This entry reverses the reversal (INVALID!)
    .execute(&gl_pool)
    .await?;

    println!("✓ Created second reversal: {} (reverses {} - INVALID!)", second_reversal_id, first_reversal_id);

    // Step 4: Assert invariant FAILS (depth = 2 is invalid)
    let result = assert_max_reversal_chain_depth(&gl_pool, &tenant_id).await;

    assert!(
        result.is_err(),
        "Double reversal (depth = 2) should fail invariant check"
    );

    match result {
        Err(InvariantViolation::ExcessiveReversalChainDepth {
            reversal_entry_id,
            original_entry_id,
            original_reverses_id,
        }) => {
            println!("✅ Invariant correctly rejected double reversal:");
            println!("   Reversal entry: {}", reversal_entry_id);
            println!("   Original entry: {} (which reverses {})", original_entry_id, original_reverses_id);
            assert_eq!(reversal_entry_id, second_reversal_id);
            assert_eq!(original_entry_id, first_reversal_id);
            assert_eq!(original_reverses_id, original_entry_id);
        }
        Err(e) => panic!("Expected ExcessiveReversalChainDepth, got: {}", e),
        Ok(_) => panic!("Expected invariant violation, but check passed"),
    }

    println!("✅ Test passed: double reversal correctly rejected\n");

    // Cleanup
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}

/// Test: Multiple independent reversals are allowed (no chaining)
#[tokio::test]
#[serial]
async fn test_multiple_independent_reversals_allowed() -> Result<()> {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;

    // Clean up tenant data before test
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    println!("\n🔍 Testing multiple independent reversals (no chaining)\n");

    // Create original entry A
    let entry_a_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency) VALUES ($1, $2, 'ar', $3, 'test.reversal', NULL, NOW(), 'USD')"
    )
    .bind(entry_a_id)
    .bind(&tenant_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await?;

    // Create original entry B
    let entry_b_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency) VALUES ($1, $2, 'ar', $3, 'test.reversal', NULL, NOW(), 'USD')"
    )
    .bind(entry_b_id)
    .bind(&tenant_id)
    .bind(Uuid::new_v4())
    .execute(&gl_pool)
    .await?;

    // Create reversal of A
    let reversal_a_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency) VALUES ($1, $2, 'ar', $3, 'test.reversal', $4, NOW(), 'USD')"
    )
    .bind(reversal_a_id)
    .bind(&tenant_id)
    .bind(Uuid::new_v4())
    .bind(entry_a_id)
    .execute(&gl_pool)
    .await?;

    // Create reversal of B
    let reversal_b_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, reverses_entry_id, posted_at, currency) VALUES ($1, $2, 'ar', $3, 'test.reversal', $4, NOW(), 'USD')"
    )
    .bind(reversal_b_id)
    .bind(&tenant_id)
    .bind(Uuid::new_v4())
    .bind(entry_b_id)
    .execute(&gl_pool)
    .await?;

    println!("✓ Created 2 original entries and 2 independent reversals");

    // Assert invariant passes (multiple independent reversals are valid)
    let result = assert_max_reversal_chain_depth(&gl_pool, &tenant_id).await;

    assert!(
        result.is_ok(),
        "Multiple independent reversals should pass invariant check"
    );

    println!("✅ Invariant check passed: multiple independent reversals are valid\n");

    // Cleanup
    cleanup_tenant_data(&ar_pool, &payments_pool, &subscriptions_pool, &gl_pool, &tenant_id)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
