//! End-to-end tests for JetStream dedup window configuration.
//!
//! Verifies that:
//! 1. A message published with a given `Nats-Msg-Id` within the dedup window
//!    is flagged as a duplicate by the server (`ack.duplicate == true`).
//! 2. After the window expires, the same `Nats-Msg-Id` is treated as a new
//!    message (`ack.duplicate == false`), documenting the expected "at-least-once
//!    after window expiry" behaviour.
//! 3. `ensure_platform_streams` creates all platform streams with the correct
//!    dedup windows (verified by reading the stream info back from NATS).
//!
//! Requires: NATS with JetStream enabled on `NATS_URL` (default: localhost:4222).
//! Run with: `cargo test -p event-bus -- dedup_window --nocapture`

use std::time::Duration;

use async_nats::jetstream::{self, context::Publish, stream};
use event_bus::{all_stream_definitions, ensure_platform_streams, StreamClass};
use uuid::Uuid;

/// Connect to NATS using the test URL.
async fn nats_client() -> async_nats::Client {
    let url = std::env::var("NATS_URL")
        .unwrap_or_else(|_| "nats://platform:dev-nats-token@localhost:4222".to_string());
    event_bus::connect_nats(&url)
        .await
        .expect("NATS must be running — start with: docker compose up -d nats")
}

/// Create a short-lived test stream with the given dedup window.
/// Returns the stream name.
async fn create_test_stream(js: &jetstream::Context, tag: &str, dedup_window: Duration) -> String {
    let name = format!("TEST_DEDUP_{}", tag.to_uppercase());
    let subject = format!("test.dedup.{}.>", tag);

    // Clean up from any prior run.
    let _ = js.delete_stream(&name).await;

    js.create_stream(stream::Config {
        name: name.clone(),
        subjects: vec![subject],
        duplicate_window: dedup_window,
        max_age: Duration::from_secs(300), // 5 min — test data
        ..Default::default()
    })
    .await
    .expect("create test stream");

    name
}

// ── Test 1: duplicate within window ──────────────────────────────────────────

/// Publishes the same `Nats-Msg-Id` twice within the dedup window.
/// First publish → not a duplicate. Second publish → duplicate.
#[tokio::test]
async fn duplicate_within_window_is_flagged() {
    let nats = nats_client().await;
    let js = jetstream::new(nats);

    let tag = Uuid::new_v4().simple().to_string();
    let dedup_window = Duration::from_secs(3);
    let stream_name = create_test_stream(&js, &tag, dedup_window).await;
    let subject = format!("test.dedup.{}.events", tag);

    let msg_id = Uuid::new_v4().to_string();
    let payload = b"financial-event-payload".to_vec();

    // First publish — must NOT be flagged as duplicate.
    let ack1 = js
        .send_publish(
            subject.clone(),
            Publish::build()
                .payload(payload.clone().into())
                .message_id(&msg_id),
        )
        .await
        .expect("publish 1")
        .await
        .expect("ack 1");

    assert!(
        !ack1.duplicate,
        "first publish must not be a duplicate (msg_id={})",
        msg_id
    );

    // Second publish within window — MUST be flagged as duplicate.
    let ack2 = js
        .send_publish(
            subject.clone(),
            Publish::build().payload(payload.into()).message_id(&msg_id),
        )
        .await
        .expect("publish 2")
        .await
        .expect("ack 2");

    assert!(
        ack2.duplicate,
        "second publish within dedup window must be a duplicate (msg_id={})",
        msg_id
    );

    // Cleanup.
    let _ = js.delete_stream(&stream_name).await;
}

// ── Test 2: after window expiry, same ID is new ───────────────────────────────

/// After the dedup window expires, the same `Nats-Msg-Id` is no longer
/// remembered by JetStream. A third publish is treated as a new message.
///
/// This documents the expected "at-least-once after window expiry" behaviour:
/// if a consumer lags longer than the dedup window AND the publisher retries,
/// the message will be processed again. Choose window lengths accordingly.
#[tokio::test]
async fn after_window_expiry_same_id_is_processed_again() {
    let nats = nats_client().await;
    let js = jetstream::new(nats);

    let tag = Uuid::new_v4().simple().to_string();
    let dedup_window = Duration::from_secs(2); // Short for test speed.
    let stream_name = create_test_stream(&js, &tag, dedup_window).await;
    let subject = format!("test.dedup.{}.events", tag);

    let msg_id = Uuid::new_v4().to_string();
    let payload = b"payload-after-window".to_vec();

    // Publish within window.
    let ack1 = js
        .send_publish(
            subject.clone(),
            Publish::build()
                .payload(payload.clone().into())
                .message_id(&msg_id),
        )
        .await
        .expect("publish 1")
        .await
        .expect("ack 1");
    assert!(!ack1.duplicate, "first publish must not be a duplicate");

    // Confirm second publish within window is duplicate.
    let ack2 = js
        .send_publish(
            subject.clone(),
            Publish::build()
                .payload(payload.clone().into())
                .message_id(&msg_id),
        )
        .await
        .expect("publish 2 (in-window)")
        .await
        .expect("ack 2");
    assert!(ack2.duplicate, "in-window publish must be a duplicate");

    // Wait for the dedup window to expire (2s window + 1s margin).
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Publish AFTER window — JetStream no longer remembers the msg_id,
    // so this must NOT be flagged as a duplicate.
    let ack3 = js
        .send_publish(
            subject.clone(),
            Publish::build().payload(payload.into()).message_id(&msg_id),
        )
        .await
        .expect("publish 3 (post-window)")
        .await
        .expect("ack 3");

    assert!(
        !ack3.duplicate,
        "publish after dedup window must NOT be flagged as duplicate (documents \
         at-least-once behaviour when consumer lags beyond window) msg_id={}",
        msg_id
    );

    // Cleanup.
    let _ = js.delete_stream(&stream_name).await;
}

// ── Test 3: platform streams have correct dedup windows ──────────────────────

/// Calls `ensure_platform_streams` and then reads back the stream configs from
/// NATS to verify the dedup windows were applied correctly.
#[tokio::test]
async fn platform_streams_have_configured_dedup_windows() {
    let nats = nats_client().await;

    ensure_platform_streams(nats.clone())
        .await
        .expect("ensure_platform_streams must succeed");

    let js = jetstream::new(nats);

    for def in all_stream_definitions() {
        let mut stream = js
            .get_stream(def.name)
            .await
            .unwrap_or_else(|e| panic!("stream '{}' must exist: {}", def.name, e));
        let info = stream
            .info()
            .await
            .unwrap_or_else(|e| panic!("stream '{}' info failed: {}", def.name, e));

        let configured_window = info.config.duplicate_window;
        let expected_window = def.class.dedup_window();

        assert_eq!(
            configured_window,
            expected_window,
            "stream '{}' (class={}) has dedup_window={:?}, expected {:?}",
            def.name,
            def.class.label(),
            configured_window,
            expected_window,
        );

        println!(
            "stream='{}' class='{}' dedup_window={}s OK",
            def.name,
            def.class.label(),
            configured_window.as_secs(),
        );
    }
}

// ── Test 4: stream class windows are correct (pure unit tests) ────────────────

#[test]
fn stream_class_dedup_windows_are_correct() {
    assert_eq!(
        StreamClass::Financial.dedup_window(),
        Duration::from_secs(86_400),
        "financial dedup window must be 24h"
    );
    assert_eq!(
        StreamClass::Operational.dedup_window(),
        Duration::from_secs(3_600),
        "operational dedup window must be 1h"
    );
    assert_eq!(
        StreamClass::Notification.dedup_window(),
        Duration::from_secs(3_600),
        "notification dedup window must be 1h"
    );
    assert_eq!(
        StreamClass::System.dedup_window(),
        Duration::from_secs(86_400),
        "system dedup window must be 24h"
    );
}

// ── Test 5: all_stream_definitions covers expected subjects ───────────────────

#[test]
fn financial_stream_covers_ap_ar_gl_payments() {
    let defs = all_stream_definitions();
    let financial = defs
        .iter()
        .find(|d| d.name == "FINANCIAL_EVENTS")
        .expect("FINANCIAL_EVENTS must be defined");

    for required in &["ap.>", "ar.>", "gl.>", "payments.>"] {
        assert!(
            financial.subjects.iter().any(|s| s == required),
            "FINANCIAL_EVENTS must cover subject '{}'; got: {:?}",
            required,
            financial.subjects
        );
    }
}

#[test]
fn operational_stream_covers_production_inventory_shipping() {
    let defs = all_stream_definitions();
    let operational = defs
        .iter()
        .find(|d| d.name == "OPERATIONAL_EVENTS")
        .expect("OPERATIONAL_EVENTS must be defined");

    for required in &["production.>", "inventory.>", "shipping.>"] {
        assert!(
            operational.subjects.iter().any(|s| s == required),
            "OPERATIONAL_EVENTS must cover subject '{}'; got: {:?}",
            required,
            operational.subjects
        );
    }
}
