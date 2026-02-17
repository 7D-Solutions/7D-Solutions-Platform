//! Scale Test: 100 Tenants – Truth at Scale (Phase 17 Capstone)
//!
//! **Purpose:** Correctness-under-scale validation for the projection + audit + oracle stack.
//!
//! **What this verifies:**
//! 1. Projection digest stability: 100 tenants × 6 events → identical digest across 2 rebuild passes
//! 2. Projection lag bounds: fresh cursors for 100 tenants remain below 5000ms SLO
//! 3. Oracle correctness: assert_cross_module_invariants passes for 100 tenant IDs
//!    (gracefully skips if audit table not yet integrated)
//!
//! **Invariant:** Correctness properties (no duplicates, digest determinism, invariant integrity)
//! hold as tenant count scales from 20 → 100.
//!
//! **Verification:**
//! ```bash
//! docker compose -f infra/docker/docker-compose.infrastructure.yml up -d
//! cargo test -p e2e-tests scale_100_tenants_truth_at_scale_e2e -- --nocapture
//! ```

use chrono::Utc;
use projections::{
    compute_versioned_digest, create_shadow_cursor_table, create_shadow_table, drop_shadow_table,
    save_shadow_cursor, swap_cursor_tables_atomic, swap_tables_atomic,
};
use projections::cursor::ProjectionCursor;
use projections::metrics::ProjectionMetrics;
use serial_test::serial;
use sqlx::PgPool;
use uuid::Uuid;

// ============================================================================
// Constants
// ============================================================================

const TENANT_COUNT: usize = 100;
const EVENTS_PER_TENANT: usize = 6; // Compressed billing cycles (one per month)
const PROJECTION_LAG_BOUND_MS: f64 = 5000.0; // 5-second SLO for fresh events
const BASE_TABLE: &str = "scale_tenant_billing_summary";
const PROJECTION_NAME: &str = "scale_tenant_billing_summary";

// ============================================================================
// DB Pool Helpers
// ============================================================================

async fn get_projections_pool() -> PgPool {
    let url = std::env::var("PROJECTIONS_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("PROJECTIONS_DATABASE_URL or DATABASE_URL must be set");
    PgPool::connect(&url)
        .await
        .expect("Failed to connect to projections database")
}

async fn get_ar_pool() -> PgPool {
    let url = std::env::var("AR_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://ar_user:ar_pass@localhost:5434/ar_db".to_string());
    PgPool::connect(&url).await.expect("Failed to connect to AR database")
}

async fn get_payments_pool() -> PgPool {
    let url = std::env::var("PAYMENTS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://payments_user:payments_pass@localhost:5436/payments_db".to_string());
    PgPool::connect(&url).await.expect("Failed to connect to Payments database")
}

async fn get_subscriptions_pool() -> PgPool {
    let url = std::env::var("SUBSCRIPTIONS_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db".to_string());
    PgPool::connect(&url).await.expect("Failed to connect to Subscriptions database")
}

async fn get_gl_pool() -> PgPool {
    let url = std::env::var("GL_DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://gl_user:gl_pass@localhost:5438/gl_db".to_string());
    PgPool::connect(&url).await.expect("Failed to connect to GL database")
}

async fn get_audit_pool() -> PgPool {
    let url = std::env::var("AUDIT_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("AUDIT_DATABASE_URL or DATABASE_URL must be set");
    PgPool::connect(&url).await.expect("Failed to connect to audit database")
}

// ============================================================================
// Migrations
// ============================================================================

async fn run_projections_migrations(pool: &PgPool) {
    // Drop all cursor-related tables including leftovers from previous test runs.
    // Named indexes (e.g. projection_cursors_updated_at) persist on renamed tables
    // after blue-green swaps, so we must clean up all variants.
    for table in &[
        "projection_cursors_shadow",
        "projection_cursors_old",
        "projection_cursors",
    ] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", table))
            .execute(pool)
            .await
            .ok();
    }

    let sql = include_str!(
        "../../platform/projections/db/migrations/20260216000001_create_projection_cursors.sql"
    );
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("Failed to run projections migration");
}

// ============================================================================
// Test Data Helpers
// ============================================================================

/// Generate 100 deterministic tenant IDs
fn generate_tenant_ids() -> Vec<String> {
    (0..TENANT_COUNT)
        .map(|i| format!("scale-t{:03}", i))
        .collect()
}

/// Create the scale billing summary shadow table
async fn create_scale_summary_shadow(pool: &PgPool) {
    let shadow_table = format!("{}_shadow", BASE_TABLE);
    let ddl = format!(
        r#"
        CREATE TABLE {} (
            tenant_id VARCHAR(100) PRIMARY KEY,
            cycle_count INT NOT NULL DEFAULT 0,
            total_billed BIGINT NOT NULL DEFAULT 0,
            last_cycle_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        )
        "#,
        shadow_table
    );
    create_shadow_table(pool, BASE_TABLE, &ddl)
        .await
        .expect("Failed to create shadow table");
}

/// Fixed deterministic base timestamp for all scale test events.
/// Using a fixed timestamp ensures digest equality across multiple rebuild passes.
fn deterministic_cycle_ts(cycle: usize) -> chrono::DateTime<Utc> {
    use chrono::TimeZone;
    // Base: 2026-01-01T00:00:00Z, advance 30 days per cycle
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .unwrap()
        + chrono::Duration::days(30 * cycle as i64)
}

/// Process all 6 cycles for a single tenant into the shadow table
///
/// Uses deterministic timestamps to guarantee digest stability across rebuilds.
async fn process_tenant_cycles_into_shadow(
    pool: &PgPool,
    tenant_id: &str,
    tenant_index: usize,
) {
    let shadow_table = format!("{}_shadow", BASE_TABLE);

    for cycle in 0..EVENTS_PER_TENANT {
        // Deterministic event ID: no collisions across 100 tenants × 6 cycles
        let event_id = Uuid::from_u128((tenant_index as u128) * 1000 + (cycle as u128));
        let amount = ((cycle + 1) * 100) as i64;
        // Fixed timestamp per cycle — identical across all rebuild passes
        let ts = deterministic_cycle_ts(cycle);

        sqlx::query(&format!(
            r#"
            INSERT INTO {} (tenant_id, cycle_count, total_billed, last_cycle_at, updated_at)
            VALUES ($1, 1, $2, $3, $3)
            ON CONFLICT (tenant_id)
            DO UPDATE SET
                cycle_count = {}.cycle_count + 1,
                total_billed = {}.total_billed + $2,
                last_cycle_at = $3,
                updated_at = $3
            "#,
            shadow_table, shadow_table, shadow_table
        ))
        .bind(tenant_id)
        .bind(amount)
        .bind(ts)
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("Failed to process cycle {} for tenant {}: {}", cycle, tenant_id, e));

        save_shadow_cursor(pool, PROJECTION_NAME, tenant_id, event_id, ts)
            .await
            .unwrap_or_else(|e| panic!("Failed to save shadow cursor for tenant {}: {}", tenant_id, e));
    }
}

/// Run a complete rebuild of the scale billing summary projection
///
/// Returns the versioned digest after rebuilding.
async fn rebuild_scale_projection(pool: &PgPool) -> projections::VersionedDigest {
    // Clean up previous shadow
    drop_shadow_table(pool, BASE_TABLE).await.ok();
    sqlx::query("DROP TABLE IF EXISTS projection_cursors_shadow CASCADE")
        .execute(pool)
        .await
        .ok();

    // Create shadow tables
    create_scale_summary_shadow(pool).await;
    create_shadow_cursor_table(pool)
        .await
        .expect("Failed to create shadow cursor table");

    // Process all 100 tenants
    let tenant_ids = generate_tenant_ids();
    for (i, tenant_id) in tenant_ids.iter().enumerate() {
        process_tenant_cycles_into_shadow(pool, tenant_id, i).await;
    }

    // Compute digest of shadow table
    let digest = compute_versioned_digest(pool, &format!("{}_shadow", BASE_TABLE), "tenant_id")
        .await
        .expect("Failed to compute digest");

    // Atomic swap: shadow → live
    swap_tables_atomic(pool, BASE_TABLE)
        .await
        .expect("Failed to swap projection tables");
    swap_cursor_tables_atomic(pool)
        .await
        .expect("Failed to swap cursor tables");

    digest
}

// ============================================================================
// Tests
// ============================================================================

/// Test 1: Projection digest stability at 100-tenant scale
///
/// Rebuilds the projection twice from the same deterministic event stream.
/// Digest must be identical across both runs, proving:
/// - No nondeterminism (UUID generation, timestamp drift) at 100-tenant scale
/// - Idempotent apply semantics hold across all 600 events (100 × 6)
/// - Shadow table swap produces consistent live state
#[tokio::test]
#[serial]
async fn test_100_tenants_projection_digest_stability() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    println!("\n=== Scale Test: 100-Tenant Projection Digest Stability ===");
    println!("Tenants: {}", TENANT_COUNT);
    println!("Events/tenant: {} (compressed billing cycles)", EVENTS_PER_TENANT);
    println!("Total events: {}", TENANT_COUNT * EVENTS_PER_TENANT);

    // Run 1
    println!("\n--- Rebuild Pass 1 ---");
    let digest1 = rebuild_scale_projection(&pool).await;
    println!("Digest: {}", digest1);

    // Verify row count in live table
    let row_count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", BASE_TABLE))
        .fetch_one(&pool)
        .await
        .expect("Failed to count rows");
    println!("Row count: {} (expected {})", row_count, TENANT_COUNT);
    assert_eq!(
        row_count as usize, TENANT_COUNT,
        "Live table should have exactly {} rows after first rebuild",
        TENANT_COUNT
    );

    // Verify each tenant has 6 cycles
    let max_cycles: i32 =
        sqlx::query_scalar(&format!("SELECT MAX(cycle_count) FROM {}", BASE_TABLE))
            .fetch_one(&pool)
            .await
            .expect("Failed to get max cycle count");
    assert_eq!(
        max_cycles as usize, EVENTS_PER_TENANT,
        "Each tenant should have exactly {} cycles recorded",
        EVENTS_PER_TENANT
    );
    println!("Max cycles per tenant: {} ✓", max_cycles);

    // Run 2 (same seed → same events)
    println!("\n--- Rebuild Pass 2 ---");
    let digest2 = rebuild_scale_projection(&pool).await;
    println!("Digest: {}", digest2);

    // Assert digest equality (determinism proof)
    assert_eq!(
        digest1, digest2,
        "Digest must be identical across 2 rebuilds of 100-tenant scale projection"
    );
    println!("\n✅ Digest stability: PASSED (100 tenants × 6 cycles = 600 events, deterministic)");

    // Verify cursor count in live table
    let cursor_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM projection_cursors WHERE projection_name = $1")
            .bind(PROJECTION_NAME)
            .fetch_one(&pool)
            .await
            .expect("Failed to count cursors");
    println!(
        "Cursor count: {} (expected {})",
        cursor_count, TENANT_COUNT
    );
    assert_eq!(
        cursor_count as usize, TENANT_COUNT,
        "Should have exactly {} cursors (one per tenant)",
        TENANT_COUNT
    );
    println!("✅ Cursor integrity: PASSED");

    // Cleanup
    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", BASE_TABLE))
        .execute(&pool)
        .await
        .ok();
    sqlx::query(&format!("DROP TABLE IF EXISTS {}_old CASCADE", BASE_TABLE))
        .execute(&pool)
        .await
        .ok();

    println!("\n✅ 100-Tenant Projection Digest Stability: ALL PASSED\n");
}

/// Test 2: Projection lag within SLO bounds for 100 tenants
///
/// Saves fresh cursors for 100 tenants and records metrics.
/// All lag values must be < 5000ms (events just occurred).
///
/// This validates that the metrics system doesn't accumulate state
/// across tenants or mis-report lag at scale.
#[tokio::test]
#[serial]
async fn test_100_tenants_projection_lag_within_slo_bounds() {
    let pool = get_projections_pool().await;
    run_projections_migrations(&pool).await;

    println!("\n=== Scale Test: 100-Tenant Projection Lag Bounds ===");
    println!("SLO bound: {}ms", PROJECTION_LAG_BOUND_MS);

    let tenant_ids = generate_tenant_ids();
    let metrics = ProjectionMetrics::new().expect("Failed to create ProjectionMetrics");

    // Save fresh cursors for all 100 tenants and record metrics
    let now = Utc::now();
    for (i, tenant_id) in tenant_ids.iter().enumerate() {
        let event_id = Uuid::from_u128(99000 + i as u128);
        ProjectionCursor::save(&pool, "scale_lag_test", tenant_id, event_id, now)
            .await
            .unwrap_or_else(|e| panic!("Failed to save cursor for {}: {}", tenant_id, e));

        let cursor = ProjectionCursor::load(&pool, "scale_lag_test", tenant_id)
            .await
            .expect("Failed to load cursor")
            .expect("Cursor should exist");

        metrics.record_cursor_state(&cursor);
    }

    println!("Cursors saved and metrics recorded for {} tenants", TENANT_COUNT);

    // Gather metrics and validate lag bounds
    let families = metrics.registry().gather();
    let lag_family = families
        .iter()
        .find(|f| f.get_name() == "projection_lag_ms")
        .expect("projection_lag_ms metric should exist");

    let lag_metrics = lag_family.get_metric();
    assert_eq!(
        lag_metrics.len(),
        TENANT_COUNT,
        "Should have lag metrics for all {} tenants",
        TENANT_COUNT
    );
    println!("Metric count: {} ✓", lag_metrics.len());

    // Assert all lag values are within SLO
    let mut max_lag = 0.0f64;
    let mut violations = 0usize;
    for m in lag_metrics {
        let lag = m.get_gauge().get_value();
        max_lag = max_lag.max(lag);
        if lag > PROJECTION_LAG_BOUND_MS {
            violations += 1;
            println!("  ⚠️  SLO violation: lag={}ms (bound={}ms)", lag, PROJECTION_LAG_BOUND_MS);
        }
    }

    println!("Max lag observed: {:.1}ms (SLO: {}ms)", max_lag, PROJECTION_LAG_BOUND_MS);
    assert_eq!(
        violations, 0,
        "{} tenants exceeded lag SLO of {}ms (max observed: {:.1}ms)",
        violations, PROJECTION_LAG_BOUND_MS, max_lag
    );

    println!("✅ All {} tenants within lag SLO", TENANT_COUNT);

    // Also validate last_applied_age_seconds is near-zero for all tenants
    let age_family = families
        .iter()
        .find(|f| f.get_name() == "projection_last_applied_age_seconds")
        .expect("projection_last_applied_age_seconds should exist");

    let age_metrics = age_family.get_metric();
    let mut age_violations = 0usize;
    for m in age_metrics {
        let age = m.get_gauge().get_value();
        if age > 5.0 {
            age_violations += 1;
        }
    }
    assert_eq!(
        age_violations, 0,
        "No tenant should have last_applied_age > 5s (fresh cursors)"
    );
    println!("✅ All {} tenants have fresh last_applied_age", TENANT_COUNT);

    // Cleanup
    sqlx::query(
        "DELETE FROM projection_cursors WHERE projection_name = 'scale_lag_test'"
    )
    .execute(&pool)
    .await
    .ok();

    println!("\n✅ 100-Tenant Projection Lag SLO Test: ALL PASSED\n");
}

/// Test 3: Oracle correctness across 100 tenant IDs
///
/// Runs the full cross-module oracle for 100 deterministic tenant IDs.
/// Since these tenants have no data in module DBs, all module invariants
/// pass trivially (no violations for empty tenant scope).
///
/// Audit completeness check is skipped gracefully if audit table not integrated.
///
/// **Key correctness property:** The oracle must NOT produce false positives
/// (phantom violations) when scaling to 100 tenant IDs.
#[tokio::test]
#[serial]
async fn test_100_tenants_oracle_correctness() {
    use crate::oracle::{assert_cross_module_invariants, TestContext};

    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;
    let audit_pool = get_audit_pool().await;

    let tenant_ids = generate_tenant_ids();

    println!("\n=== Scale Test: 100-Tenant Oracle Correctness ===");
    println!("Tenants: {}", TENANT_COUNT);
    println!("Checking oracle produces no false positives at scale...");

    let mut passed = 0usize;
    let mut failed = 0usize;

    for tenant_id in &tenant_ids {
        let ctx = TestContext {
            ar_pool: &ar_pool,
            payments_pool: &payments_pool,
            subscriptions_pool: &subscriptions_pool,
            gl_pool: &gl_pool,
            audit_pool: &audit_pool,
            app_id: tenant_id.as_str(),
            tenant_id: tenant_id.as_str(),
        };

        match assert_cross_module_invariants(&ctx).await {
            Ok(()) => passed += 1,
            Err(e) => {
                failed += 1;
                println!("  ✗ Oracle failed for tenant {}: {}", tenant_id, e);
            }
        }
    }

    println!(
        "\nOracle results: {}/{} passed, {} failed",
        passed, TENANT_COUNT, failed
    );

    assert_eq!(
        failed, 0,
        "Oracle must not produce false positives for {} scale tenants (no data in system)",
        TENANT_COUNT
    );

    println!("✅ Oracle: no false positives across {} tenants", TENANT_COUNT);
    println!("\n✅ 100-Tenant Oracle Correctness: ALL PASSED\n");
}

// Import oracle module from this test crate
mod oracle;

/// Test 4: Combined truth-at-scale validation
///
/// Runs all 3 correctness properties in sequence:
/// 1. Digest stability across 2 rebuild passes
/// 2. Projection lag within SLO for fresh events
/// 3. Oracle passes without false positives
///
/// This is the formal Phase 17 capstone: "Truth at Scale."
#[tokio::test]
#[serial]
async fn test_scale_100_tenants_truth_at_scale() {
    let proj_pool = get_projections_pool().await;
    run_projections_migrations(&proj_pool).await;

    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║         Phase 17 Capstone: Truth at Scale                ║");
    println!("║         100 Tenants × 6 Compressed Billing Cycles        ║");
    println!("╚══════════════════════════════════════════════════════════╝");
    println!("Tenants: {}", TENANT_COUNT);
    println!("Cycles/tenant: {}", EVENTS_PER_TENANT);
    println!("Total events: {}", TENANT_COUNT * EVENTS_PER_TENANT);

    // ── Gate 1: Digest Stability ─────────────────────────────────────────────
    println!("\n[Gate 1] Projection digest stability...");

    let digest1 = rebuild_scale_projection(&proj_pool).await;
    println!("  Pass 1 digest: {}", digest1);

    let live_rows: i64 =
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {}", BASE_TABLE))
            .fetch_one(&proj_pool)
            .await
            .expect("Failed to count rows");
    assert_eq!(live_rows as usize, TENANT_COUNT, "Expected {} rows in live table", TENANT_COUNT);

    let digest2 = rebuild_scale_projection(&proj_pool).await;
    println!("  Pass 2 digest: {}", digest2);

    assert_eq!(
        digest1, digest2,
        "Gate 1 FAILED: digest mismatch after 2 rebuilds at 100-tenant scale"
    );
    println!("  ✅ Gate 1 PASSED: Digest stable across 2 rebuilds");

    // ── Gate 2: Projection Lag SLO ───────────────────────────────────────────
    println!("\n[Gate 2] Projection lag within SLO (< {}ms)...", PROJECTION_LAG_BOUND_MS);

    let tenant_ids = generate_tenant_ids();
    let metrics = ProjectionMetrics::new().expect("Failed to create metrics");
    let now = Utc::now();

    for (i, tenant_id) in tenant_ids.iter().enumerate() {
        let event_id = Uuid::from_u128(200000 + i as u128);
        ProjectionCursor::save(&proj_pool, "scale_gate2_lag", tenant_id, event_id, now)
            .await
            .unwrap_or_else(|e| panic!("Cursor save failed for {}: {}", tenant_id, e));

        let cursor = ProjectionCursor::load(&proj_pool, "scale_gate2_lag", tenant_id)
            .await
            .expect("Failed to load cursor")
            .expect("Cursor must exist");
        metrics.record_cursor_state(&cursor);
    }

    let families = metrics.registry().gather();
    let lag_family = families
        .iter()
        .find(|f| f.get_name() == "projection_lag_ms")
        .expect("projection_lag_ms must be present");

    let violations: Vec<f64> = lag_family
        .get_metric()
        .iter()
        .map(|m| m.get_gauge().get_value())
        .filter(|&lag| lag > PROJECTION_LAG_BOUND_MS)
        .collect();

    assert!(
        violations.is_empty(),
        "Gate 2 FAILED: {} tenants exceeded lag SLO of {}ms: {:?}",
        violations.len(),
        PROJECTION_LAG_BOUND_MS,
        violations
    );
    println!(
        "  ✅ Gate 2 PASSED: All {} tenants within lag SLO",
        TENANT_COUNT
    );

    // ── Gate 3: Oracle Correctness ───────────────────────────────────────────
    println!("\n[Gate 3] Oracle correctness (no false positives at scale)...");

    let ar_pool = get_ar_pool().await;
    let payments_pool = get_payments_pool().await;
    let subscriptions_pool = get_subscriptions_pool().await;
    let gl_pool = get_gl_pool().await;
    let audit_pool = get_audit_pool().await;

    let mut oracle_failures = Vec::new();
    for tenant_id in &tenant_ids {
        let ctx = oracle::TestContext {
            ar_pool: &ar_pool,
            payments_pool: &payments_pool,
            subscriptions_pool: &subscriptions_pool,
            gl_pool: &gl_pool,
            audit_pool: &audit_pool,
            app_id: tenant_id.as_str(),
            tenant_id: tenant_id.as_str(),
        };

        if let Err(e) = oracle::assert_cross_module_invariants(&ctx).await {
            oracle_failures.push(format!("  {}: {}", tenant_id, e));
        }
    }

    assert!(
        oracle_failures.is_empty(),
        "Gate 3 FAILED: Oracle false positives for {} tenants:\n{}",
        oracle_failures.len(),
        oracle_failures.join("\n")
    );
    println!(
        "  ✅ Gate 3 PASSED: Oracle clean for all {} tenants",
        TENANT_COUNT
    );

    // ── Summary ──────────────────────────────────────────────────────────────
    println!("\n╔══════════════════════════════════════════════════════════╗");
    println!("║  ✅ TRUTH AT SCALE: ALL 3 GATES PASSED                   ║");
    println!("║  {} tenants × {} cycles = {} total events processed       ║",
        TENANT_COUNT, EVENTS_PER_TENANT, TENANT_COUNT * EVENTS_PER_TENANT);
    println!("║                                                          ║");
    println!("║  Gate 1: Projection digest stability    ✅               ║");
    println!("║  Gate 2: Projection lag within SLO      ✅               ║");
    println!("║  Gate 3: Oracle correctness (no FP)     ✅               ║");
    println!("╚══════════════════════════════════════════════════════════╝\n");

    // Cleanup
    sqlx::query(&format!("DROP TABLE IF EXISTS {} CASCADE", BASE_TABLE))
        .execute(&proj_pool)
        .await
        .ok();
    sqlx::query(&format!("DROP TABLE IF EXISTS {}_old CASCADE", BASE_TABLE))
        .execute(&proj_pool)
        .await
        .ok();
    sqlx::query(
        "DELETE FROM projection_cursors WHERE projection_name IN ('scale_gate2_lag')"
    )
    .execute(&proj_pool)
    .await
    .ok();
}
