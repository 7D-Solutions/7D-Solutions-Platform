//! Integration tests for form submission validation (bd-3q6j).
//!
//! Covers: missing required fields, invalid number range, invalid dropdown,
//! invalid date format, checkbox type enforcement.

mod submission_helpers;

use pdf_editor::domain::submissions::{
    CreateSubmissionRequest, SubmissionError, SubmissionRepo,
};
use serial_test::serial;
use submission_helpers::{create_test_template_with_fields, setup_db, unique_tenant};

#[tokio::test]
#[serial]
async fn test_submit_rejects_missing_required_fields() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(serde_json::json!({})),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid)
        .await
        .unwrap_err();
    match err {
        SubmissionError::Validation(msg) => {
            assert!(msg.contains("'company_name' is required"));
            assert!(msg.contains("'mileage' is required"));
            assert!(msg.contains("'inspection_date' is required"));
            assert!(msg.contains("'vehicle_type' is required"));
            assert!(msg.contains("'passed' is required"));
        }
        other => panic!("Expected Validation error, got: {:?}", other),
    }

    // Verify submission remains draft
    let fetched = SubmissionRepo::find_by_id(&pool, sub.id, &tid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.status, "draft");
}

#[tokio::test]
#[serial]
async fn test_submit_rejects_invalid_number_range() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(serde_json::json!({
                "company_name": "Acme",
                "mileage": -5,
                "inspection_date": "2026-02-24",
                "vehicle_type": "truck",
                "passed": true,
            })),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid)
        .await
        .unwrap_err();
    match err {
        SubmissionError::Validation(msg) => {
            assert!(msg.contains("'mileage' must be >= 0"));
        }
        other => panic!("Expected Validation error, got: {:?}", other),
    }
}

#[tokio::test]
#[serial]
async fn test_submit_rejects_invalid_dropdown_value() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(serde_json::json!({
                "company_name": "Acme",
                "mileage": 10000,
                "inspection_date": "2026-02-24",
                "vehicle_type": "bicycle",
                "passed": true,
            })),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid)
        .await
        .unwrap_err();
    match err {
        SubmissionError::Validation(msg) => {
            assert!(msg.contains("'vehicle_type' must be one of: truck, van, car"));
        }
        other => panic!("Expected Validation error, got: {:?}", other),
    }
}

#[tokio::test]
#[serial]
async fn test_submit_rejects_invalid_date() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(serde_json::json!({
                "company_name": "Acme",
                "mileage": 10000,
                "inspection_date": "not-a-date",
                "vehicle_type": "truck",
                "passed": true,
            })),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid)
        .await
        .unwrap_err();
    match err {
        SubmissionError::Validation(msg) => {
            assert!(msg.contains("'inspection_date' must be a valid date"));
        }
        other => panic!("Expected Validation error, got: {:?}", other),
    }
}

#[tokio::test]
#[serial]
async fn test_checkbox_must_be_boolean() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(serde_json::json!({
                "company_name": "Acme",
                "mileage": 10000,
                "inspection_date": "2026-02-24",
                "vehicle_type": "truck",
                "passed": "yes",
            })),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid)
        .await
        .unwrap_err();
    match err {
        SubmissionError::Validation(msg) => {
            assert!(msg.contains("'passed' must be a boolean"));
        }
        other => panic!("Expected Validation error, got: {:?}", other),
    }
}

#[tokio::test]
#[serial]
async fn test_submit_rejects_number_over_max() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(serde_json::json!({
                "company_name": "Acme",
                "mileage": 1_500_000,
                "inspection_date": "2026-02-24",
                "vehicle_type": "truck",
                "passed": true,
            })),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid)
        .await
        .unwrap_err();
    match err {
        SubmissionError::Validation(msg) => {
            assert!(msg.contains("'mileage' must be <= 999999"));
        }
        other => panic!("Expected Validation error, got: {:?}", other),
    }
}
