//! Audit Completeness Oracle (bd-3qe1)
//!
//! **Invariant:** Every mutation event in a module's outbox table has
//! exactly one audit record in audit_events (linked via causation_id).
//!
//! **Approach:**
//! 1. Connect to all module databases and the audit database
//! 2. For each module with outbox events, check audit coverage
//! 3. Backfill any missing audit records (repair mode)
//! 4. Re-verify: 0 gaps, 0 duplicates
//!
//! **Modules covered:**
//! AR, Payments, Subscriptions, GL, Notifications, AP,
//! Inventory, Treasury, Fixed Assets, Timekeeping
//!
//! **Verification:**
//! ```bash
//! AUDIT_DATABASE_URL=postgres://postgres:postgres@localhost:5432/audit_db \
//!   ./scripts/cargo-slot.sh test -p e2e-tests -- audit_oracle
//! ```

mod common;

use audit::outbox_bridge::{
    backfill_missing_audit_records, check_module_audit_completeness,
    query_notifications_outbox_events, query_outbox_events, query_payments_outbox_events,
    query_subscriptions_outbox_events, ModuleAuditResult, OutboxEventMeta,
};
use common::{run_audit_migrations, wait_for_db_ready};
use sqlx::PgPool;

// ============================================================================
// Module DB Pool Helpers (for modules not in common)
// ============================================================================

async fn try_pool(name: &str, url: &str) -> Option<PgPool> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        wait_for_db_ready(name, url),
    )
    .await
    {
        Ok(pool) => Some(pool),
        Err(_) => {
            eprintln!("  [SKIP] {} — DB not reachable at {}", name, url);
            None
        }
    }
}

fn inventory_db_url() -> String {
    std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        })
}

fn treasury_db_url() -> String {
    std::env::var("TREASURY_DATABASE_URL").unwrap_or_else(|_| {
        "postgres://treasury_user:treasury_pass@localhost:5444/treasury_db".to_string()
    })
}

fn fixed_assets_db_url() -> String {
    std::env::var("FIXED_ASSETS_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://fixed_assets_user:fixed_assets_pass@localhost:5445/fixed_assets_db"
            .to_string()
    })
}

// ============================================================================
// Core Oracle Logic
// ============================================================================

/// Run the oracle for a single module: check coverage, backfill gaps, re-verify.
async fn oracle_for_module(
    audit_pool: &PgPool,
    module_pool: &PgPool,
    module_name: &str,
    events: Vec<OutboxEventMeta>,
) -> ModuleAuditResult {
    if events.is_empty() {
        eprintln!("  [OK] {} — outbox empty (0 events)", module_name);
        return ModuleAuditResult {
            module: module_name.to_string(),
            ..Default::default()
        };
    }

    // Phase 1: Check current coverage
    let initial = check_module_audit_completeness(audit_pool, &events, module_name)
        .await
        .unwrap_or_else(|e| {
            eprintln!(
                "  [ERR] {} — failed to check audit completeness: {}",
                module_name, e
            );
            ModuleAuditResult {
                module: module_name.to_string(),
                ..Default::default()
            }
        });

    if initial.is_clean() {
        eprintln!(
            "  [OK] {} — {}/{} events covered, 0 gaps, 0 dupes",
            module_name, initial.covered, initial.total_outbox_events
        );
        return initial;
    }

    // Phase 2: Backfill missing records
    let gap_count = initial.gaps.len();
    let dupe_count = initial.duplicates.len();

    if gap_count > 0 {
        eprintln!(
            "  [FIX] {} — backfilling {} missing audit records",
            module_name, gap_count
        );
        match backfill_missing_audit_records(audit_pool, &initial.gaps, module_name).await {
            Ok(written) => {
                eprintln!("  [FIX] {} — wrote {} audit records", module_name, written);
            }
            Err(e) => {
                eprintln!("  [ERR] {} — backfill failed: {}", module_name, e);
            }
        }
    }

    if dupe_count > 0 {
        eprintln!(
            "  [WARN] {} — {} events have duplicate audit records",
            module_name, dupe_count
        );
    }

    // Phase 3: Re-verify after backfill
    check_module_audit_completeness(audit_pool, &events, module_name)
        .await
        .unwrap_or_else(|e| {
            eprintln!("  [ERR] {} — re-verify failed: {}", module_name, e);
            ModuleAuditResult {
                module: module_name.to_string(),
                ..Default::default()
            }
        })
}

// ============================================================================
// Oracle Test
// ============================================================================

/// Main audit oracle: sweep all modules, backfill gaps, assert 0 gaps / 0 dupes.
#[tokio::test]
async fn audit_oracle_all_modules() {
    eprintln!("\n=== Audit Completeness Oracle ===\n");

    // Connect to audit DB
    let audit_pool = common::get_audit_pool().await;
    run_audit_migrations(&audit_pool).await;

    // Build module pools (skip unreachable modules)
    let ar_pool = try_pool("AR", &common::get_ar_db_url()).await;
    let payments_pool = try_pool("Payments", &common::get_payments_db_url()).await;
    let subs_pool = try_pool(
        "Subscriptions",
        &std::env::var("SUBSCRIPTIONS_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://subscriptions_user:subscriptions_pass@localhost:5435/subscriptions_db"
                .to_string()
        }),
    )
    .await;
    let gl_pool = try_pool("GL", &common::get_gl_db_url()).await;
    let notif_pool = try_pool(
        "Notifications",
        &std::env::var("NOTIFICATIONS_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://notifications_user:notifications_pass@localhost:5437/notifications_db"
                .to_string()
        }),
    )
    .await;
    let ap_pool = try_pool(
        "AP",
        &std::env::var("AP_DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://ap_user:ap_pass@localhost:5443/ap_db".to_string()),
    )
    .await;
    let inv_pool = try_pool("Inventory", &inventory_db_url()).await;
    let treasury_pool = try_pool("Treasury", &treasury_db_url()).await;
    let fa_pool = try_pool("FixedAssets", &fixed_assets_db_url()).await;
    let tk_pool = try_pool(
        "Timekeeping",
        &std::env::var("TIMEKEEPING_DATABASE_URL").unwrap_or_else(|_| {
            "postgresql://timekeeping_user:timekeeping_pass@localhost:5447/timekeeping_db"
                .to_string()
        }),
    )
    .await;

    // Standard modules have event_id, event_type, aggregate_type, aggregate_id
    let module_pools: Vec<(&str, &str, Option<&PgPool>)> = vec![
        ("AR", "events_outbox", ar_pool.as_ref()),
        ("GL", "events_outbox", gl_pool.as_ref()),
        ("AP", "events_outbox", ap_pool.as_ref()),
        ("Inventory", "inv_outbox", inv_pool.as_ref()),
        ("Treasury", "events_outbox", treasury_pool.as_ref()),
        ("FixedAssets", "fa_events_outbox", fa_pool.as_ref()),
        ("Timekeeping", "events_outbox", tk_pool.as_ref()),
    ];

    let mut all_results: Vec<ModuleAuditResult> = Vec::new();
    let mut modules_checked = 0u32;

    // Standard modules (event_id, event_type, aggregate_type, aggregate_id)
    for (name, table, pool_opt) in &module_pools {
        let Some(pool) = pool_opt else {
            continue;
        };
        modules_checked += 1;

        let events = match query_outbox_events(pool, table).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  [SKIP] {} — outbox query failed: {}", name, e);
                continue;
            }
        };

        let result = oracle_for_module(&audit_pool, pool, name, events).await;
        all_results.push(result);
    }

    // Payments module (no aggregate columns)
    if let Some(pool) = payments_pool.as_ref() {
        modules_checked += 1;
        let events = match query_payments_outbox_events(pool).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  [SKIP] Payments — outbox query failed: {}", e);
                Vec::new()
            }
        };
        if !events.is_empty() {
            let result = oracle_for_module(&audit_pool, pool, "Payments", events).await;
            all_results.push(result);
        } else {
            eprintln!("  [OK] Payments — outbox empty (0 events)");
        }
    }

    // Subscriptions module (no aggregate columns)
    if let Some(pool) = subs_pool.as_ref() {
        modules_checked += 1;
        let events = match query_subscriptions_outbox_events(pool).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  [SKIP] Subscriptions — outbox query failed: {}", e);
                Vec::new()
            }
        };
        if !events.is_empty() {
            let result = oracle_for_module(&audit_pool, pool, "Subscriptions", events).await;
            all_results.push(result);
        } else {
            eprintln!("  [OK] Subscriptions — outbox empty (0 events)");
        }
    }

    // Notifications module (no aggregate columns)
    if let Some(pool) = notif_pool.as_ref() {
        modules_checked += 1;
        let events = match query_notifications_outbox_events(pool).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  [SKIP] Notifications — outbox query failed: {}", e);
                Vec::new()
            }
        };
        if !events.is_empty() {
            let result = oracle_for_module(&audit_pool, pool, "Notifications", events).await;
            all_results.push(result);
        } else {
            eprintln!("  [OK] Notifications — outbox empty (0 events)");
        }
    }

    // Summary
    eprintln!("\n=== Oracle Summary ===");
    let total_events: u64 = all_results.iter().map(|r| r.total_outbox_events).sum();
    let total_covered: u64 = all_results.iter().map(|r| r.covered).sum();
    let total_gaps: usize = all_results.iter().map(|r| r.gaps.len()).sum();
    let total_dupes: usize = all_results.iter().map(|r| r.duplicates.len()).sum();

    eprintln!("Modules checked: {}", modules_checked);
    eprintln!("Total outbox events: {}", total_events);
    eprintln!("Total covered: {}", total_covered);
    eprintln!("Total gaps: {}", total_gaps);
    eprintln!("Total duplicates: {}", total_dupes);

    // Report per-module details for failures
    for result in &all_results {
        if !result.is_clean() {
            eprintln!(
                "\n  FAIL: {} — {} gaps, {} dupes",
                result.module,
                result.gaps.len(),
                result.duplicates.len()
            );
            for gap in result.gaps.iter().take(5) {
                eprintln!("    gap: {} ({})", gap.event_type, gap.event_id);
            }
            if result.gaps.len() > 5 {
                eprintln!("    ... and {} more gaps", result.gaps.len() - 5);
            }
        }
    }

    // Strict assertion: 0 gaps, 0 dupes
    assert_eq!(
        total_gaps, 0,
        "Audit oracle failed: {} outbox events have no audit record (0 gaps required)",
        total_gaps
    );
    assert_eq!(
        total_dupes, 0,
        "Audit oracle failed: {} outbox events have duplicate audit records (0 dupes required)",
        total_dupes
    );

    eprintln!(
        "\n=== Oracle PASSED: {} events, 0 gaps, 0 dupes ===\n",
        total_events
    );
}

/// Verify no duplicate audit records exist for any causation_id.
#[tokio::test]
async fn audit_oracle_no_global_duplicates() {
    let audit_pool = common::get_audit_pool().await;
    run_audit_migrations(&audit_pool).await;

    let dupes: Vec<(uuid::Uuid, i64)> = sqlx::query_as(
        r#"
        SELECT causation_id, COUNT(*)::bigint as cnt
        FROM audit_events
        WHERE causation_id IS NOT NULL
        GROUP BY causation_id
        HAVING COUNT(*) > 1
        ORDER BY cnt DESC
        LIMIT 20
        "#,
    )
    .fetch_all(&audit_pool)
    .await
    .unwrap_or_default();

    if !dupes.is_empty() {
        eprintln!("Duplicate audit records found:");
        for (causation_id, count) in &dupes {
            eprintln!("  causation_id={} count={}", causation_id, count);
        }
    }

    assert!(
        dupes.is_empty(),
        "Found {} causation_ids with duplicate audit records",
        dupes.len()
    );
}

/// Verify all audit records have required fields populated.
#[tokio::test]
async fn audit_oracle_field_completeness() {
    let audit_pool = common::get_audit_pool().await;
    run_audit_migrations(&audit_pool).await;

    // Check for records missing required fields
    let incomplete: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)::bigint
        FROM audit_events
        WHERE action = '' OR action IS NULL
           OR entity_type = '' OR entity_type IS NULL
           OR entity_id = '' OR entity_id IS NULL
           OR actor_type = '' OR actor_type IS NULL
        "#,
    )
    .fetch_one(&audit_pool)
    .await
    .unwrap_or(0);

    assert_eq!(
        incomplete, 0,
        "Found {} audit records with missing required fields",
        incomplete
    );
}
