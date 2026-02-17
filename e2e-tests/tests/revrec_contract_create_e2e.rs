//! E2E Test: Revrec Contract Creation (Phase 24a — bd-3ar)
//!
//! Verifies the create-contract command:
//! 1. Contract + obligations persist atomically
//! 2. Outbox emits revrec.contract_created atomically
//! 3. Idempotency: duplicate contract_id returns 409 CONFLICT
//! 4. Allocation sum invariant enforced (obligations must sum to total)

mod common;

use common::{generate_test_tenant, get_gl_pool};
use chrono::Utc;
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use gl_rs::repos::revrec_repo;
use gl_rs::revrec::{
    ContractCreatedPayload, PerformanceObligation, RecognitionPattern,
    EVENT_TYPE_CONTRACT_CREATED,
};

// ============================================================================
// Helpers
// ============================================================================

/// Advisory lock key for serializing revrec migration execution.
const REVREC_MIGRATION_LOCK_KEY: i64 = 7_419_283_562_i64;

/// Run revrec migrations on the GL database with advisory lock to prevent deadlocks.
async fn run_revrec_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire revrec migration advisory lock");

    let migration_sql = include_str!("../../modules/gl/db/migrations/20260217000001_create_revrec_tables.sql");
    let result = sqlx::raw_sql(migration_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release revrec migration advisory lock");

    result.expect("Failed to run revrec migration");
}

/// Build a sample contract payload with two obligations
fn sample_contract(tenant_id: &str) -> (Uuid, ContractCreatedPayload) {
    let contract_id = Uuid::new_v4();
    let obligation_1 = PerformanceObligation {
        obligation_id: Uuid::new_v4(),
        name: "SaaS License".to_string(),
        description: "12-month platform access".to_string(),
        allocated_amount_minor: 96000_00, // $96,000
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
        satisfaction_start: "2026-01-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };
    let obligation_2 = PerformanceObligation {
        obligation_id: Uuid::new_v4(),
        name: "Implementation Services".to_string(),
        description: "One-time setup and configuration".to_string(),
        allocated_amount_minor: 24000_00, // $24,000
        recognition_pattern: RecognitionPattern::PointInTime,
        satisfaction_start: "2026-01-15".to_string(),
        satisfaction_end: None,
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-acme-001".to_string(),
        contract_name: "Enterprise SaaS — Acme Corp 2026".to_string(),
        contract_start: "2026-01-01".to_string(),
        contract_end: Some("2026-12-31".to_string()),
        total_transaction_price_minor: 120000_00, // $120,000
        currency: "USD".to_string(),
        performance_obligations: vec![obligation_1, obligation_2],
        external_contract_ref: Some("CRM-ACME-2026".to_string()),
        created_at: Utc::now(),
    };

    (contract_id, payload)
}

/// Cleanup revrec test data for a tenant
async fn cleanup_revrec(pool: &PgPool, tenant_id: &str) {
    // Delete in FK order: schedule_lines → schedules → obligations → contracts
    let _ = sqlx::query(
        "DELETE FROM revrec_schedule_lines WHERE schedule_id IN (
            SELECT schedule_id FROM revrec_schedules WHERE tenant_id = $1
        )",
    )
    .bind(tenant_id)
    .execute(pool)
    .await;

    let _ = sqlx::query("DELETE FROM revrec_schedules WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;

    let _ = sqlx::query("DELETE FROM revrec_obligations WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;

    let _ = sqlx::query("DELETE FROM revrec_contracts WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Contract + obligations persist atomically with outbox event
#[tokio::test]
async fn test_revrec_contract_create_persists_atomically() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, payload) = sample_contract(&tenant_id);
    let event_id = Uuid::new_v4();

    // Act: create contract
    let result = revrec_repo::create_contract(&gl_pool, event_id, &payload).await;
    assert!(result.is_ok(), "Contract creation failed: {:?}", result.err());
    assert_eq!(result.unwrap(), contract_id);

    // Assert: contract row exists
    let contract = revrec_repo::get_contract(&gl_pool, contract_id)
        .await
        .expect("get_contract query failed")
        .expect("Contract not found");
    assert_eq!(contract.tenant_id, tenant_id);
    assert_eq!(contract.customer_id, "cust-acme-001");
    assert_eq!(contract.total_transaction_price_minor, 120000_00);
    assert_eq!(contract.currency, "USD");
    assert_eq!(contract.status, "active");
    println!("✅ Contract row persisted: {}", contract_id);

    // Assert: obligation rows exist
    let obligations = revrec_repo::get_obligations(&gl_pool, contract_id)
        .await
        .expect("get_obligations query failed");
    assert_eq!(obligations.len(), 2, "Expected 2 obligations");

    let saas = obligations.iter().find(|o| o.name == "SaaS License").unwrap();
    assert_eq!(saas.allocated_amount_minor, 96000_00);
    assert_eq!(saas.status, "unsatisfied");

    let impl_svc = obligations.iter().find(|o| o.name == "Implementation Services").unwrap();
    assert_eq!(impl_svc.allocated_amount_minor, 24000_00);
    println!("✅ {} obligation rows persisted", obligations.len());

    // Assert: allocation sum invariant holds in DB
    let db_sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(allocated_amount_minor), 0)::BIGINT FROM revrec_obligations WHERE contract_id = $1",
    )
    .bind(contract_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Sum query failed");
    assert_eq!(
        db_sum, contract.total_transaction_price_minor,
        "Allocation sum in DB must equal total_transaction_price_minor"
    );
    println!("✅ Allocation sum invariant: {} == {}", db_sum, contract.total_transaction_price_minor);

    // Assert: outbox has revrec.contract_created event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_id = $1 AND event_type = $2",
    )
    .bind(event_id)
    .bind(EVENT_TYPE_CONTRACT_CREATED)
    .fetch_one(&gl_pool)
    .await
    .expect("Outbox query failed");
    assert_eq!(
        outbox_count, 1,
        "❌ ATOMICITY VIOLATION: contract committed but no outbox event (expected 1, found {})",
        outbox_count
    );
    println!("✅ Outbox event {} emitted atomically", EVENT_TYPE_CONTRACT_CREATED);

    // Assert: outbox aggregate references the contract
    let outbox_aggregate: String = sqlx::query_scalar(
        "SELECT aggregate_id FROM events_outbox WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Outbox aggregate query failed");
    assert_eq!(outbox_aggregate, contract_id.to_string());
    println!("✅ Outbox aggregate_id matches contract_id");

    cleanup_revrec(&gl_pool, &tenant_id).await;
    println!("\n🎯 Contract creation atomicity verified");
}

/// Test 2: Idempotency — duplicate contract_id is rejected
#[tokio::test]
async fn test_revrec_contract_create_idempotent() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, payload) = sample_contract(&tenant_id);

    // First create succeeds
    let event_id_1 = Uuid::new_v4();
    let result1 = revrec_repo::create_contract(&gl_pool, event_id_1, &payload).await;
    assert!(result1.is_ok(), "First creation should succeed");
    println!("✅ First contract creation succeeded");

    // Duplicate contract_id is rejected
    let event_id_2 = Uuid::new_v4();
    let result2 = revrec_repo::create_contract(&gl_pool, event_id_2, &payload).await;
    assert!(result2.is_err(), "Duplicate contract_id must be rejected");
    match result2.unwrap_err() {
        revrec_repo::RevrecRepoError::DuplicateContract(id) => {
            assert_eq!(id, contract_id);
            println!("✅ Duplicate contract {} correctly rejected", id);
        }
        other => panic!("Expected DuplicateContract error, got: {:?}", other),
    }

    // Verify only one contract row exists
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revrec_contracts WHERE contract_id = $1",
    )
    .bind(contract_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Count query failed");
    assert_eq!(count, 1, "Must have exactly one contract row");

    // Verify only one outbox event (from first create)
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE aggregate_id = $1 AND event_type = $2",
    )
    .bind(contract_id.to_string())
    .bind(EVENT_TYPE_CONTRACT_CREATED)
    .fetch_one(&gl_pool)
    .await
    .expect("Outbox count query failed");
    assert_eq!(outbox_count, 1, "Must have exactly one outbox event");
    println!("✅ Idempotency verified: 1 contract, 1 outbox event");

    cleanup_revrec(&gl_pool, &tenant_id).await;
}

/// Test 3: Allocation sum mismatch is rejected
#[tokio::test]
async fn test_revrec_contract_allocation_mismatch_rejected() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (_, mut payload) = sample_contract(&tenant_id);

    // Tamper with allocation: reduce first obligation so sum != total
    payload.performance_obligations[0].allocated_amount_minor = 50000_00;
    // Now sum = 50,000 + 24,000 = 74,000 but total = 120,000

    let event_id = Uuid::new_v4();
    let result = revrec_repo::create_contract(&gl_pool, event_id, &payload).await;
    assert!(result.is_err(), "Mismatched allocation must be rejected");
    match result.unwrap_err() {
        revrec_repo::RevrecRepoError::AllocationMismatch { sum, expected } => {
            assert_eq!(sum, 74000_00);
            assert_eq!(expected, 120000_00);
            println!("✅ Allocation mismatch correctly rejected: sum={}, expected={}", sum, expected);
        }
        other => panic!("Expected AllocationMismatch error, got: {:?}", other),
    }

    // Verify nothing was persisted
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revrec_contracts WHERE tenant_id = $1",
    )
    .bind(&tenant_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Count query failed");
    assert_eq!(count, 0, "No contract should be persisted on validation failure");
    println!("✅ No partial data persisted on validation failure");

    cleanup_revrec(&gl_pool, &tenant_id).await;
}

/// Test 4: Contract with usage-based obligation persists correctly
#[tokio::test]
async fn test_revrec_contract_usage_based_pattern() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let contract_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id: Uuid::new_v4(),
        name: "API Usage Commitment".to_string(),
        description: "1M API calls capacity commitment".to_string(),
        allocated_amount_minor: 50000_00,
        recognition_pattern: RecognitionPattern::UsageBased {
            metric: "api_calls".to_string(),
            total_contracted_quantity: 1_000_000.0,
            unit: "calls".to_string(),
        },
        satisfaction_start: "2026-01-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.clone(),
        customer_id: "cust-api-user".to_string(),
        contract_name: "API Usage Contract".to_string(),
        contract_start: "2026-01-01".to_string(),
        contract_end: Some("2026-12-31".to_string()),
        total_transaction_price_minor: 50000_00,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation],
        external_contract_ref: None,
        created_at: Utc::now(),
    };

    let event_id = Uuid::new_v4();
    let result = revrec_repo::create_contract(&gl_pool, event_id, &payload).await;
    assert!(result.is_ok(), "Usage-based contract creation failed: {:?}", result.err());

    // Verify the recognition_pattern JSON roundtrips correctly
    let obligations = revrec_repo::get_obligations(&gl_pool, contract_id)
        .await
        .expect("get_obligations failed");
    assert_eq!(obligations.len(), 1);

    let pattern = &obligations[0].recognition_pattern;
    assert_eq!(pattern["type"], "usage_based");
    assert_eq!(pattern["metric"], "api_calls");
    assert_eq!(pattern["total_contracted_quantity"], 1_000_000.0);
    assert_eq!(pattern["unit"], "calls");
    println!("✅ Usage-based recognition pattern roundtripped correctly");

    cleanup_revrec(&gl_pool, &tenant_id).await;
}
