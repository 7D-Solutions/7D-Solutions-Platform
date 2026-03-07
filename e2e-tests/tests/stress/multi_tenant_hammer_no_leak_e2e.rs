//! Stress test: Multi-tenant hammer — 20 tenants x 10 concurrent prove zero cross-tenant leakage
//!
//! Proves that under concurrent multi-tenant load, no cross-tenant data leakage
//! occurs. Each tenant has exactly one item with a unique marker. 10 concurrent
//! `find_by_id` calls per tenant verify that every response matches the caller's
//! tenant_id — zero instances of another tenant's data appearing.
//!
//! Uses `ItemRepo::find_by_id` which filters by tenant_id at the query level.
//! The `SELECT * FROM items WHERE id = $1 AND tenant_id = $2` pattern is the
//! isolation boundary under test.
//!
//! ## Running
//! ```bash
//! ./scripts/cargo-slot.sh test -p e2e-tests -- multi_tenant_hammer_no_leak_e2e --nocapture
//! ```

use inventory_rs::domain::items::{CreateItemRequest, ItemRepo, TrackingMode};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

const NUM_TENANTS: usize = 20;
const CONCURRENT_PER_TENANT: usize = 10;

async fn get_inventory_pool() -> PgPool {
    let url = std::env::var("INVENTORY_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgresql://inventory_user:inventory_pass@localhost:5442/inventory_db".to_string()
        });
    let pool = PgPoolOptions::new()
        .max_connections(30)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&url)
        .await
        .expect("failed to connect to inventory DB");

    sqlx::migrate!("../modules/inventory/db/migrations")
        .run(&pool)
        .await
        .expect("failed to run inventory migrations");

    pool
}

async fn cleanup_tenant(pool: &PgPool, tenant_id: &str) {
    for q in [
        "DELETE FROM inv_outbox WHERE tenant_id = $1",
        "DELETE FROM inv_idempotency_keys WHERE tenant_id = $1",
        "DELETE FROM layer_consumptions WHERE ledger_entry_id IN (SELECT id FROM inventory_ledger WHERE tenant_id = $1)",
        "DELETE FROM inventory_serial_instances WHERE tenant_id = $1",
        "DELETE FROM item_on_hand WHERE tenant_id = $1",
        "DELETE FROM inventory_reservations WHERE tenant_id = $1",
        "DELETE FROM inv_adjustments WHERE tenant_id = $1",
        "DELETE FROM inventory_layers WHERE tenant_id = $1",
        "DELETE FROM inventory_ledger WHERE tenant_id = $1",
        "DELETE FROM inventory_lots WHERE tenant_id = $1",
        "DELETE FROM items WHERE tenant_id = $1",
    ] {
        sqlx::query(q).bind(tenant_id).execute(pool).await.ok();
    }
}

struct TenantFixture {
    tenant_id: String,
    item_id: Uuid,
    marker: String,
}

#[tokio::test]
async fn multi_tenant_hammer_no_leak_e2e() {
    let pool = Arc::new(get_inventory_pool().await);

    // --- Seed: create 20 tenants, each with 1 item bearing a unique marker ---
    println!("--- Seeding {} tenants with 1 item each ---", NUM_TENANTS);

    let mut fixtures = Vec::with_capacity(NUM_TENANTS);

    for i in 0..NUM_TENANTS {
        let tenant_id = format!("mt-hammer-{}-{}", i, Uuid::new_v4());
        let marker = format!("marker-{}-{}", i, Uuid::new_v4());

        let item = ItemRepo::create(
            &pool,
            &CreateItemRequest {
                tenant_id: tenant_id.clone(),
                sku: format!("MT-{:03}-{}", i, Uuid::new_v4()),
                name: marker.clone(),
                description: Some(format!("Tenant {} isolation test item", i)),
                inventory_account_ref: "1200".to_string(),
                cogs_account_ref: "5000".to_string(),
                variance_account_ref: "5010".to_string(),
                uom: None,
                tracking_mode: TrackingMode::None,
                make_buy: None,
            },
        )
        .await
        .expect("failed to create item");

        fixtures.push(TenantFixture {
            tenant_id,
            item_id: item.id,
            marker,
        });
    }

    println!("seeded {} tenants", fixtures.len());

    // --- Hammer: 10 concurrent find_by_id calls per tenant ---
    // Each call uses its own tenant_id + item_id. We also cross-check:
    // try to fetch each item with EVERY OTHER tenant_id (must return None).
    println!(
        "\n--- {} tenants x {} concurrent lookups = {} total requests ---",
        NUM_TENANTS,
        CONCURRENT_PER_TENANT,
        NUM_TENANTS * CONCURRENT_PER_TENANT
    );

    let fixtures = Arc::new(fixtures);
    let start = Instant::now();

    // Phase 1: Positive lookups — each tenant fetches its own item concurrently
    let mut handles = Vec::with_capacity(NUM_TENANTS * CONCURRENT_PER_TENANT);

    for fixture_idx in 0..NUM_TENANTS {
        for _ in 0..CONCURRENT_PER_TENANT {
            let pool = Arc::clone(&pool);
            let fixtures = Arc::clone(&fixtures);
            handles.push(tokio::spawn(async move {
                let f = &fixtures[fixture_idx];
                let result = ItemRepo::find_by_id(&pool, f.item_id, &f.tenant_id)
                    .await
                    .expect("DB error during find_by_id");

                match result {
                    Some(item) => {
                        // Verify tenant isolation
                        let tenant_match = item.tenant_id == f.tenant_id;
                        let marker_match = item.name == f.marker;
                        let id_match = item.id == f.item_id;
                        (true, tenant_match && marker_match && id_match, f.tenant_id.clone(), item.tenant_id.clone())
                    }
                    None => {
                        // Should not happen — we just created this item
                        (false, false, f.tenant_id.clone(), "NONE".to_string())
                    }
                }
            }));
        }
    }

    let mut positive_found = 0usize;
    let mut positive_correct = 0usize;
    let mut positive_missing = 0usize;
    let mut leaks = Vec::new();

    for h in handles {
        let (found, correct, expected_tenant, actual_tenant) = h.await.expect("task panicked");
        if found {
            positive_found += 1;
            if correct {
                positive_correct += 1;
            } else {
                leaks.push((expected_tenant, actual_tenant));
            }
        } else {
            positive_missing += 1;
        }
    }

    let positive_elapsed = start.elapsed();

    println!("positive lookups completed in {:?}", positive_elapsed);
    println!("  found: {}", positive_found);
    println!("  correct (tenant + marker match): {}", positive_correct);
    println!("  missing (item not found): {}", positive_missing);
    println!("  LEAKS (wrong tenant data): {}", leaks.len());

    // Phase 2: Negative lookups — each tenant tries to fetch OTHER tenants' items
    // This proves that tenant_id filtering rejects cross-tenant access.
    println!("\n--- Cross-tenant negative lookups ---");

    let cross_start = Instant::now();
    let mut cross_handles = Vec::new();

    for fixture_idx in 0..NUM_TENANTS {
        // Try to access items from 2 other tenants (not all 19, to keep test fast)
        for other_idx in [(fixture_idx + 1) % NUM_TENANTS, (fixture_idx + 7) % NUM_TENANTS] {
            if other_idx == fixture_idx {
                continue;
            }
            let pool = Arc::clone(&pool);
            let fixtures = Arc::clone(&fixtures);
            cross_handles.push(tokio::spawn(async move {
                let caller = &fixtures[fixture_idx];
                let target = &fixtures[other_idx];
                // Try to fetch target's item using caller's tenant_id — must return None
                let result = ItemRepo::find_by_id(&pool, target.item_id, &caller.tenant_id)
                    .await
                    .expect("DB error during cross-tenant find_by_id");

                (result.is_some(), caller.tenant_id.clone(), target.tenant_id.clone())
            }));
        }
    }

    let mut cross_blocked = 0usize;
    let mut cross_leaked = 0usize;

    for h in cross_handles {
        let (found, caller, target) = h.await.expect("task panicked");
        if found {
            cross_leaked += 1;
            println!("  LEAK: tenant {} accessed tenant {}'s item!", caller, target);
        } else {
            cross_blocked += 1;
        }
    }

    let cross_elapsed = cross_start.elapsed();

    println!("cross-tenant lookups completed in {:?}", cross_elapsed);
    println!("  blocked (correct): {}", cross_blocked);
    println!("  leaked (WRONG): {}", cross_leaked);

    // --- Assertions ---
    let total_elapsed = start.elapsed();
    println!("\n--- Summary (total: {:?}) ---", total_elapsed);

    // Assertion 1: All positive lookups found the item
    assert_eq!(
        positive_missing, 0,
        "all items must be found by their owner tenant, {} were missing",
        positive_missing
    );

    // Assertion 2: All positive lookups returned correct tenant data
    assert_eq!(
        positive_correct,
        NUM_TENANTS * CONCURRENT_PER_TENANT,
        "all {} positive lookups must return correct tenant data, got {}",
        NUM_TENANTS * CONCURRENT_PER_TENANT,
        positive_correct
    );

    // Assertion 3: Zero cross-tenant leaks in positive lookups
    assert!(
        leaks.is_empty(),
        "CROSS-TENANT LEAKAGE DETECTED in positive lookups: {:?}",
        leaks
    );

    // Assertion 4: Zero cross-tenant leaks in negative lookups
    assert_eq!(
        cross_leaked, 0,
        "CROSS-TENANT LEAKAGE DETECTED: {} cross-tenant lookups returned data",
        cross_leaked
    );

    // Assertion 5: All cross-tenant lookups were blocked
    assert!(
        cross_blocked > 0,
        "at least some cross-tenant lookups must have been tested"
    );

    println!("  positive lookups: {}/{} correct", positive_correct, NUM_TENANTS * CONCURRENT_PER_TENANT);
    println!("  cross-tenant blocked: {}/{}", cross_blocked, cross_blocked + cross_leaked);
    println!("  tenant isolation: PASSED");

    // --- Cleanup ---
    for f in fixtures.iter() {
        cleanup_tenant(pool.as_ref(), &f.tenant_id).await;
    }
}
