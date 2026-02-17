//! E2E Test: Revrec Schedule Builder (Phase 24a — bd-t1b)
//!
//! Verifies schedule generation and persistence:
//! 1. Straight-line schedule: 12-month ratable creates 12 lines summing to total
//! 2. Point-in-time schedule: single-line full recognition
//! 3. Determinism: same inputs produce identical schedules across runs
//! 4. Versioning: new schedule for same obligation gets version 2 linked to v1
//! 5. Append-only: original schedule is never modified when v2 is created
//! 6. Outbox: revrec.schedule_created emitted atomically

mod common;

use common::{generate_test_tenant, get_gl_pool};
use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use gl_rs::repos::revrec_repo;
use gl_rs::revrec::{
    ContractCreatedPayload, PerformanceObligation, RecognitionPattern,
    EVENT_TYPE_SCHEDULE_CREATED,
};
use gl_rs::revrec::schedule_builder::generate_schedule;

// ============================================================================
// Helpers
// ============================================================================

/// Advisory lock key for serializing revrec migration execution.
const REVREC_MIGRATION_LOCK_KEY: i64 = 7_419_283_562_i64;

/// Run revrec migrations on the GL database with advisory lock.
async fn run_revrec_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire revrec migration advisory lock");

    let migration_sql = include_str!("../../modules/gl/db/migrations/20260217000001_create_revrec_tables.sql");
    let result = sqlx::raw_sql(migration_sql).execute(pool).await;

    // Run versioning migration
    let versioning_sql = include_str!("../../modules/gl/db/migrations/20260217000002_add_schedule_versioning.sql");
    let result2 = sqlx::raw_sql(versioning_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release revrec migration advisory lock");

    result.expect("Failed to run revrec base migration");
    result2.expect("Failed to run revrec versioning migration");
}

/// Build a contract with a 12-month ratable obligation
fn ratable_contract(tenant_id: &str) -> (Uuid, Uuid, ContractCreatedPayload) {
    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id,
        name: "SaaS License".to_string(),
        description: "12-month platform access".to_string(),
        allocated_amount_minor: 120000_00, // $120,000
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
        satisfaction_start: "2026-01-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-acme-001".to_string(),
        contract_name: "Enterprise SaaS — Acme Corp 2026".to_string(),
        contract_start: "2026-01-01".to_string(),
        contract_end: Some("2026-12-31".to_string()),
        total_transaction_price_minor: 120000_00,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation],
        external_contract_ref: Some("CRM-ACME-2026".to_string()),
        created_at: Utc::now(),
    };

    (contract_id, obligation_id, payload)
}

/// Build a contract with a point-in-time obligation
fn point_in_time_contract(tenant_id: &str) -> (Uuid, Uuid, ContractCreatedPayload) {
    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id,
        name: "Implementation Services".to_string(),
        description: "One-time setup and configuration".to_string(),
        allocated_amount_minor: 24000_00, // $24,000
        recognition_pattern: RecognitionPattern::PointInTime,
        satisfaction_start: "2026-03-15".to_string(),
        satisfaction_end: None,
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-setup-001".to_string(),
        contract_name: "Implementation — Beta Corp".to_string(),
        contract_start: "2026-03-01".to_string(),
        contract_end: None,
        total_transaction_price_minor: 24000_00,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation],
        external_contract_ref: None,
        created_at: Utc::now(),
    };

    (contract_id, obligation_id, payload)
}

/// Cleanup revrec test data for a tenant
async fn cleanup_revrec(pool: &PgPool, tenant_id: &str) {
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

    let _ = sqlx::query("DELETE FROM events_outbox WHERE aggregate_id IN (
        SELECT schedule_id::TEXT FROM revrec_schedules WHERE tenant_id = $1
    )")
    .bind(tenant_id)
    .execute(pool)
    .await;
}

/// Create a contract and return its IDs for schedule testing
async fn setup_contract(
    pool: &PgPool,
    tenant_id: &str,
    contract_payload: &ContractCreatedPayload,
) -> Uuid {
    let event_id = Uuid::new_v4();
    revrec_repo::create_contract(pool, event_id, contract_payload)
        .await
        .expect("Contract creation failed in setup")
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Straight-line schedule (12-month ratable) generates correct lines
#[tokio::test]
async fn test_ratable_schedule_creates_12_lines_summing_to_total() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    setup_contract(&gl_pool, &tenant_id, &contract_payload).await;

    // Generate schedule
    let obligation = &contract_payload.performance_obligations[0];
    let schedule_id = Uuid::new_v4();
    let now = Utc::now();
    let schedule_payload = generate_schedule(
        schedule_id,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        now,
    )
    .expect("Schedule generation failed");

    // Persist schedule
    let event_id = Uuid::new_v4();
    let result = revrec_repo::create_schedule(&gl_pool, event_id, &schedule_payload).await;
    assert!(result.is_ok(), "Schedule persistence failed: {:?}", result.err());
    println!("✅ Schedule created: {}", schedule_id);

    // Assert: schedule row exists with correct metadata
    let schedule = revrec_repo::get_schedule(&gl_pool, schedule_id)
        .await
        .expect("get_schedule query failed")
        .expect("Schedule not found");
    assert_eq!(schedule.contract_id, contract_id);
    assert_eq!(schedule.obligation_id, obligation_id);
    assert_eq!(schedule.total_to_recognize_minor, 120000_00);
    assert_eq!(schedule.currency, "USD");
    assert_eq!(schedule.first_period, "2026-01");
    assert_eq!(schedule.last_period, "2026-12");
    assert_eq!(schedule.version, 1);
    assert!(schedule.previous_schedule_id.is_none());
    println!("✅ Schedule metadata correct (v{})", schedule.version);

    // Assert: 12 schedule lines
    let lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id)
        .await
        .expect("get_schedule_lines failed");
    assert_eq!(lines.len(), 12, "Expected 12 schedule lines");

    // Assert: all lines are $10,000.00 (even division)
    for line in &lines {
        assert_eq!(line.amount_to_recognize_minor, 10000_00);
        assert_eq!(line.deferred_revenue_account, "DEFERRED_REV");
        assert_eq!(line.recognized_revenue_account, "REV");
        assert!(!line.recognized);
    }
    println!("✅ 12 lines at $10,000 each");

    // Assert: lines sum equals total
    let db_sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_to_recognize_minor), 0)::BIGINT FROM revrec_schedule_lines WHERE schedule_id = $1",
    )
    .bind(schedule_id)
    .fetch_one(&gl_pool)
    .await
    .expect("Sum query failed");
    assert_eq!(db_sum, 120000_00, "Lines must sum to total");
    println!("✅ Lines sum: {} == {}", db_sum, 120000_00);

    // Assert: outbox has revrec.schedule_created event
    let outbox_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox WHERE event_id = $1 AND event_type = $2",
    )
    .bind(event_id)
    .bind(EVENT_TYPE_SCHEDULE_CREATED)
    .fetch_one(&gl_pool)
    .await
    .expect("Outbox query failed");
    assert_eq!(outbox_count, 1, "Expected exactly 1 outbox event");
    println!("✅ Outbox event {} emitted atomically", EVENT_TYPE_SCHEDULE_CREATED);

    // Assert: periods are in order 2026-01 through 2026-12
    let periods: Vec<&str> = lines.iter().map(|l| l.period.as_str()).collect();
    let expected_periods: Vec<String> = (1..=12).map(|m| format!("2026-{:02}", m)).collect();
    let expected_refs: Vec<&str> = expected_periods.iter().map(|s| s.as_str()).collect();
    assert_eq!(periods, expected_refs, "Periods must be 2026-01..2026-12");
    println!("✅ Periods ordered correctly: {} through {}", periods[0], periods[11]);

    cleanup_revrec(&gl_pool, &tenant_id).await;
    println!("\n🎯 Ratable schedule generation verified");
}

/// Test 2: Point-in-time schedule creates single line with full amount
#[tokio::test]
async fn test_point_in_time_schedule_single_line() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = point_in_time_contract(&tenant_id);
    setup_contract(&gl_pool, &tenant_id, &contract_payload).await;

    let obligation = &contract_payload.performance_obligations[0];
    let schedule_id = Uuid::new_v4();
    let schedule_payload = generate_schedule(
        schedule_id,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("Schedule generation failed");

    let event_id = Uuid::new_v4();
    revrec_repo::create_schedule(&gl_pool, event_id, &schedule_payload)
        .await
        .expect("Schedule persistence failed");

    // Assert: single line
    let lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id)
        .await
        .expect("get_schedule_lines failed");
    assert_eq!(lines.len(), 1, "Point-in-time should have exactly 1 line");
    assert_eq!(lines[0].amount_to_recognize_minor, 24000_00);
    assert_eq!(lines[0].period, "2026-03");
    println!("✅ Single line: {} in period {}", lines[0].amount_to_recognize_minor, lines[0].period);

    // Assert: schedule metadata
    let schedule = revrec_repo::get_schedule(&gl_pool, schedule_id)
        .await
        .expect("get_schedule failed")
        .expect("Schedule not found");
    assert_eq!(schedule.first_period, "2026-03");
    assert_eq!(schedule.last_period, "2026-03");
    assert_eq!(schedule.version, 1);
    println!("✅ Point-in-time schedule verified");

    cleanup_revrec(&gl_pool, &tenant_id).await;
}

/// Test 3: Determinism — same inputs produce identical schedule across runs
#[tokio::test]
async fn test_schedule_determinism() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    setup_contract(&gl_pool, &tenant_id, &contract_payload).await;

    let obligation = &contract_payload.performance_obligations[0];
    let now = Utc::now();

    // Generate two schedules with different IDs but same obligation inputs
    let schedule_id_1 = Uuid::new_v4();
    let payload_1 = generate_schedule(
        schedule_id_1,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        now,
    )
    .expect("First schedule generation failed");

    let schedule_id_2 = Uuid::new_v4();
    let payload_2 = generate_schedule(
        schedule_id_2,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        now,
    )
    .expect("Second schedule generation failed");

    // Assert: identical line counts
    assert_eq!(payload_1.lines.len(), payload_2.lines.len());

    // Assert: identical periods and amounts
    for (a, b) in payload_1.lines.iter().zip(payload_2.lines.iter()) {
        assert_eq!(a.period, b.period, "Periods must match");
        assert_eq!(
            a.amount_to_recognize_minor, b.amount_to_recognize_minor,
            "Amounts must match for period {}",
            a.period
        );
        assert_eq!(a.deferred_revenue_account, b.deferred_revenue_account);
        assert_eq!(a.recognized_revenue_account, b.recognized_revenue_account);
    }
    println!("✅ Determinism verified: two runs produce identical schedules");

    // Persist first schedule to verify DB determinism too
    let event_id = Uuid::new_v4();
    revrec_repo::create_schedule(&gl_pool, event_id, &payload_1)
        .await
        .expect("Schedule persistence failed");

    let db_lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id_1)
        .await
        .expect("get_schedule_lines failed");
    for (db_line, gen_line) in db_lines.iter().zip(payload_1.lines.iter()) {
        assert_eq!(db_line.period, gen_line.period);
        assert_eq!(
            db_line.amount_to_recognize_minor,
            gen_line.amount_to_recognize_minor
        );
    }
    println!("✅ DB persistence preserves deterministic output");

    cleanup_revrec(&gl_pool, &tenant_id).await;
}

/// Test 4: Version linkage — second schedule for same obligation gets v2
#[tokio::test]
async fn test_schedule_versioning_links_to_prior() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    setup_contract(&gl_pool, &tenant_id, &contract_payload).await;

    let obligation = &contract_payload.performance_obligations[0];

    // Create version 1
    let schedule_id_v1 = Uuid::new_v4();
    let payload_v1 = generate_schedule(
        schedule_id_v1,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("V1 generation failed");

    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &payload_v1)
        .await
        .expect("V1 persistence failed");

    let v1 = revrec_repo::get_schedule(&gl_pool, schedule_id_v1)
        .await
        .expect("get_schedule failed")
        .expect("V1 not found");
    assert_eq!(v1.version, 1);
    assert!(v1.previous_schedule_id.is_none());
    println!("✅ V1 created: version={}, prev=None", v1.version);

    // Create version 2 (same obligation, simulating a modification)
    let schedule_id_v2 = Uuid::new_v4();
    let payload_v2 = generate_schedule(
        schedule_id_v2,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("V2 generation failed");

    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &payload_v2)
        .await
        .expect("V2 persistence failed");

    let v2 = revrec_repo::get_schedule(&gl_pool, schedule_id_v2)
        .await
        .expect("get_schedule failed")
        .expect("V2 not found");
    assert_eq!(v2.version, 2, "Second schedule must be version 2");
    assert_eq!(
        v2.previous_schedule_id,
        Some(schedule_id_v1),
        "V2 must link to V1"
    );
    println!("✅ V2 created: version={}, prev={:?}", v2.version, v2.previous_schedule_id);

    // Assert: get_latest returns v2
    let latest = revrec_repo::get_latest_schedule_for_obligation(&gl_pool, obligation_id)
        .await
        .expect("get_latest failed")
        .expect("No latest found");
    assert_eq!(latest.schedule_id, schedule_id_v2);
    assert_eq!(latest.version, 2);
    println!("✅ get_latest_schedule_for_obligation returns v2");

    cleanup_revrec(&gl_pool, &tenant_id).await;
    println!("\n🎯 Version linkage verified");
}

/// Test 5: Append-only — original schedule lines untouched after v2 creation
#[tokio::test]
async fn test_schedule_append_only_v1_untouched() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    let (contract_id, _, contract_payload) = ratable_contract(&tenant_id);
    setup_contract(&gl_pool, &tenant_id, &contract_payload).await;

    let obligation = &contract_payload.performance_obligations[0];

    // Create V1
    let schedule_id_v1 = Uuid::new_v4();
    let payload_v1 = generate_schedule(
        schedule_id_v1,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .unwrap();
    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &payload_v1)
        .await
        .unwrap();

    // Capture V1 line amounts before V2
    let v1_lines_before = revrec_repo::get_schedule_lines(&gl_pool, schedule_id_v1)
        .await
        .unwrap();
    let v1_amounts_before: Vec<i64> = v1_lines_before
        .iter()
        .map(|l| l.amount_to_recognize_minor)
        .collect();

    // Create V2
    let schedule_id_v2 = Uuid::new_v4();
    let payload_v2 = generate_schedule(
        schedule_id_v2,
        contract_id,
        obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .unwrap();
    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &payload_v2)
        .await
        .unwrap();

    // Assert: V1 lines unchanged after V2 creation
    let v1_lines_after = revrec_repo::get_schedule_lines(&gl_pool, schedule_id_v1)
        .await
        .unwrap();
    let v1_amounts_after: Vec<i64> = v1_lines_after
        .iter()
        .map(|l| l.amount_to_recognize_minor)
        .collect();

    assert_eq!(v1_amounts_before, v1_amounts_after, "V1 lines must be unchanged");
    assert_eq!(v1_lines_after.len(), 12, "V1 must still have 12 lines");
    println!("✅ V1 lines untouched after V2 creation (append-only verified)");

    // Assert: V1 schedule metadata unchanged
    let v1_schedule = revrec_repo::get_schedule(&gl_pool, schedule_id_v1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1_schedule.version, 1);
    assert!(v1_schedule.previous_schedule_id.is_none());
    println!("✅ V1 metadata untouched");

    cleanup_revrec(&gl_pool, &tenant_id).await;
}

/// Test 6: Rounding — uneven division distributes remainder correctly
#[tokio::test]
async fn test_schedule_rounding_distributes_remainder() {
    let tenant_id = generate_test_tenant();
    let gl_pool = get_gl_pool().await;
    run_revrec_migrations(&gl_pool).await;
    cleanup_revrec(&gl_pool, &tenant_id).await;

    // $100,000.00 over 7 months: 10000000 / 7 = 1428571 remainder 3
    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id,
        name: "7-Month Service".to_string(),
        description: "Uneven division test".to_string(),
        allocated_amount_minor: 10000000, // $100,000.00
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 7 },
        satisfaction_start: "2026-01-01".to_string(),
        satisfaction_end: Some("2026-07-31".to_string()),
    };

    let contract_payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.clone(),
        customer_id: "cust-rounding".to_string(),
        contract_name: "Rounding Test Contract".to_string(),
        contract_start: "2026-01-01".to_string(),
        contract_end: Some("2026-07-31".to_string()),
        total_transaction_price_minor: 10000000,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation.clone()],
        external_contract_ref: None,
        created_at: Utc::now(),
    };
    setup_contract(&gl_pool, &tenant_id, &contract_payload).await;

    let schedule_id = Uuid::new_v4();
    let schedule_payload = generate_schedule(
        schedule_id,
        contract_id,
        &obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("Schedule generation failed");

    revrec_repo::create_schedule(&gl_pool, Uuid::new_v4(), &schedule_payload)
        .await
        .expect("Schedule persistence failed");

    let lines = revrec_repo::get_schedule_lines(&gl_pool, schedule_id)
        .await
        .expect("get_schedule_lines failed");
    assert_eq!(lines.len(), 7);

    // First 3 lines get extra penny (remainder=3)
    for i in 0..3 {
        assert_eq!(
            lines[i].amount_to_recognize_minor, 1428572,
            "Line {} should be 1428572 (base+1)",
            i
        );
    }
    for i in 3..7 {
        assert_eq!(
            lines[i].amount_to_recognize_minor, 1428571,
            "Line {} should be 1428571 (base)",
            i
        );
    }

    // Assert exact sum
    let db_sum: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount_to_recognize_minor), 0)::BIGINT FROM revrec_schedule_lines WHERE schedule_id = $1",
    )
    .bind(schedule_id)
    .fetch_one(&gl_pool)
    .await
    .unwrap();
    assert_eq!(db_sum, 10000000, "Lines must sum to exactly $100,000.00");
    println!("✅ Rounding: 3 lines at 1428572 + 4 lines at 1428571 = {}", db_sum);

    cleanup_revrec(&gl_pool, &tenant_id).await;
}
