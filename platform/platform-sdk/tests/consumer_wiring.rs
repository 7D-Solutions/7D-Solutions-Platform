//! Integration tests for SDK consumer wiring and retry.
//!
//! Uses InMemoryBus (a real EventBus implementation) and a real Postgres pool.
//! Set `DATABASE_URL` to run these tests.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use event_bus::{EventBus, EventEnvelope, InMemoryBus};
use platform_sdk::consumer::ConsumerDef;
use platform_sdk::{ConsumerError, Manifest, ModuleContext};

/// Connect to the test database or skip.
async fn test_pool() -> Option<sqlx::PgPool> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("DATABASE_URL not set — skipping integration test");
            return None;
        }
    };
    Some(
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .expect("failed to connect to test database"),
    )
}

fn test_manifest() -> Manifest {
    Manifest::from_str(
        r#"
[module]
name = "test-consumer-wiring"

[bus]
type = "inmemory"
"#,
        None,
    )
    .expect("test manifest should parse")
}

fn test_envelope(event_type: &str) -> EventEnvelope<serde_json::Value> {
    EventEnvelope::new(
        "tenant-test".into(),
        "test-module".into(),
        event_type.into(),
        serde_json::json!({"key": "value"}),
    )
}

// ──────────────────────────────────────────────────────────────────
// Test 1: Consumer receives and processes an event
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn consumer_receives_event() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();

    let consumers = vec![ConsumerDef::new("test.event", move |_ctx, _env| {
        let count = count_clone.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    })];

    let handles = platform_sdk::consumer::wire_consumers(consumers, &bus, &ctx)
        .await
        .expect("wire_consumers should succeed");

    // Publish a test envelope
    let envelope = test_envelope("test.event");
    let payload = serde_json::to_vec(&envelope).unwrap();
    bus.publish("test.event", payload).await.unwrap();

    // Wait for processing
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "handler should be called once"
    );

    handles.shutdown().await;
}

// ──────────────────────────────────────────────────────────────────
// Test 2: Consumer retries on failure (3 attempts)
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn consumer_retries_on_failure() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let count_clone = attempt_count.clone();

    let consumers = vec![ConsumerDef::new("test.retry", move |_ctx, _env| {
        let count = count_clone.clone();
        async move {
            let attempt = count.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt < 3 {
                Err(ConsumerError::Processing(format!(
                    "transient error attempt {attempt}"
                )))
            } else {
                Ok(())
            }
        }
    })];

    let handles = platform_sdk::consumer::wire_consumers(consumers, &bus, &ctx)
        .await
        .expect("wire_consumers should succeed");

    let envelope = test_envelope("test.retry");
    let payload = serde_json::to_vec(&envelope).unwrap();
    bus.publish("test.retry", payload).await.unwrap();

    // Wait for retries (100ms + 200ms backoff + processing)
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    assert_eq!(
        attempt_count.load(Ordering::SeqCst),
        3,
        "handler should be called 3 times (2 failures + 1 success)"
    );

    handles.shutdown().await;
}

// ──────────────────────────────────────────────────────────────────
// Test 3: Consumer exhausts retries and logs error
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn consumer_exhausts_retries() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let count_clone = attempt_count.clone();

    let consumers = vec![ConsumerDef::new("test.exhaust", move |_ctx, _env| {
        let count = count_clone.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            Err(ConsumerError::Processing("persistent error".into()))
        }
    })];

    let handles = platform_sdk::consumer::wire_consumers(consumers, &bus, &ctx)
        .await
        .expect("wire_consumers should succeed");

    let envelope = test_envelope("test.exhaust");
    let payload = serde_json::to_vec(&envelope).unwrap();
    bus.publish("test.exhaust", payload).await.unwrap();

    // Wait for all retries (100ms + 200ms backoff + processing)
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    assert_eq!(
        attempt_count.load(Ordering::SeqCst),
        3,
        "handler should be called exactly 3 times (max_attempts default)"
    );

    handles.shutdown().await;
}

// ──────────────────────────────────────────────────────────────────
// Test 4: Shutdown drains consumers cleanly
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn shutdown_drains_consumers() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let consumers = vec![ConsumerDef::new(
        "test.shutdown",
        |_ctx, _env| async { Ok(()) },
    )];

    let handles = platform_sdk::consumer::wire_consumers(consumers, &bus, &ctx)
        .await
        .expect("wire_consumers should succeed");

    // Shutdown without publishing — should complete quickly
    let start = std::time::Instant::now();
    handles.shutdown().await;
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "shutdown should complete quickly when idle"
    );
}

// ──────────────────────────────────────────────────────────────────
// Test 5: Multiple consumers on different subjects
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn multiple_consumers_independent() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let count_a = Arc::new(AtomicUsize::new(0));
    let count_b = Arc::new(AtomicUsize::new(0));

    let ca = count_a.clone();
    let cb = count_b.clone();

    let consumers = vec![
        ConsumerDef::new("test.multi.a", move |_ctx, _env| {
            let count = ca.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }),
        ConsumerDef::new("test.multi.b", move |_ctx, _env| {
            let count = cb.clone();
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        }),
    ];

    let handles = platform_sdk::consumer::wire_consumers(consumers, &bus, &ctx)
        .await
        .expect("wire_consumers should succeed");

    // Publish to subject a only
    let envelope = test_envelope("test.multi.a");
    let payload = serde_json::to_vec(&envelope).unwrap();
    bus.publish("test.multi.a", payload).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(
        count_a.load(Ordering::SeqCst),
        1,
        "handler_a should be called"
    );
    assert_eq!(
        count_b.load(Ordering::SeqCst),
        0,
        "handler_b should not be called"
    );

    handles.shutdown().await;
}

// ──────────────────────────────────────────────────────────────────
// Test 6: Provisioning hook receives tenant.provisioned event
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn provisioning_hook_receives_event() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();

    let received_id = Arc::new(tokio::sync::Mutex::new(None));
    let id_clone = received_id.clone();

    let handler: platform_sdk::consumer::ProvisioningHandler = Arc::new(move |_ctx, event| {
        let count = count_clone.clone();
        let id = id_clone.clone();
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            *id.lock().await = Some(event.tenant_id);
            Ok(())
        })
    });

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let handle = platform_sdk::consumer::wire_provisioning_hook(
        handler,
        &bus,
        &ctx,
        shutdown_rx,
    )
    .await
    .expect("wire_provisioning_hook should succeed");

    // Publish a raw JSON payload (matching what the provisioning outbox relay sends)
    let tenant_id = uuid::Uuid::new_v4();
    let payload = serde_json::json!({
        "tenant_id": tenant_id.to_string(),
        "activated_at": "2026-04-01T12:00:00Z",
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    bus.publish("tenant.provisioned", bytes).await.unwrap();

    // Wait for processing
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "provisioning hook should be called once"
    );
    assert_eq!(
        *received_id.lock().await,
        Some(tenant_id),
        "hook should receive the correct tenant_id"
    );

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}

// ──────────────────────────────────────────────────────────────────
// Test 7: Provisioning hook retries on failure
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn provisioning_hook_retries() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let attempt_count = Arc::new(AtomicUsize::new(0));
    let count_clone = attempt_count.clone();

    let handler: platform_sdk::consumer::ProvisioningHandler = Arc::new(move |_ctx, _event| {
        let count = count_clone.clone();
        Box::pin(async move {
            let attempt = count.fetch_add(1, Ordering::SeqCst) + 1;
            if attempt < 3 {
                Err(ConsumerError::Processing(format!(
                    "transient error attempt {attempt}"
                )))
            } else {
                Ok(())
            }
        })
    });

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let handle = platform_sdk::consumer::wire_provisioning_hook(
        handler,
        &bus,
        &ctx,
        shutdown_rx,
    )
    .await
    .expect("wire_provisioning_hook should succeed");

    let payload = serde_json::json!({
        "tenant_id": uuid::Uuid::new_v4().to_string(),
    });
    bus.publish("tenant.provisioned", serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;

    assert_eq!(
        attempt_count.load(Ordering::SeqCst),
        3,
        "hook should retry 3 times (2 failures + 1 success)"
    );

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}

// ──────────────────────────────────────────────────────────────────
// Test 8: Provisioning hook skips malformed payload
// ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn provisioning_hook_skips_bad_payload() {
    let pool = match test_pool().await {
        Some(p) => p,
        None => return,
    };

    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let manifest = test_manifest();
    let ctx = ModuleContext::new(pool, manifest, Some(bus.clone()));

    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();

    let handler: platform_sdk::consumer::ProvisioningHandler = Arc::new(move |_ctx, _event| {
        let count = count_clone.clone();
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    });

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let handle = platform_sdk::consumer::wire_provisioning_hook(
        handler,
        &bus,
        &ctx,
        shutdown_rx,
    )
    .await
    .expect("wire_provisioning_hook should succeed");

    // Publish malformed payload (missing tenant_id)
    bus.publish(
        "tenant.provisioned",
        serde_json::to_vec(&serde_json::json!({"bad": "data"})).unwrap(),
    )
    .await
    .unwrap();

    // Then publish a valid one
    let payload = serde_json::json!({
        "tenant_id": uuid::Uuid::new_v4().to_string(),
    });
    bus.publish("tenant.provisioned", serde_json::to_vec(&payload).unwrap())
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    assert_eq!(
        call_count.load(Ordering::SeqCst),
        1,
        "handler should be called once (bad message skipped, good message processed)"
    );

    let _ = shutdown_tx.send(true);
    let _ = handle.await;
}
