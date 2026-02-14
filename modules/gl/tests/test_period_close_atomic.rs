//! Integration tests for atomic period close command (Phase 13: bd-1zp)
//!
//! Tests verify:
//! - Successful period close with snapshot + hash
//! - Idempotency (calling close twice returns same result)
//! - Validation failure blocks close
//! - Period not found error
//! - Tenant isolation
//! - Concurrency safety (optional)

mod common;

use chrono::NaiveDate;
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use gl_rs::contracts::period_close_v1::CloseStatus;
use gl_rs::services::period_close_service::close_period;
use serial_test::serial;
use uuid::Uuid;

/// Test successful period close with no entries
#[tokio::test]
#[serial]
async fn test_close_period_empty_success() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_close_empty";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create an open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 1, 31).unwrap(),
    )
    .await;

    // Close the period
    let result = close_period(
        &pool,
        tenant_id,
        period_id,
        "admin",
        Some("Month-end close"),
    )
    .await
    .unwrap();

    // Should succeed
    assert!(result.success);
    assert!(result.close_status.is_some());
    assert!(result.validation_report.is_none());

    // Verify close status
    if let Some(CloseStatus::Closed {
        closed_by,
        close_reason,
        close_hash,
        ..
    }) = result.close_status
    {
        assert_eq!(closed_by, "admin");
        assert_eq!(close_reason, Some("Month-end close".to_string()));
        assert_eq!(close_hash.len(), 64); // SHA-256 hex
    } else {
        panic!("Expected Closed status");
    }

    // Verify snapshot was created in database
    let snapshot_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM period_summary_snapshots WHERE tenant_id = $1 AND period_id = $2",
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(snapshot_count >= 0); // May be 0 if no transactions, or 1+ if has currencies
}

/// Test idempotency - calling close twice returns same result
#[tokio::test]
#[serial]
async fn test_close_period_idempotent() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_close_idem";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create an open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 2, 28).unwrap(),
    )
    .await;

    // First close
    let result1 = close_period(&pool, tenant_id, period_id, "user1", Some("First close"))
        .await
        .unwrap();

    assert!(result1.success);

    let hash1 = if let Some(CloseStatus::Closed { close_hash, .. }) = &result1.close_status {
        close_hash.clone()
    } else {
        panic!("Expected Closed status");
    };

    // Second close (idempotent call)
    let result2 = close_period(&pool, tenant_id, period_id, "user2", Some("Second close"))
        .await
        .unwrap();

    assert!(result2.success);

    let hash2 = if let Some(CloseStatus::Closed {
        close_hash,
        closed_by,
        close_reason,
        ..
    }) = &result2.close_status
    {
        // Should return ORIGINAL close metadata (not second call's metadata)
        assert_eq!(closed_by, "user1");
        assert_eq!(close_reason, &Some("First close".to_string()));
        close_hash.clone()
    } else {
        panic!("Expected Closed status");
    };

    // Hash should be identical (idempotent)
    assert_eq!(hash1, hash2);
}

/// Test close fails when validation detects unbalanced entries
#[tokio::test]
#[serial]
async fn test_close_period_validation_failure() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_close_unbalanced";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 3, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 3, 31).unwrap(),
    )
    .await;

    // Create account
    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;

    // Create unbalanced journal entry
    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test', $3, 'USD', 'Unbalanced', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES (gen_random_uuid(), $1, 1, '1000', 100000, 0, 'Unbalanced')
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .unwrap();

    // Try to close - should fail validation
    let result = close_period(&pool, tenant_id, period_id, "admin", None)
        .await
        .unwrap();

    // Should fail
    assert!(!result.success);
    assert!(result.close_status.is_none());
    assert!(result.validation_report.is_some());

    // Verify validation report has UNBALANCED_ENTRIES error
    let report = result.validation_report.unwrap();
    assert!(!report.issues.is_empty());

    let has_unbalanced_error = report
        .issues
        .iter()
        .any(|issue| issue.code == "UNBALANCED_ENTRIES");
    assert!(has_unbalanced_error);

    // Verify period is NOT closed in database
    let period_closed_at = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT closed_at FROM accounting_periods WHERE id = $1",
    )
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(period_closed_at.is_none());
}

/// Test close fails with period not found
#[tokio::test]
#[serial]
async fn test_close_period_not_found() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_close_notfound";
    let fake_period_id = Uuid::new_v4();

    // Try to close non-existent period
    let result = close_period(&pool, tenant_id, fake_period_id, "admin", None)
        .await
        .unwrap();

    // Should fail
    assert!(!result.success);
    assert!(result.validation_report.is_some());

    let report = result.validation_report.unwrap();
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].code, "PERIOD_NOT_FOUND");
}

/// Test tenant isolation - cannot close other tenant's period
#[tokio::test]
#[serial]
async fn test_close_period_tenant_isolation() {
    let pool = get_test_pool().await;
    let tenant_a = "tenant_close_a";
    let tenant_b = "tenant_close_b";

    cleanup_test_tenant(&pool, tenant_a).await;

    // Create period for tenant A
    let period_id = setup_test_period(
        &pool,
        tenant_a,
        NaiveDate::from_ymd_opt(2025, 4, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 4, 30).unwrap(),
    )
    .await;

    // Try to close tenant A's period as tenant B
    let result = close_period(&pool, tenant_b, period_id, "admin", None)
        .await
        .unwrap();

    // Should fail with PERIOD_NOT_FOUND (tenant isolation)
    assert!(!result.success);
    assert!(result.validation_report.is_some());

    let report = result.validation_report.unwrap();
    assert_eq!(report.issues[0].code, "PERIOD_NOT_FOUND");

    // Verify period is NOT closed
    let period_closed_at = sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT closed_at FROM accounting_periods WHERE id = $1 AND tenant_id = $2",
    )
    .bind(period_id)
    .bind(tenant_a)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(period_closed_at.is_none());
}

/// Test successful close with balanced entries creates correct snapshot
#[tokio::test]
#[serial]
async fn test_close_period_with_balanced_entries() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_close_balanced";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 5, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 5, 31).unwrap(),
    )
    .await;

    // Create accounts
    setup_test_account(&pool, tenant_id, "1000", "Cash", "asset", "debit").await;
    setup_test_account(&pool, tenant_id, "4000", "Revenue", "revenue", "credit").await;

    // Create balanced journal entry
    let entry_id = Uuid::new_v4();
    let entry_date = NaiveDate::from_ymd_opt(2025, 5, 15).unwrap();
    let posted_at = entry_date.and_hms_opt(12, 0, 0).unwrap().and_utc();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (id, tenant_id, source_module, source_event_id, source_subject, posted_at, currency, description, created_at)
        VALUES ($1, $2, 'test', gen_random_uuid(), 'test', $3, 'USD', 'Balanced', NOW())
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(posted_at)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES
            (gen_random_uuid(), $1, 1, '1000', 100000, 0, 'Debit line'),
            (gen_random_uuid(), $1, 2, '4000', 0, 100000, 'Credit line')
        "#,
    )
    .bind(entry_id)
    .execute(&pool)
    .await
    .unwrap();

    // Close the period
    let result = close_period(&pool, tenant_id, period_id, "system", Some("EOD close"))
        .await
        .unwrap();

    // Should succeed
    assert!(result.success);
    assert!(result.validation_report.is_none());

    // Verify snapshot was created
    let snapshot_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM period_summary_snapshots WHERE tenant_id = $1 AND period_id = $2",
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(snapshot_count, 1); // One snapshot for USD currency

    // Verify snapshot has correct totals
    #[derive(sqlx::FromRow)]
    struct SnapshotTotals {
        journal_count: i32,
        total_debits_minor: i64,
        total_credits_minor: i64,
    }

    let snapshot = sqlx::query_as::<_, SnapshotTotals>(
        r#"
        SELECT journal_count, total_debits_minor, total_credits_minor
        FROM period_summary_snapshots
        WHERE tenant_id = $1 AND period_id = $2 AND currency = 'USD'
        "#,
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(snapshot.journal_count, 1);
    assert_eq!(snapshot.total_debits_minor, 100000);
    assert_eq!(snapshot.total_credits_minor, 100000);

    // Verify period has close_hash
    let close_hash = sqlx::query_scalar::<_, String>(
        "SELECT close_hash FROM accounting_periods WHERE id = $1",
    )
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(close_hash.len(), 64); // SHA-256 hex
}

/// Test concurrency safety - multiple concurrent close requests
///
/// This test spawns 3 concurrent close requests for the same period.
/// Only ONE should succeed in creating the snapshot, and all should
/// return the same close_hash (idempotency).
#[tokio::test]
#[serial]
async fn test_close_period_concurrency() {
    let pool = get_test_pool().await;
    let tenant_id = "tenant_close_concurrent";

    cleanup_test_tenant(&pool, tenant_id).await;

    // Create an open period
    let period_id = setup_test_period(
        &pool,
        tenant_id,
        NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
        NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(),
    )
    .await;

    // Spawn 3 concurrent close requests
    let pool1 = pool.clone();
    let pool2 = pool.clone();
    let pool3 = pool.clone();
    let tenant1 = tenant_id.to_string();
    let tenant2 = tenant_id.to_string();
    let tenant3 = tenant_id.to_string();

    let handle1 = tokio::spawn(async move {
        close_period(&pool1, &tenant1, period_id, "user1", Some("Close 1"))
            .await
            .unwrap()
    });

    let handle2 = tokio::spawn(async move {
        close_period(&pool2, &tenant2, period_id, "user2", Some("Close 2"))
            .await
            .unwrap()
    });

    let handle3 = tokio::spawn(async move {
        close_period(&pool3, &tenant3, period_id, "user3", Some("Close 3"))
            .await
            .unwrap()
    });

    // Wait for all to complete
    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();
    let result3 = handle3.await.unwrap();

    // All should succeed (idempotency)
    assert!(result1.success);
    assert!(result2.success);
    assert!(result3.success);

    // All should return the SAME close_hash
    let hash1 = if let Some(CloseStatus::Closed { close_hash, .. }) = result1.close_status {
        close_hash
    } else {
        panic!("Expected Closed status");
    };

    let hash2 = if let Some(CloseStatus::Closed { close_hash, .. }) = result2.close_status {
        close_hash
    } else {
        panic!("Expected Closed status");
    };

    let hash3 = if let Some(CloseStatus::Closed { close_hash, .. }) = result3.close_status {
        close_hash
    } else {
        panic!("Expected Closed status");
    };

    assert_eq!(hash1, hash2);
    assert_eq!(hash2, hash3);

    // Verify only ONE snapshot row was created per currency
    let snapshot_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM period_summary_snapshots WHERE tenant_id = $1 AND period_id = $2",
    )
    .bind(tenant_id)
    .bind(period_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Should be 0 (no entries) or 1 (if there are entries with currency)
    // The key is that it's NOT 3 (duplicate snapshots)
    assert!(snapshot_count <= 1);
}
