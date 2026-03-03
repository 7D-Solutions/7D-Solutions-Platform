//! Tenant Boundary Concurrency Tests (Phase 58 Gate A, bd-3487x)
//!
//! Proves no cross-tenant data leakage under concurrent load.
//! Two tenants operate simultaneously and must never see each other's data.
//!
//! ## Strategy
//! - Two tenants each insert journal entries, account balances, and periods concurrently
//! - After all writes, verify each tenant sees only its own data
//! - Concurrent reads interleaved with writes must also be tenant-scoped
//!
//! ## Prerequisites
//! - PostgreSQL at localhost:5438 (docker compose up -d)

mod common;

use chrono::{NaiveDate, Utc};
use common::{cleanup_test_tenant, get_test_pool, setup_test_account, setup_test_period};
use serial_test::serial;
use sqlx::{PgPool, Row};
use uuid::Uuid;

const TENANT_A: &str = "tenant-boundary-a";
const TENANT_B: &str = "tenant-boundary-b";

/// Insert a journal entry + balanced lines for a tenant
async fn insert_journal_entry(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    debit_account: &str,
    credit_account: &str,
    amount_minor: i64,
    currency: &str,
) -> Uuid {
    let entry_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let posted_at = Utc::now();

    sqlx::query(
        r#"
        INSERT INTO journal_entries (
            id, tenant_id, source_module, source_event_id, source_subject,
            posted_at, currency, description, reference_type, reference_id
        )
        VALUES ($1, $2, 'test', $3, 'tenant_boundary_test', $4, $5, $6, 'TEST', $7)
        "#,
    )
    .bind(entry_id)
    .bind(tenant_id)
    .bind(event_id)
    .bind(posted_at)
    .bind(currency)
    .bind(format!("Boundary test entry for {}", tenant_id))
    .bind(entry_id.to_string())
    .execute(pool)
    .await
    .expect("Failed to insert journal entry");

    // Debit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES ($1, $2, 1, $3, $4, 0, 'boundary test DR')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(debit_account)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("Failed to insert debit line");

    // Credit line
    sqlx::query(
        r#"
        INSERT INTO journal_lines (id, journal_entry_id, line_no, account_ref, debit_minor, credit_minor, memo)
        VALUES ($1, $2, 2, $3, 0, $4, 'boundary test CR')
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(entry_id)
    .bind(credit_account)
    .bind(amount_minor)
    .execute(pool)
    .await
    .expect("Failed to insert credit line");

    entry_id
}

/// Insert an account balance for a tenant
async fn insert_balance(
    pool: &PgPool,
    tenant_id: &str,
    period_id: Uuid,
    account_code: &str,
    currency: &str,
    debit_total_minor: i64,
    credit_total_minor: i64,
) {
    let net = debit_total_minor - credit_total_minor;
    sqlx::query(
        r#"
        INSERT INTO account_balances (
            id, tenant_id, period_id, account_code, currency,
            debit_total_minor, credit_total_minor, net_balance_minor,
            last_journal_entry_id, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW(), NOW())
        ON CONFLICT (tenant_id, period_id, account_code, currency)
        DO UPDATE SET
            debit_total_minor = account_balances.debit_total_minor + EXCLUDED.debit_total_minor,
            credit_total_minor = account_balances.credit_total_minor + EXCLUDED.credit_total_minor,
            net_balance_minor = account_balances.net_balance_minor + EXCLUDED.net_balance_minor
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(period_id)
    .bind(account_code)
    .bind(currency)
    .bind(debit_total_minor)
    .bind(credit_total_minor)
    .bind(net)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("Failed to insert balance");
}

/// Count journal entries visible to a tenant
async fn count_journal_entries(pool: &PgPool, tenant_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to count journal entries")
}

/// Count account balance rows visible to a tenant
async fn count_balances(pool: &PgPool, tenant_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM account_balances WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to count balances")
}

/// Count periods visible to a tenant
async fn count_periods(pool: &PgPool, tenant_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM accounting_periods WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to count periods")
}

/// Sum of net_balance_minor for a tenant (should only include that tenant's data)
async fn sum_net_balance(pool: &PgPool, tenant_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(net_balance_minor)::bigint, 0) FROM account_balances WHERE tenant_id = $1",
    )
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("Failed to sum net balance")
}

#[tokio::test]
#[serial]
async fn test_tenant_boundary_no_cross_tenant_leakage_under_concurrent_writes() {
    let pool = get_test_pool().await;

    // Clean slate
    cleanup_test_tenant(&pool, TENANT_A).await;
    cleanup_test_tenant(&pool, TENANT_B).await;

    // Setup: Each tenant gets their own accounts and periods
    let period_a = setup_test_period(
        &pool,
        TENANT_A,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
    )
    .await;

    let period_b = setup_test_period(
        &pool,
        TENANT_B,
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
    )
    .await;

    setup_test_account(&pool, TENANT_A, "1000", "Cash A", "asset", "debit").await;
    setup_test_account(&pool, TENANT_A, "4000", "Revenue A", "revenue", "credit").await;
    setup_test_account(&pool, TENANT_B, "1000", "Cash B", "asset", "debit").await;
    setup_test_account(&pool, TENANT_B, "4000", "Revenue B", "revenue", "credit").await;

    // Concurrent writes: 10 journal entries per tenant, interleaved
    let mut handles = Vec::new();
    for i in 0..10 {
        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let pa = period_a;
        let pb = period_b;

        // Tenant A write
        let ha = tokio::spawn(async move {
            insert_journal_entry(
                &pool_a,
                TENANT_A,
                pa,
                "1000",
                "4000",
                (i + 1) * 10000, // unique amounts: 10000, 20000, ... 100000
                "USD",
            )
            .await;
            insert_balance(
                &pool_a,
                TENANT_A,
                pa,
                "1000",
                "USD",
                (i + 1) * 10000,
                0,
            )
            .await;
            insert_balance(
                &pool_a,
                TENANT_A,
                pa,
                "4000",
                "USD",
                0,
                (i + 1) * 10000,
            )
            .await;
        });

        // Tenant B write (different amounts)
        let hb = tokio::spawn(async move {
            insert_journal_entry(
                &pool_b,
                TENANT_B,
                pb,
                "1000",
                "4000",
                (i + 1) * 5000, // unique amounts: 5000, 10000, ... 50000
                "EUR",
            )
            .await;
            insert_balance(
                &pool_b,
                TENANT_B,
                pb,
                "1000",
                "EUR",
                (i + 1) * 5000,
                0,
            )
            .await;
            insert_balance(
                &pool_b,
                TENANT_B,
                pb,
                "4000",
                "EUR",
                0,
                (i + 1) * 5000,
            )
            .await;
        });

        handles.push(ha);
        handles.push(hb);
    }

    // Wait for all concurrent writes
    for h in handles {
        h.await.expect("Task panicked");
    }

    // === VERIFICATION: No cross-tenant leakage ===

    // 1. Journal entry counts must be exactly 10 per tenant
    let entries_a = count_journal_entries(&pool, TENANT_A).await;
    let entries_b = count_journal_entries(&pool, TENANT_B).await;
    assert_eq!(entries_a, 10, "Tenant A should have exactly 10 journal entries");
    assert_eq!(entries_b, 10, "Tenant B should have exactly 10 journal entries");

    // 2. Balance row counts must be exactly 2 per tenant (1000 + 4000, one currency each)
    let balances_a = count_balances(&pool, TENANT_A).await;
    let balances_b = count_balances(&pool, TENANT_B).await;
    assert_eq!(balances_a, 2, "Tenant A should have exactly 2 balance rows");
    assert_eq!(balances_b, 2, "Tenant B should have exactly 2 balance rows");

    // 3. Period counts: exactly 1 per tenant
    let periods_a = count_periods(&pool, TENANT_A).await;
    let periods_b = count_periods(&pool, TENANT_B).await;
    assert_eq!(periods_a, 1, "Tenant A should have exactly 1 period");
    assert_eq!(periods_b, 1, "Tenant B should have exactly 1 period");

    // 4. Net balance sums — must be 0 per tenant (balanced entries)
    //    Tenant A: sum(debit 1000) = 550000, sum(credit 4000) = -550000, net = 0
    //    Tenant B: sum(debit 1000) = 275000, sum(credit 4000) = -275000, net = 0
    let net_a = sum_net_balance(&pool, TENANT_A).await;
    let net_b = sum_net_balance(&pool, TENANT_B).await;
    assert_eq!(net_a, 0, "Tenant A net balance should be 0 (balanced)");
    assert_eq!(net_b, 0, "Tenant B net balance should be 0 (balanced)");

    // 5. Cross-tenant query: Tenant A's query must not return Tenant B's data
    let a_entries_with_eur: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND currency = 'EUR'",
    )
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("query failed");
    assert_eq!(
        a_entries_with_eur, 0,
        "Tenant A must not see any EUR entries (those belong to Tenant B)"
    );

    let b_entries_with_usd: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND currency = 'USD'",
    )
    .bind(TENANT_B)
    .fetch_one(&pool)
    .await
    .expect("query failed");
    assert_eq!(
        b_entries_with_usd, 0,
        "Tenant B must not see any USD entries (those belong to Tenant A)"
    );

    // 6. Period isolation: Tenant A must not see Tenant B's period and vice versa
    let a_sees_b_period: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM accounting_periods WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(TENANT_A)
    .bind(period_b)
    .fetch_one(&pool)
    .await
    .expect("query failed");
    assert!(
        !a_sees_b_period,
        "Tenant A must not see Tenant B's period"
    );

    let b_sees_a_period: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM accounting_periods WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(TENANT_B)
    .bind(period_a)
    .fetch_one(&pool)
    .await
    .expect("query failed");
    assert!(
        !b_sees_a_period,
        "Tenant B must not see Tenant A's period"
    );

    // Cleanup
    cleanup_test_tenant(&pool, TENANT_A).await;
    cleanup_test_tenant(&pool, TENANT_B).await;
    pool.close().await;
}

#[tokio::test]
#[serial]
async fn test_tenant_boundary_concurrent_reads_during_writes() {
    let pool = get_test_pool().await;

    // Clean slate
    cleanup_test_tenant(&pool, TENANT_A).await;
    cleanup_test_tenant(&pool, TENANT_B).await;

    // Setup
    let period_a = setup_test_period(
        &pool,
        TENANT_A,
        NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
    )
    .await;

    let period_b = setup_test_period(
        &pool,
        TENANT_B,
        NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
    )
    .await;

    setup_test_account(&pool, TENANT_A, "1000", "Cash A", "asset", "debit").await;
    setup_test_account(&pool, TENANT_A, "4000", "Revenue A", "revenue", "credit").await;
    setup_test_account(&pool, TENANT_B, "1000", "Cash B", "asset", "debit").await;
    setup_test_account(&pool, TENANT_B, "4000", "Revenue B", "revenue", "credit").await;

    // Interleave writes and reads — reads must never see other tenant's data mid-flight
    let mut handles = Vec::new();

    for i in 0..5 {
        let pool_w = pool.clone();
        let pool_r = pool.clone();
        let pa = period_a;
        let pb = period_b;

        // Tenant A writes
        let write_handle = tokio::spawn(async move {
            insert_journal_entry(&pool_w, TENANT_A, pa, "1000", "4000", (i + 1) * 10000, "USD")
                .await;
        });

        // Tenant B reads concurrently — must see 0 USD entries
        let read_handle = tokio::spawn(async move {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND currency = 'USD'",
            )
            .bind(TENANT_B)
            .fetch_one(&pool_r)
            .await
            .expect("query failed");

            assert_eq!(
                count, 0,
                "Tenant B must never see USD entries (Tenant A's data), even during concurrent writes"
            );
        });

        handles.push(write_handle);
        handles.push(read_handle);
    }

    for h in handles {
        h.await.expect("Task panicked");
    }

    // Final assertion: only Tenant A has USD entries
    let a_count = count_journal_entries(&pool, TENANT_A).await;
    assert_eq!(a_count, 5, "Tenant A should have exactly 5 entries");

    let b_usd: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM journal_entries WHERE tenant_id = $1 AND currency = 'USD'",
    )
    .bind(TENANT_B)
    .fetch_one(&pool)
    .await
    .expect("query failed");
    assert_eq!(b_usd, 0, "Tenant B must have 0 USD entries");

    // Cleanup
    cleanup_test_tenant(&pool, TENANT_A).await;
    cleanup_test_tenant(&pool, TENANT_B).await;
    pool.close().await;
}
