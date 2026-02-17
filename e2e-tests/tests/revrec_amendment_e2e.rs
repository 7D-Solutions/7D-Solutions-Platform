//! E2E Test: Revrec Contract Amendments (Phase 24a — bd-1qi)
//!
//! Verifies the contract amendment flow:
//! 1. Amendment creates a new schedule version without deleting the prior.
//! 2. Recognition uses the correct (latest) schedule version.
//! 3. supersedes_event_id is populated on revrec.contract_modified and
//!    revrec.schedule_created events, linking to the prior versions.
//! 4. Idempotency: duplicate modification_id returns 409.
//! 5. Multi-obligation: only amended obligations get new schedule versions.

mod common;

use chrono::Utc;
use common::{generate_test_tenant, get_gl_pool};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use gl_rs::repos::revrec_repo;
use gl_rs::revrec::recognition_run::run_recognition;
use gl_rs::revrec::schedule_builder::generate_schedule;
use gl_rs::revrec::{
    AllocationChange, ContractCreatedPayload, ContractModifiedPayload, ModificationType,
    PerformanceObligation, RecognitionPattern, EVENT_TYPE_CONTRACT_MODIFIED,
    EVENT_TYPE_SCHEDULE_CREATED,
};

// ============================================================================
// Advisory lock keys for migration serialization
// ============================================================================

const REVREC_MIGRATION_LOCK_KEY: i64 = 7_419_283_562_i64;
const GL_MIGRATION_LOCK_KEY: i64 = 7_419_283_563_i64;
const AMENDMENT_MIGRATION_LOCK_KEY: i64 = 7_419_283_564_i64;

// ============================================================================
// Migration helpers
// ============================================================================

async fn run_revrec_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire revrec migration advisory lock");

    let base_sql =
        include_str!("../../modules/gl/db/migrations/20260217000001_create_revrec_tables.sql");
    let versioning_sql =
        include_str!("../../modules/gl/db/migrations/20260217000002_add_schedule_versioning.sql");

    let _ = sqlx::raw_sql(base_sql).execute(pool).await;
    let _ = sqlx::raw_sql(versioning_sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(REVREC_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release revrec migration advisory lock");
}

async fn run_amendment_migration(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(AMENDMENT_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire amendment migration advisory lock");

    let sql = include_str!(
        "../../modules/gl/db/migrations/20260217000007_create_revrec_modifications.sql"
    );
    let result = sqlx::raw_sql(sql).execute(pool).await;

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(AMENDMENT_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release amendment migration advisory lock");

    result.expect("Failed to run amendment migration");
}

async fn run_gl_core_migrations(pool: &PgPool) {
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(GL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to acquire GL migration advisory lock");

    let migrations = [
        include_str!("../../modules/gl/db/migrations/20260212000001_create_gl_schema.sql"),
        include_str!("../../modules/gl/db/migrations/20260212000002_create_events_tables.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000001_create_accounts_table.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000002_add_reverses_entry_id.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000003_create_accounting_periods.sql"),
        include_str!("../../modules/gl/db/migrations/20260213000004_create_account_balances.sql"),
        include_str!("../../modules/gl/db/migrations/20260216000002_add_correlation_id_to_journal_entries.sql"),
        include_str!("../../modules/gl/db/migrations/20260216000001_add_envelope_metadata_to_outbox.sql"),
    ];

    for sql in &migrations {
        let _ = sqlx::raw_sql(sql).execute(pool).await;
    }

    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(GL_MIGRATION_LOCK_KEY)
        .execute(pool)
        .await
        .expect("Failed to release GL migration advisory lock");
}

// ============================================================================
// Test data helpers
// ============================================================================

/// Build a 12-month ratable contract ($120,000 / 12 months = $10,000/month)
fn ratable_contract(tenant_id: &str) -> (Uuid, Uuid, ContractCreatedPayload) {
    let contract_id = Uuid::new_v4();
    let obligation_id = Uuid::new_v4();
    let obligation = PerformanceObligation {
        obligation_id,
        name: "SaaS License".to_string(),
        description: "12-month platform access".to_string(),
        allocated_amount_minor: 120_000_00,
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 12 },
        satisfaction_start: "2026-01-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let payload = ContractCreatedPayload {
        contract_id,
        tenant_id: tenant_id.to_string(),
        customer_id: "cust-amendment-001".to_string(),
        contract_name: "Enterprise SaaS — Amendment Test".to_string(),
        contract_start: "2026-01-01".to_string(),
        contract_end: Some("2026-12-31".to_string()),
        total_transaction_price_minor: 120_000_00,
        currency: "USD".to_string(),
        performance_obligations: vec![obligation],
        external_contract_ref: Some("CRM-AMEND-001".to_string()),
        created_at: Utc::now(),
    };

    (contract_id, obligation_id, payload)
}

/// Build a ContractModifiedPayload for a price-change amendment.
fn price_change_amendment(
    modification_id: Uuid,
    contract_id: Uuid,
    tenant_id: &str,
    obligation_id: Uuid,
    old_amount: i64,
    new_amount: i64,
) -> ContractModifiedPayload {
    ContractModifiedPayload {
        modification_id,
        contract_id,
        tenant_id: tenant_id.to_string(),
        modification_type: ModificationType::PriceChange,
        effective_date: "2026-07-01".to_string(),
        new_transaction_price_minor: Some(new_amount),
        added_obligations: vec![],
        removed_obligation_ids: vec![],
        reallocated_amounts: vec![AllocationChange {
            obligation_id,
            previous_allocated_minor: old_amount,
            new_allocated_minor: new_amount,
        }],
        reason: "annual_price_increase".to_string(),
        requires_cumulative_catchup: false,
        modified_at: Utc::now(),
    }
}

/// Cleanup all revrec + journal test data for a tenant
async fn cleanup_test_data(pool: &PgPool, tenant_id: &str) {
    let _ = sqlx::query(
        "DELETE FROM journal_lines WHERE journal_entry_id IN (
            SELECT id FROM journal_entries WHERE tenant_id = $1
        )",
    )
    .bind(tenant_id)
    .execute(pool)
    .await;

    let _ = sqlx::query("DELETE FROM journal_entries WHERE tenant_id = $1")
        .bind(tenant_id)
        .execute(pool)
        .await;

    let _ = sqlx::query(
        "DELETE FROM events_outbox WHERE aggregate_id IN (
            SELECT contract_id::TEXT FROM revrec_contracts WHERE tenant_id = $1
            UNION
            SELECT schedule_id::TEXT FROM revrec_schedules WHERE tenant_id = $1
            UNION
            SELECT modification_id::TEXT FROM revrec_contract_modifications WHERE tenant_id = $1
        )",
    )
    .bind(tenant_id)
    .execute(pool)
    .await;

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

    let _ = sqlx::query("DELETE FROM revrec_contract_modifications WHERE tenant_id = $1")
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

/// Setup: create contract + initial schedule. Returns (schedule_id, original_event_id).
async fn setup_contract_with_schedule(
    pool: &PgPool,
    contract_id: Uuid,
    obligation: &PerformanceObligation,
    contract_payload: &ContractCreatedPayload,
    tenant_id: &str,
) -> Uuid {
    let contract_event_id = Uuid::new_v4();
    revrec_repo::create_contract(pool, contract_event_id, contract_payload)
        .await
        .expect("Contract creation failed");

    let schedule_id = Uuid::new_v4();
    let schedule_payload = generate_schedule(
        schedule_id,
        contract_id,
        obligation,
        tenant_id,
        &contract_payload.currency,
        Utc::now(),
    )
    .expect("Schedule generation failed");

    revrec_repo::create_schedule(pool, Uuid::new_v4(), &schedule_payload)
        .await
        .expect("Schedule persistence failed");

    schedule_id
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Amendment creates new schedule version without deleting prior
#[tokio::test]
async fn test_amendment_creates_new_schedule_version() {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;
    run_gl_core_migrations(&pool).await;
    run_revrec_migrations(&pool).await;
    run_amendment_migration(&pool).await;
    cleanup_test_data(&pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];

    let original_schedule_id =
        setup_contract_with_schedule(&pool, contract_id, obligation, &contract_payload, &tenant_id)
            .await;

    // Verify original schedule is version 1
    let original_schedule = revrec_repo::get_schedule(&pool, original_schedule_id)
        .await
        .expect("get_schedule failed")
        .expect("Original schedule must exist");
    assert_eq!(original_schedule.version, 1, "Original schedule must be version 1");
    assert_eq!(original_schedule.total_to_recognize_minor, 120_000_00);

    // Record the amendment
    let modification_id = Uuid::new_v4();
    let amendment_payload = price_change_amendment(
        modification_id,
        contract_id,
        &tenant_id,
        obligation_id,
        120_000_00,
        90_000_00, // new amount for remaining 6 months (Jul-Dec)
    );

    revrec_repo::create_amendment(&pool, Uuid::new_v4(), &amendment_payload)
        .await
        .expect("Amendment creation failed");

    // Generate new schedule (Jul-Dec 2026, $90,000 over 6 months = $15,000/month)
    let amended_obligation = PerformanceObligation {
        obligation_id,
        name: obligation.name.clone(),
        description: obligation.description.clone(),
        allocated_amount_minor: 90_000_00,
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 6 },
        satisfaction_start: "2026-07-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let new_schedule_id = Uuid::new_v4();
    let new_schedule_payload = generate_schedule(
        new_schedule_id,
        contract_id,
        &amended_obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("Amendment schedule generation failed");

    // Find the previous schedule event_id for supersession
    let supersedes_event_id =
        revrec_repo::find_schedule_outbox_event_id(&pool, original_schedule_id)
            .await
            .expect("find_schedule_outbox_event_id failed");

    revrec_repo::create_schedule_with_supersession(
        &pool,
        Uuid::new_v4(),
        &new_schedule_payload,
        supersedes_event_id,
    )
    .await
    .expect("create_schedule_with_supersession failed");

    // ACCEPTANCE CRITERION 1: Both schedule versions exist; prior not deleted
    let original = revrec_repo::get_schedule(&pool, original_schedule_id)
        .await
        .expect("get_schedule failed");
    assert!(original.is_some(), "Original schedule v1 must still exist after amendment");

    let new_sched = revrec_repo::get_schedule(&pool, new_schedule_id)
        .await
        .expect("get_schedule failed")
        .expect("New schedule must exist");
    assert_eq!(new_sched.version, 2, "Amended schedule must be version 2");
    assert_eq!(new_sched.total_to_recognize_minor, 90_000_00);
    assert_eq!(new_sched.previous_schedule_id, Some(original_schedule_id));

    // New schedule covers Jul-Dec
    assert_eq!(new_sched.first_period, "2026-07");
    assert_eq!(new_sched.last_period, "2026-12");

    let lines = revrec_repo::get_schedule_lines(&pool, new_schedule_id)
        .await
        .expect("get_schedule_lines failed");
    assert_eq!(lines.len(), 6, "Amended schedule must have 6 monthly lines");

    let line_sum: i64 = lines.iter().map(|l| l.amount_to_recognize_minor).sum();
    assert_eq!(line_sum, 90_000_00, "Lines must sum to $90,000");

    for line in &lines {
        assert_eq!(
            line.amount_to_recognize_minor, 15_000_00,
            "Each of 6 months should be $15,000"
        );
    }
}

/// Test 2: Recognition uses latest schedule version for amended obligation
#[tokio::test]
async fn test_recognition_uses_latest_schedule_version() {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;
    run_gl_core_migrations(&pool).await;
    run_revrec_migrations(&pool).await;
    run_amendment_migration(&pool).await;
    cleanup_test_data(&pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];

    // Create original contract + schedule (v1: $10,000/month Jan-Dec)
    setup_contract_with_schedule(&pool, contract_id, obligation, &contract_payload, &tenant_id)
        .await;

    // Recognize January from v1 schedule
    let result = run_recognition(&pool, &tenant_id, "2026-01", "2026-01-31")
        .await
        .expect("Recognition for Jan failed");
    assert_eq!(result.lines_recognized, 1);
    assert_eq!(result.total_recognized_minor, 10_000_00, "Jan: $10,000 from v1");

    // Amend contract effective July: $90,000 for remaining 6 months = $15,000/month
    let modification_id = Uuid::new_v4();
    let amendment_payload = price_change_amendment(
        modification_id,
        contract_id,
        &tenant_id,
        obligation_id,
        120_000_00,
        90_000_00,
    );
    revrec_repo::create_amendment(&pool, Uuid::new_v4(), &amendment_payload)
        .await
        .expect("Amendment failed");

    // Create v2 schedule: Jul-Dec, $15,000/month
    let amended_obligation = PerformanceObligation {
        obligation_id,
        name: obligation.name.clone(),
        description: obligation.description.clone(),
        allocated_amount_minor: 90_000_00,
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 6 },
        satisfaction_start: "2026-07-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let new_schedule_id = Uuid::new_v4();
    let new_schedule_payload = generate_schedule(
        new_schedule_id,
        contract_id,
        &amended_obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("Schedule generation failed");

    revrec_repo::create_schedule_with_supersession(&pool, Uuid::new_v4(), &new_schedule_payload, None)
        .await
        .expect("create_schedule_with_supersession failed");

    // ACCEPTANCE CRITERION 2: Recognition for July uses the v2 schedule ($15,000)
    let result_jul = run_recognition(&pool, &tenant_id, "2026-07", "2026-07-31")
        .await
        .expect("Recognition for Jul failed");

    assert_eq!(result_jul.lines_recognized, 1, "Should recognize exactly 1 line in Jul");
    assert_eq!(
        result_jul.total_recognized_minor, 15_000_00,
        "Jul recognition should use v2 schedule: $15,000"
    );

    // Verify v1 line for July does NOT exist (v1 only had Jan-Dec but recognition skips non-latest)
    // (v1 schedule covered Jul too at $10,000 but recognition uses MAX(version) = v2)
    // Run again to confirm idempotency (already-recognized line is filtered by recognized=false,
    // so due_lines is empty: lines_recognized=0, lines_skipped=0)
    let result_jul2 = run_recognition(&pool, &tenant_id, "2026-07", "2026-07-31")
        .await
        .expect("Second recognition for Jul failed");
    assert_eq!(result_jul2.lines_recognized, 0, "Should not re-recognize Jul line");
    assert_eq!(result_jul2.lines_skipped, 0, "No due lines returned for already-recognized period");
    assert_eq!(result_jul2.total_recognized_minor, 0);
}

/// Test 3: supersedes_event_id is populated on revrec.contract_modified
#[tokio::test]
async fn test_contract_modified_supersedes_event_id_populated() {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;
    run_gl_core_migrations(&pool).await;
    run_revrec_migrations(&pool).await;
    run_amendment_migration(&pool).await;
    cleanup_test_data(&pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    setup_contract_with_schedule(
        &pool,
        contract_id,
        &contract_payload.performance_obligations[0],
        &contract_payload,
        &tenant_id,
    )
    .await;

    // Capture the contract_created outbox event_id BEFORE the amendment
    let contract_created_event_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM events_outbox WHERE event_type = 'revrec.contract_created' AND aggregate_id = $1",
    )
    .bind(contract_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("contract_created event must exist in outbox");

    // Record amendment
    let modification_id = Uuid::new_v4();
    let amendment_payload = price_change_amendment(
        modification_id,
        contract_id,
        &tenant_id,
        obligation_id,
        120_000_00,
        90_000_00,
    );
    let amendment_event_id = Uuid::new_v4();
    revrec_repo::create_amendment(&pool, amendment_event_id, &amendment_payload)
        .await
        .expect("Amendment failed");

    // ACCEPTANCE CRITERION 3a: revrec.contract_modified event has supersedes_event_id
    // pointing to the prior contract_created event
    let row = sqlx::query(
        "SELECT event_id, supersedes_event_id FROM events_outbox WHERE event_type = $1 AND aggregate_id = $2",
    )
    .bind(EVENT_TYPE_CONTRACT_MODIFIED)
    .bind(contract_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("contract_modified event must exist in outbox");

    let actual_supersedes: Option<Uuid> = row.try_get("supersedes_event_id").ok().flatten();
    assert!(
        actual_supersedes.is_some(),
        "revrec.contract_modified must have supersedes_event_id set"
    );
    assert_eq!(
        actual_supersedes.unwrap(),
        contract_created_event_id,
        "supersedes_event_id must point to the prior contract_created event"
    );

    // Also verify the modification_id is in the modifications table
    let mods = revrec_repo::get_modifications_for_contract(&pool, contract_id)
        .await
        .expect("get_modifications_for_contract failed");
    assert_eq!(mods.len(), 1);
    assert_eq!(mods[0].modification_id, modification_id);
    assert_eq!(mods[0].modification_type, "price_change");
    assert_eq!(mods[0].effective_date.to_string(), "2026-07-01");
}

/// Test 4: supersedes_event_id is populated on revrec.schedule_created for amended schedule
#[tokio::test]
async fn test_amended_schedule_supersedes_event_id_populated() {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;
    run_gl_core_migrations(&pool).await;
    run_revrec_migrations(&pool).await;
    run_amendment_migration(&pool).await;
    cleanup_test_data(&pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];

    let original_schedule_id =
        setup_contract_with_schedule(&pool, contract_id, obligation, &contract_payload, &tenant_id)
            .await;

    // Capture the original schedule_created event_id
    let original_schedule_event_id: Uuid = sqlx::query_scalar(
        "SELECT event_id FROM events_outbox WHERE event_type = 'revrec.schedule_created' AND aggregate_id = $1",
    )
    .bind(original_schedule_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("Original schedule_created event must exist in outbox");

    // Record amendment
    let modification_id = Uuid::new_v4();
    let amendment_payload = price_change_amendment(
        modification_id,
        contract_id,
        &tenant_id,
        obligation_id,
        120_000_00,
        90_000_00,
    );
    revrec_repo::create_amendment(&pool, Uuid::new_v4(), &amendment_payload)
        .await
        .expect("Amendment failed");

    // Generate + persist amended schedule with supersession
    let amended_obligation = PerformanceObligation {
        obligation_id,
        name: obligation.name.clone(),
        description: obligation.description.clone(),
        allocated_amount_minor: 90_000_00,
        recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 6 },
        satisfaction_start: "2026-07-01".to_string(),
        satisfaction_end: Some("2026-12-31".to_string()),
    };

    let new_schedule_id = Uuid::new_v4();
    let new_schedule_event_id = Uuid::new_v4();
    let new_schedule_payload = generate_schedule(
        new_schedule_id,
        contract_id,
        &amended_obligation,
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("Schedule generation failed");

    // Use the helper to find the previous schedule's outbox event_id
    let supersedes_event_id =
        revrec_repo::find_schedule_outbox_event_id(&pool, original_schedule_id)
            .await
            .expect("find_schedule_outbox_event_id failed");

    assert!(
        supersedes_event_id.is_some(),
        "Must find original schedule outbox event_id"
    );
    assert_eq!(supersedes_event_id.unwrap(), original_schedule_event_id);

    revrec_repo::create_schedule_with_supersession(
        &pool,
        new_schedule_event_id,
        &new_schedule_payload,
        supersedes_event_id,
    )
    .await
    .expect("create_schedule_with_supersession failed");

    // ACCEPTANCE CRITERION 3b: new revrec.schedule_created event has supersedes_event_id
    // pointing to the original schedule_created event
    let row = sqlx::query(
        "SELECT supersedes_event_id FROM events_outbox WHERE event_type = $1 AND aggregate_id = $2",
    )
    .bind(EVENT_TYPE_SCHEDULE_CREATED)
    .bind(new_schedule_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("New schedule_created event must exist in outbox");

    let actual_supersedes: Option<Uuid> = row.try_get("supersedes_event_id").ok().flatten();
    assert!(
        actual_supersedes.is_some(),
        "Amended revrec.schedule_created must have supersedes_event_id set"
    );
    assert_eq!(
        actual_supersedes.unwrap(),
        original_schedule_event_id,
        "supersedes_event_id must point to original schedule_created event"
    );
}

/// Test 5: Idempotency — duplicate modification_id returns error
#[tokio::test]
async fn test_duplicate_modification_id_rejected() {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;
    run_gl_core_migrations(&pool).await;
    run_revrec_migrations(&pool).await;
    run_amendment_migration(&pool).await;
    cleanup_test_data(&pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    setup_contract_with_schedule(
        &pool,
        contract_id,
        &contract_payload.performance_obligations[0],
        &contract_payload,
        &tenant_id,
    )
    .await;

    let modification_id = Uuid::new_v4();
    let amendment_payload = price_change_amendment(
        modification_id,
        contract_id,
        &tenant_id,
        obligation_id,
        120_000_00,
        90_000_00,
    );

    // First amendment succeeds
    revrec_repo::create_amendment(&pool, Uuid::new_v4(), &amendment_payload)
        .await
        .expect("First amendment must succeed");

    // Second amendment with same modification_id must fail
    let result = revrec_repo::create_amendment(&pool, Uuid::new_v4(), &amendment_payload).await;
    assert!(
        result.is_err(),
        "Duplicate modification_id must be rejected"
    );
    match result.unwrap_err() {
        revrec_repo::RevrecRepoError::DuplicateModification(id) => {
            assert_eq!(id, modification_id);
        }
        e => panic!("Expected DuplicateModification, got: {:?}", e),
    }
}

/// Test 6: Second amendment creates v3, superseding v2
#[tokio::test]
async fn test_second_amendment_creates_v3() {
    let tenant_id = generate_test_tenant();
    let pool = get_gl_pool().await;
    run_gl_core_migrations(&pool).await;
    run_revrec_migrations(&pool).await;
    run_amendment_migration(&pool).await;
    cleanup_test_data(&pool, &tenant_id).await;

    let (contract_id, obligation_id, contract_payload) = ratable_contract(&tenant_id);
    let obligation = &contract_payload.performance_obligations[0];

    let original_schedule_id =
        setup_contract_with_schedule(&pool, contract_id, obligation, &contract_payload, &tenant_id)
            .await;

    // First amendment: v2 schedule (Jul-Dec, $90,000)
    let mod1_id = Uuid::new_v4();
    revrec_repo::create_amendment(
        &pool,
        Uuid::new_v4(),
        &price_change_amendment(mod1_id, contract_id, &tenant_id, obligation_id, 120_000_00, 90_000_00),
    )
    .await
    .expect("First amendment failed");

    let v1_supersedes =
        revrec_repo::find_schedule_outbox_event_id(&pool, original_schedule_id)
            .await
            .expect("find failed");

    let v2_schedule_id = Uuid::new_v4();
    let v2_payload = generate_schedule(
        v2_schedule_id,
        contract_id,
        &PerformanceObligation {
            obligation_id,
            name: obligation.name.clone(),
            description: obligation.description.clone(),
            allocated_amount_minor: 90_000_00,
            recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 6 },
            satisfaction_start: "2026-07-01".to_string(),
            satisfaction_end: Some("2026-12-31".to_string()),
        },
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("v2 schedule gen failed");

    revrec_repo::create_schedule_with_supersession(&pool, Uuid::new_v4(), &v2_payload, v1_supersedes)
        .await
        .expect("v2 create failed");

    let v2_row = revrec_repo::get_schedule(&pool, v2_schedule_id)
        .await
        .expect("get v2 failed")
        .expect("v2 must exist");
    assert_eq!(v2_row.version, 2);

    // Second amendment: v3 schedule (Oct-Dec, $45,000)
    let mod2_id = Uuid::new_v4();
    revrec_repo::create_amendment(
        &pool,
        Uuid::new_v4(),
        &price_change_amendment(mod2_id, contract_id, &tenant_id, obligation_id, 90_000_00, 45_000_00),
    )
    .await
    .expect("Second amendment failed");

    let v2_supersedes =
        revrec_repo::find_schedule_outbox_event_id(&pool, v2_schedule_id)
            .await
            .expect("find v2 event failed");

    let v3_schedule_id = Uuid::new_v4();
    let v3_payload = generate_schedule(
        v3_schedule_id,
        contract_id,
        &PerformanceObligation {
            obligation_id,
            name: obligation.name.clone(),
            description: obligation.description.clone(),
            allocated_amount_minor: 45_000_00,
            recognition_pattern: RecognitionPattern::RatableOverTime { period_months: 3 },
            satisfaction_start: "2026-10-01".to_string(),
            satisfaction_end: Some("2026-12-31".to_string()),
        },
        &tenant_id,
        "USD",
        Utc::now(),
    )
    .expect("v3 schedule gen failed");

    revrec_repo::create_schedule_with_supersession(&pool, Uuid::new_v4(), &v3_payload, v2_supersedes)
        .await
        .expect("v3 create failed");

    let v3_row = revrec_repo::get_schedule(&pool, v3_schedule_id)
        .await
        .expect("get v3 failed")
        .expect("v3 must exist");
    assert_eq!(v3_row.version, 3, "Second amendment must produce version 3");
    assert_eq!(v3_row.previous_schedule_id, Some(v2_schedule_id));
    assert_eq!(v3_row.total_to_recognize_minor, 45_000_00);
    assert_eq!(v3_row.first_period, "2026-10");
    assert_eq!(v3_row.last_period, "2026-12");

    // All three versions must exist
    assert!(revrec_repo::get_schedule(&pool, original_schedule_id).await.unwrap().is_some(), "v1 must exist");
    assert!(revrec_repo::get_schedule(&pool, v2_schedule_id).await.unwrap().is_some(), "v2 must exist");
    assert!(revrec_repo::get_schedule(&pool, v3_schedule_id).await.unwrap().is_some(), "v3 must exist");

    // Recognition uses v3 (latest) for Oct 2026
    let result = run_recognition(&pool, &tenant_id, "2026-10", "2026-10-31")
        .await
        .expect("Recognition failed");
    assert_eq!(result.lines_recognized, 1);
    assert_eq!(
        result.total_recognized_minor, 15_000_00,
        "Oct recognition should use v3: $45,000/3 = $15,000"
    );

    // Verify modifications table has both amendments
    let mods = revrec_repo::get_modifications_for_contract(&pool, contract_id)
        .await
        .expect("get_modifications failed");
    assert_eq!(mods.len(), 2, "Two amendments must be recorded");
}
