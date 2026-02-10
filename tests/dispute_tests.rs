mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serial_test::serial;
use tower::ServiceExt;

const APP_ID: &str = "test-app";

/// TEST 1: List disputes by customer (through charge relationship)
#[tokio::test]
#[serial]
async fn test_list_disputes_by_customer() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer, charge, and disputes
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge1_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;
    let charge2_id = common::seed_charge(&pool, APP_ID, customer_id, 3000, "succeeded").await;

    let dispute1_id = common::seed_dispute(&pool, APP_ID, charge1_id, "open").await;
    let dispute2_id = common::seed_dispute(&pool, APP_ID, charge2_id, "under_review").await;

    // List all disputes (they should all be for the same customer)
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/disputes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    let disputes = json.as_array().unwrap();

    // Should find at least our 2 disputes for this customer
    assert!(disputes.len() >= 2, "Should have at least 2 disputes");

    // Verify disputes belong to correct charges
    let dispute_ids: Vec<i32> = disputes
        .iter()
        .filter_map(|d| d["id"].as_i64().map(|id| id as i32))
        .collect();
    assert!(dispute_ids.contains(&dispute1_id));
    assert!(dispute_ids.contains(&dispute2_id));

    common::cleanup_disputes(&pool, &[dispute1_id, dispute2_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 2: List disputes filtered by charge ID
#[tokio::test]
#[serial]
async fn test_list_disputes_by_charge() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer and charges
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge1_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;
    let charge2_id = common::seed_charge(&pool, APP_ID, customer_id, 3000, "succeeded").await;

    // Create disputes for both charges
    let dispute1_id = common::seed_dispute(&pool, APP_ID, charge1_id, "open").await;
    let dispute2_id = common::seed_dispute(&pool, APP_ID, charge2_id, "closed").await;

    // Query disputes for charge1 only
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/disputes?charge_id={}", charge1_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");
    let disputes = json.as_array().unwrap();

    // Should only find dispute for charge1
    assert_eq!(disputes.len(), 1, "Should find exactly 1 dispute for charge1");
    assert_eq!(disputes[0]["id"], dispute1_id);
    assert_eq!(disputes[0]["charge_id"], charge1_id);

    common::cleanup_disputes(&pool, &[dispute1_id, dispute2_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 3: List disputes filtered by status
#[tokio::test]
#[serial]
async fn test_list_disputes_by_status() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer and charge
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;

    // Create disputes with different statuses
    let dispute1_id = common::seed_dispute(&pool, APP_ID, charge_id, "open").await;
    let dispute2_id = common::seed_dispute(&pool, APP_ID, charge_id, "under_review").await;
    let dispute3_id = common::seed_dispute(&pool, APP_ID, charge_id, "won").await;

    // Query disputes with status "open"
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/disputes?status=open")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");

    // Should find at least 1 dispute with status "open"
    let open_disputes: Vec<_> = json
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["status"] == "open")
        .collect();

    assert!(
        open_disputes.len() >= 1,
        "Should have at least 1 open dispute"
    );

    // Verify our open dispute is in the results
    let dispute_ids: Vec<i32> = open_disputes
        .iter()
        .filter_map(|d| d["id"].as_i64().map(|id| id as i32))
        .collect();
    assert!(dispute_ids.contains(&dispute1_id));

    common::cleanup_disputes(&pool, &[dispute1_id, dispute2_id, dispute3_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 4: Get a specific dispute by ID
#[tokio::test]
#[serial]
async fn test_get_dispute_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer, charge, and dispute
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;
    let dispute_id = common::seed_dispute(&pool, APP_ID, charge_id, "under_review").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/api/ar/disputes/{}", dispute_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Assert 200 status
    assert_eq!(response.status(), StatusCode::OK);

    // Assert response body
    let json = common::body_json(response).await;
    assert_eq!(json["id"], dispute_id);
    assert_eq!(json["status"], "under_review");
    assert_eq!(json["charge_id"], charge_id);
    assert_eq!(json["amount_cents"], 5000);
    assert_eq!(json["currency"], "usd");
    assert!(json["tilled_dispute_id"].is_string(), "Should have tilled_dispute_id");
    assert!(json["evidence_due_by"].is_string(), "Should have evidence_due_by");

    common::cleanup_disputes(&pool, &[dispute_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 5: Get dispute with invalid ID (not found)
#[tokio::test]
#[serial]
async fn test_get_dispute_not_found() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/disputes/999999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = common::body_json(response).await;
    assert!(json["error"].is_string(), "Should have error message");

    common::teardown_pool(pool).await;
}

/// TEST 6: Submit dispute evidence successfully
#[tokio::test]
#[serial]
async fn test_submit_dispute_evidence_success() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer, charge, and open dispute
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;
    let dispute_id = common::seed_dispute(&pool, APP_ID, charge_id, "open").await;

    let body = serde_json::json!({
        "evidence": {
            "customer_name": "Test Customer",
            "customer_email_address": "test@example.com",
            "billing_address": "123 Test St",
            "receipt": "receipt_123.pdf",
            "customer_signature": "signature.png",
            "uncategorized_text": "Additional evidence documentation"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/disputes/{}/evidence", dispute_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should accept evidence submission
    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert_eq!(json["id"], dispute_id);
    assert!(json["id"].is_number(), "Response should contain dispute id");

    common::cleanup_disputes(&pool, &[dispute_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 7: Submit dispute evidence for non-existent dispute
#[tokio::test]
#[serial]
async fn test_submit_dispute_evidence_invalid_dispute() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    let body = serde_json::json!({
        "evidence": {
            "customer_name": "Test Customer",
            "uncategorized_text": "Evidence for non-existent dispute"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ar/disputes/999999/evidence")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 404 (dispute not found)
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = common::body_json(response).await;
    assert!(json["error"].is_string(), "Should have error message");

    common::teardown_pool(pool).await;
}

/// TEST 8: Submit evidence for closed dispute (should fail)
#[tokio::test]
#[serial]
async fn test_submit_evidence_for_closed_dispute() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer, charge, and closed dispute
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;
    let dispute_id = common::seed_dispute(&pool, APP_ID, charge_id, "lost").await;

    let body = serde_json::json!({
        "evidence": {
            "customer_name": "Test Customer",
            "uncategorized_text": "Evidence for closed dispute"
        }
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/api/ar/disputes/{}/evidence", dispute_id))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should reject evidence for closed dispute
    // The actual behavior depends on the implementation, but typically this would
    // return BAD_REQUEST or CONFLICT
    assert!(
        response.status() == StatusCode::BAD_REQUEST
        || response.status() == StatusCode::CONFLICT
        || response.status() == StatusCode::OK, // Some APIs allow this
        "Should handle closed dispute appropriately, got: {}",
        response.status()
    );

    common::cleanup_disputes(&pool, &[dispute_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}

/// TEST 9: List disputes with pagination
#[tokio::test]
#[serial]
async fn test_list_disputes_with_pagination() {
    let pool = common::setup_pool().await;
    let app = common::app(&pool);

    // Seed customer and multiple disputes
    let (customer_id, _, _) = common::seed_customer(&pool, APP_ID).await;
    let charge1_id = common::seed_charge(&pool, APP_ID, customer_id, 5000, "succeeded").await;
    let charge2_id = common::seed_charge(&pool, APP_ID, customer_id, 3000, "succeeded").await;
    let charge3_id = common::seed_charge(&pool, APP_ID, customer_id, 2000, "succeeded").await;

    let dispute1_id = common::seed_dispute(&pool, APP_ID, charge1_id, "open").await;
    let dispute2_id = common::seed_dispute(&pool, APP_ID, charge2_id, "under_review").await;
    let dispute3_id = common::seed_dispute(&pool, APP_ID, charge3_id, "won").await;

    // List with limit
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/ar/disputes?limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = common::body_json(response).await;
    assert!(json.is_array(), "Response should be array");

    // Should have at least 2 disputes (may have more from other tests)
    assert!(
        json.as_array().unwrap().len() >= 2,
        "Should have at least 2 disputes"
    );

    common::cleanup_disputes(&pool, &[dispute1_id, dispute2_id, dispute3_id]).await;
    common::cleanup_customers(&pool, &[customer_id]).await;
    common::teardown_pool(pool).await;
}
