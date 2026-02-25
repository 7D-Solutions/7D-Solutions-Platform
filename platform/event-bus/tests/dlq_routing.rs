//! Integration tests for DLQ routing: failed handlers exhaust retries then route to DLQ.

use event_bus::consumer_retry::{retry_with_backoff, RetryConfig};
use event_bus::{EventBus, InMemoryBus};
use futures::StreamExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Simulates a handler that always fails, causing the event to be routed to DLQ.
#[tokio::test]
async fn handler_failure_exhausts_retries_then_routes_to_dlq() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("orders.>").await.unwrap();
    let mut dlq_stream = bus.subscribe("dlq.orders.>").await.unwrap();

    // Publish an event
    bus.publish("orders.created", b"bad-event".to_vec())
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap();

    // Attempt to handle with a permanently-failing handler
    let config = RetryConfig {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(50),
    };

    let attempt_count = Arc::new(Mutex::new(0u32));
    let attempt_clone = attempt_count.clone();

    let result = retry_with_backoff(
        || {
            let attempts = attempt_clone.clone();
            async move {
                *attempts.lock().unwrap() += 1;
                Err::<(), String>("handler crashed".to_string())
            }
        },
        &config,
        "process_order",
    )
    .await;

    assert!(result.is_err());
    assert_eq!(*attempt_count.lock().unwrap(), 3);

    // On final failure, route to DLQ
    let dlq_subject = format!("dlq.{}", msg.subject);
    bus.publish(&dlq_subject, msg.payload.clone())
        .await
        .unwrap();

    // Verify DLQ consumer receives it
    let dlq_msg = tokio::time::timeout(Duration::from_secs(1), dlq_stream.next())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(dlq_msg.subject, "dlq.orders.created");
    assert_eq!(dlq_msg.payload, b"bad-event");
}

/// Handler succeeds on second attempt — no DLQ routing.
#[tokio::test]
async fn handler_recovers_on_retry_no_dlq() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("orders.>").await.unwrap();
    let mut dlq_stream = bus.subscribe("dlq.orders.>").await.unwrap();

    bus.publish("orders.created", b"retry-event".to_vec())
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap()
        .unwrap();

    let config = RetryConfig {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(10),
        max_backoff: Duration::from_millis(50),
    };

    let attempt_count = Arc::new(Mutex::new(0u32));
    let attempt_clone = attempt_count.clone();

    let result = retry_with_backoff(
        || {
            let attempts = attempt_clone.clone();
            async move {
                let mut count = attempts.lock().unwrap();
                *count += 1;
                if *count < 2 {
                    Err("transient failure".to_string())
                } else {
                    Ok(())
                }
            }
        },
        &config,
        "process_order",
    )
    .await;

    assert!(result.is_ok());
    assert_eq!(*attempt_count.lock().unwrap(), 2);

    // Should NOT route to DLQ since handler succeeded
    // Verify nothing on DLQ
    let _ = msg; // consumed
    let timeout = tokio::time::timeout(Duration::from_millis(100), dlq_stream.next()).await;
    assert!(timeout.is_err(), "DLQ should be empty when handler succeeds");
}

/// Multiple events: some fail permanently (→ DLQ), some succeed.
#[tokio::test]
async fn mixed_success_and_failure_routes_only_failures_to_dlq() {
    let bus = InMemoryBus::new();
    let mut stream = bus.subscribe("jobs.>").await.unwrap();
    let mut dlq_stream = bus.subscribe("dlq.jobs.>").await.unwrap();

    // Publish 4 jobs: jobs 0 and 2 will fail, 1 and 3 will succeed
    for i in 0..4 {
        let payload = format!("job-{}", i).into_bytes();
        bus.publish("jobs.process", payload).await.unwrap();
    }

    let config = RetryConfig {
        max_attempts: 2,
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
    };

    let mut dlq_count = 0;
    let mut success_count = 0;

    for i in 0..4 {
        let msg = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap();

        let should_fail = i % 2 == 0;

        let result = retry_with_backoff(
            || {
                let fail = should_fail;
                async move {
                    if fail {
                        Err::<(), String>("permanent failure".into())
                    } else {
                        Ok(())
                    }
                }
            },
            &config,
            "process_job",
        )
        .await;

        if result.is_err() {
            // Route to DLQ
            let dlq_subject = format!("dlq.{}", msg.subject);
            bus.publish(&dlq_subject, msg.payload).await.unwrap();
            dlq_count += 1;
        } else {
            success_count += 1;
        }
    }

    assert_eq!(success_count, 2);
    assert_eq!(dlq_count, 2);

    // Drain DLQ and verify exactly 2 messages
    for _ in 0..2 {
        let dlq_msg = tokio::time::timeout(Duration::from_secs(1), dlq_stream.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dlq_msg.subject, "dlq.jobs.process");
    }

    // No more on DLQ
    let timeout = tokio::time::timeout(Duration::from_millis(100), dlq_stream.next()).await;
    assert!(timeout.is_err(), "DLQ should have exactly 2 messages");
}

/// Verify retry backoff actually executes multiple attempts before DLQ.
#[tokio::test]
async fn retry_exhaustion_takes_expected_attempts() {
    let config = RetryConfig {
        max_attempts: 5,
        initial_backoff: Duration::from_millis(5),
        max_backoff: Duration::from_millis(20),
    };

    let attempts = Arc::new(Mutex::new(Vec::new()));
    let attempts_clone = attempts.clone();

    let result = retry_with_backoff(
        || {
            let log = attempts_clone.clone();
            async move {
                let mut v = log.lock().unwrap();
                v.push(std::time::Instant::now());
                Err::<(), String>("always fails".into())
            }
        },
        &config,
        "exhaustion_test",
    )
    .await;

    assert!(result.is_err());

    let timestamps = attempts.lock().unwrap();
    assert_eq!(timestamps.len(), 5, "should attempt exactly max_attempts times");

    // Verify there are delays between attempts (backoff is working)
    for i in 1..timestamps.len() {
        let gap = timestamps[i].duration_since(timestamps[i - 1]);
        assert!(
            gap >= Duration::from_millis(3),
            "gap between attempt {} and {} should show backoff: {:?}",
            i - 1,
            i,
            gap
        );
    }
}
