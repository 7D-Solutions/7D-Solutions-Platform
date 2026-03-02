//! Integration tests for consumer receive, ack, and filtering behavior.

use event_bus::{EventBus, InMemoryBus};
use futures::StreamExt;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn consumer_receives_matching_events_only() {
    let bus = Arc::new(InMemoryBus::new());

    // Two consumers: one for auth, one for billing
    let mut auth_stream = bus.subscribe("auth.events.*").await.unwrap();
    let mut billing_stream = bus.subscribe("billing.events.*").await.unwrap();

    // Publish to both topics
    bus.publish("auth.events.login", b"login-payload".to_vec())
        .await
        .unwrap();
    bus.publish("billing.events.invoice", b"invoice-payload".to_vec())
        .await
        .unwrap();
    bus.publish("auth.events.logout", b"logout-payload".to_vec())
        .await
        .unwrap();

    // auth consumer gets 2 messages
    let m1 = tokio::time::timeout(Duration::from_secs(1), auth_stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m1.subject, "auth.events.login");

    let m2 = tokio::time::timeout(Duration::from_secs(1), auth_stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m2.subject, "auth.events.logout");

    // billing consumer gets 1 message
    let m3 = tokio::time::timeout(Duration::from_secs(1), billing_stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m3.subject, "billing.events.invoice");

    // No more for either
    let timeout_auth = tokio::time::timeout(Duration::from_millis(100), auth_stream.next()).await;
    assert!(timeout_auth.is_err(), "auth should have no more messages");

    let timeout_billing =
        tokio::time::timeout(Duration::from_millis(100), billing_stream.next()).await;
    assert!(
        timeout_billing.is_err(),
        "billing should have no more messages"
    );
}

#[tokio::test]
async fn consumer_receives_deep_wildcard_events() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("app.>").await.unwrap();

    bus.publish("app.module.sub.deep", b"deep".to_vec())
        .await
        .unwrap();
    bus.publish("app.x", b"shallow".to_vec()).await.unwrap();
    bus.publish("other.x", b"wrong-prefix".to_vec())
        .await
        .unwrap();

    let m1 = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m1.subject, "app.module.sub.deep");

    let m2 = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(m2.subject, "app.x");

    // "other.x" should not arrive
    let timeout = tokio::time::timeout(Duration::from_millis(100), stream.next()).await;
    assert!(timeout.is_err());
}

#[tokio::test]
async fn multiple_consumers_each_get_a_copy() {
    let bus = InMemoryBus::new();
    let mut s1 = bus.subscribe("shared.>").await.unwrap();
    let mut s2 = bus.subscribe("shared.>").await.unwrap();
    let mut s3 = bus.subscribe("shared.>").await.unwrap();

    bus.publish("shared.event", b"broadcast".to_vec())
        .await
        .unwrap();

    for stream in [&mut s1, &mut s2, &mut s3] {
        let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg.subject, "shared.event");
        assert_eq!(msg.payload, b"broadcast");
    }
}

#[tokio::test]
async fn consumer_processes_payload_correctly() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("orders.>").await.unwrap();

    let payload = serde_json::json!({
        "order_id": "o-42",
        "items": [{"sku": "A", "qty": 2}],
        "total": 9999
    });
    let bytes = serde_json::to_vec(&payload).unwrap();

    bus.publish("orders.created", bytes).await.unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap();

    let received: serde_json::Value = serde_json::from_slice(&msg.payload).unwrap();
    assert_eq!(received["order_id"], "o-42");
    assert_eq!(received["total"], 9999);
    assert_eq!(received["items"][0]["sku"], "A");
}
