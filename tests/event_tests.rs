mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

/// Clean up all test events before running tests
async fn cleanup_all_test_events(pool: &sqlx::PgPool) {
    sqlx::query("DELETE FROM ar_events WHERE app_id = $1")
        .bind(APP_ID)
        .execute(pool)
        .await
        .ok();
}

/// TEST 1: List events filtered by customer entity
#[tokio::test]
#[serial]
async fn test_list_events_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_events(&pool).await;

    // Seed customer for events
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed events for the customer
    let event1_id = common::seed_event(
        &pool,
        APP_ID,
        "customer.created",
        "api",
        Some("customer"),
        Some(&customer_id.to_string()),
    )
    .await;

    let event2_id = common::seed_event(
        &pool,
        APP_ID,
        "customer.updated",
        "api",
        Some("customer"),
        Some(&customer_id.to_string()),
    )
    .await;

    // Seed event for different customer (should not appear)
    let _other_event_id = common::seed_event(
        &pool,
        APP_ID,
        "customer.created",
        "api",
        Some("customer"),
        Some("99999"),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!(
                    "/api/ar/events?entity_type=customer&entity_id={}",
                    customer_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(
        json.as_array().unwrap().len(),
        2,
        "Should have 2 events for this customer"
    );

    // Verify events belong to correct customer
    for event in json.as_array().unwrap() {
        assert_eq!(event["entity_id"], customer_id.to_string());
    }

    common::cleanup_events(&pool, &[event1_id, event2_id, _other_event_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: List events filtered by event type
#[tokio::test]
#[serial]
async fn test_list_events_by_type() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_events(&pool).await;

    // Seed customer for realistic events
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed different event types
    let event1_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.succeeded",
        "webhook",
        Some("payment"),
        Some("pay_123"),
    )
    .await;

    let event2_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.succeeded",
        "webhook",
        Some("payment"),
        Some("pay_456"),
    )
    .await;

    let event3_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.failed",
        "webhook",
        Some("payment"),
        Some("pay_789"),
    )
    .await;

    let event4_id = common::seed_event(
        &pool,
        APP_ID,
        "subscription.canceled",
        "api",
        Some("subscription"),
        Some("sub_123"),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/events?event_type=payment.succeeded")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(
        json.as_array().unwrap().len(),
        2,
        "Should have 2 payment.succeeded events"
    );

    // Verify all returned events have correct event_type
    for event in json.as_array().unwrap() {
        assert_eq!(event["event_type"], "payment.succeeded");
    }

    common::cleanup_events(&pool, &[event1_id, event2_id, event3_id, event4_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 3: List events with pagination
#[tokio::test]
#[serial]
async fn test_list_events_pagination() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_events(&pool).await;

    // Seed customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed 10 events
    let mut event_ids = Vec::new();
    for i in 0..10 {
        let event_id = common::seed_event(
            &pool,
            APP_ID,
            &format!("test.event.{}", i),
            "api",
            Some("test"),
            Some(&format!("test_{}", i)),
        )
        .await;
        event_ids.push(event_id);
    }

    // Test with limit parameter
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/events?limit=5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(
        json.as_array().unwrap().len(),
        5,
        "Should return only 5 events"
    );

    common::cleanup_events(&pool, &event_ids).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: Get single event by ID - success case
#[tokio::test]
#[serial]
async fn test_get_event_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed a specific event
    let event_id = common::seed_event(
        &pool,
        APP_ID,
        "subscription.created",
        "api",
        Some("subscription"),
        Some("sub_test123"),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/events/{}", event_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], event_id);
    assert_eq!(json["event_type"], "subscription.created");
    assert_eq!(json["source"], "api");
    assert_eq!(json["entity_type"], "subscription");
    assert_eq!(json["entity_id"], "sub_test123");
    assert!(json["payload"].is_object(), "Payload should be object");
    assert!(json["created_at"].is_string(), "Should have created_at");

    common::cleanup_events(&pool, &[event_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: Get event by ID - not found case
#[tokio::test]
#[serial]
async fn test_get_event_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Try to get non-existent event
    let non_existent_id = 999999;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/events/{}", non_existent_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = common::body_json(response).await;
    assert!(
        json["error"].is_string() || json["message"].is_string(),
        "Should have error message"
    );

    common::teardown_pool(pool).await;
}

/// TEST 6: List events filtered by source
#[tokio::test]
#[serial]
async fn test_list_events_by_source() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_events(&pool).await;

    // Seed customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed events from different sources
    let event1_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.succeeded",
        "webhook",
        Some("payment"),
        Some("pay_1"),
    )
    .await;

    let event2_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.refunded",
        "webhook",
        Some("payment"),
        Some("pay_2"),
    )
    .await;

    let event3_id = common::seed_event(
        &pool,
        APP_ID,
        "customer.updated",
        "api",
        Some("customer"),
        Some(&customer_id.to_string()),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/events?source=webhook")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(
        json.as_array().unwrap().len(),
        2,
        "Should have 2 webhook events"
    );

    // Verify all returned events have correct source
    for event in json.as_array().unwrap() {
        assert_eq!(event["source"], "webhook");
    }

    common::cleanup_events(&pool, &[event1_id, event2_id, event3_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: List events filtered by entity_type
#[tokio::test]
#[serial]
async fn test_list_events_by_entity_type() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_events(&pool).await;

    // Seed customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed events for different entity types
    let event1_id = common::seed_event(
        &pool,
        APP_ID,
        "subscription.created",
        "api",
        Some("subscription"),
        Some("sub_1"),
    )
    .await;

    let event2_id = common::seed_event(
        &pool,
        APP_ID,
        "subscription.canceled",
        "api",
        Some("subscription"),
        Some("sub_2"),
    )
    .await;

    let event3_id = common::seed_event(
        &pool,
        APP_ID,
        "customer.created",
        "api",
        Some("customer"),
        Some(&customer_id.to_string()),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/events?entity_type=subscription")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(
        json.as_array().unwrap().len(),
        2,
        "Should have 2 subscription events"
    );

    // Verify all returned events have correct entity_type
    for event in json.as_array().unwrap() {
        assert_eq!(event["entity_type"], "subscription");
    }

    common::cleanup_events(&pool, &[event1_id, event2_id, event3_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 8: List events with multiple filters (combined)
#[tokio::test]
#[serial]
async fn test_list_events_combined_filters() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Clean up any leftover test data
    cleanup_all_test_events(&pool).await;

    // Seed customer
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;

    // Seed various events
    let event1_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.succeeded",
        "webhook",
        Some("payment"),
        Some("pay_1"),
    )
    .await;

    let event2_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.succeeded",
        "api",
        Some("payment"),
        Some("pay_2"),
    )
    .await;

    let event3_id = common::seed_event(
        &pool,
        APP_ID,
        "payment.failed",
        "webhook",
        Some("payment"),
        Some("pay_3"),
    )
    .await;

    // Query with multiple filters: event_type AND source
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/events?event_type=payment.succeeded&source=webhook")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    assert_eq!(
        json.as_array().unwrap().len(),
        1,
        "Should have 1 event matching both filters"
    );

    // Verify the returned event matches both filters
    let event = &json[0];
    assert_eq!(event["event_type"], "payment.succeeded");
    assert_eq!(event["source"], "webhook");

    common::cleanup_events(&pool, &[event1_id, event2_id, event3_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
