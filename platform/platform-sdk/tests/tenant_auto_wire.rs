//! Integration test: SDK auto-wires TenantPoolResolver to `tenant.provisioned`.
//!
//! Verifies that a module registering a TenantPoolResolver without an explicit
//! on_tenant_provisioned callback still has pool_for() called when the
//! tenant.provisioned event fires — no manual callback needed.
//!
//! Uses InMemoryBus (a real EventBus implementation — no NATS required).
//! The pool_for lookup intentionally returns an error; only the *call* matters.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use event_bus::{EventBus, InMemoryBus};
use platform_sdk::provisioning_hook::{test_payload, wire_pool_resolver_auto_register};
use platform_sdk::{DefaultTenantResolver, Manifest, ModuleContext, TenantPoolError};
use uuid::Uuid;

fn lazy_pool() -> sqlx::PgPool {
    // connect_lazy does not open a connection — safe for tests that don't
    // touch the default pool (only the resolver's pool_for is invoked).
    sqlx::PgPool::connect_lazy("postgres://unused:unused@localhost/unused")
        .expect("connect_lazy is infallible")
}

fn test_manifest() -> Manifest {
    Manifest::from_str(
        "[module]\nname = \"test-auto-wire\"\nversion = \"0.1.0\"",
        None,
    )
    .expect("valid minimal manifest")
}

// ──────────────────────────────────────────────────────────────────────────────
// Test: pool_for is called automatically when tenant.provisioned fires
// ──────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tenant_auto_wire() {
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let ctx = ModuleContext::new(lazy_pool(), test_manifest(), Some(bus.clone()));

    // Track calls to pool_for (retries may call multiple times — assert >= 1).
    let lookup_count = Arc::new(AtomicUsize::new(0));
    let received_id: Arc<tokio::sync::Mutex<Option<Uuid>>> =
        Arc::new(tokio::sync::Mutex::new(None));

    let lc = lookup_count.clone();
    let ri = received_id.clone();

    // Resolver that records the first call and returns an error (no real DB).
    let resolver = Arc::new(DefaultTenantResolver::new(move |tenant_id| {
        let count = lc.clone();
        let id = ri.clone();
        async move {
            if count.fetch_add(1, Ordering::SeqCst) == 0 {
                *id.lock().await = Some(tenant_id);
            }
            Err::<String, _>(TenantPoolError::UnknownTenant(tenant_id))
        }
    }));

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let handle = wire_pool_resolver_auto_register(resolver, &bus, &ctx, shutdown_rx)
        .await
        .expect("wire_pool_resolver_auto_register must succeed");

    // Publish a synthetic tenant.provisioned event.
    let tenant_id = Uuid::new_v4();
    bus.publish("tenant.provisioned", test_payload(tenant_id))
        .await
        .expect("publish tenant.provisioned");

    // Wait for the hook to process (retry backoff: 100 ms → 200 ms → done).
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    assert!(
        lookup_count.load(Ordering::SeqCst) >= 1,
        "pool_for must be called at least once for the provisioned tenant"
    );
    assert_eq!(
        *received_id.lock().await,
        Some(tenant_id),
        "pool_for must be called with the correct tenant_id"
    );

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}
