//! Integration tests for form submission CRUD + events (bd-3q6j).
//!
//! Covers: create draft, autosave, submit + event, no double submit,
//! cannot autosave submitted, list with filters, tenant isolation.

mod submission_helpers;

use pdf_editor_rs::domain::submissions::{
    AutosaveRequest, CreateSubmissionRequest, ListSubmissionsQuery, SubmissionError,
    SubmissionRepo,
};
use serial_test::serial;
use submission_helpers::{
    create_test_template_with_fields, setup_db, unique_tenant, valid_field_data,
};

#[tokio::test]
#[serial]
async fn test_create_draft_submission() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "field-worker".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(sub.status, "draft");
    assert_eq!(sub.submitted_by, "field-worker");
    assert_eq!(sub.template_id, tmpl_id);
    assert_eq!(sub.field_data, serde_json::json!({}));
    assert!(sub.submitted_at.is_none());
}

#[tokio::test]
#[serial]
async fn test_autosave_field_data() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    // First autosave — partial data
    let saved = SubmissionRepo::autosave(
        &pool,
        sub.id,
        &tid,
        &AutosaveRequest {
            field_data: serde_json::json!({"company_name": "Acme"}),
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.field_data["company_name"], "Acme");

    // Second autosave — more data
    let saved2 = SubmissionRepo::autosave(
        &pool,
        sub.id,
        &tid,
        &AutosaveRequest {
            field_data: serde_json::json!({"company_name": "Acme Corp", "mileage": 42000}),
        },
    )
    .await
    .unwrap();
    assert_eq!(saved2.field_data["company_name"], "Acme Corp");
    assert_eq!(saved2.field_data["mileage"], 42000);
}

#[tokio::test]
#[serial]
async fn test_submit_with_valid_data() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(valid_field_data()),
        },
    )
    .await
    .unwrap();

    let submitted = SubmissionRepo::submit(&pool, sub.id, &tid).await.unwrap();
    assert_eq!(submitted.status, "submitted");
    assert!(submitted.submitted_at.is_some());

    // Verify event was enqueued in outbox
    let event: Option<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT event_type, payload
        FROM events_outbox
        WHERE tenant_id = $1 AND event_type = 'pdf.form.submitted'
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&tid)
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, payload) = event.expect("Event should be in outbox");
    assert_eq!(event_type, "pdf.form.submitted");
    let payload_data = payload["payload"].clone();
    assert_eq!(payload_data["submission_id"], sub.id.to_string());
    assert_eq!(payload_data["template_id"], tmpl_id.to_string());
    assert_eq!(payload_data["submitted_by"], "worker");
}

#[tokio::test]
#[serial]
async fn test_cannot_submit_twice() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(valid_field_data()),
        },
    )
    .await
    .unwrap();

    SubmissionRepo::submit(&pool, sub.id, &tid).await.unwrap();
    let err = SubmissionRepo::submit(&pool, sub.id, &tid).await.unwrap_err();
    assert!(matches!(err, SubmissionError::AlreadySubmitted));
}

#[tokio::test]
#[serial]
async fn test_cannot_autosave_submitted() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: Some(valid_field_data()),
        },
    )
    .await
    .unwrap();

    SubmissionRepo::submit(&pool, sub.id, &tid).await.unwrap();

    let err = SubmissionRepo::autosave(
        &pool,
        sub.id,
        &tid,
        &AutosaveRequest {
            field_data: serde_json::json!({"company_name": "Changed"}),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SubmissionError::AlreadySubmitted));
}

#[tokio::test]
#[serial]
async fn test_list_submissions_with_filters() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid).await;

    SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker-a".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    let sub2 = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid.clone(),
            template_id: tmpl_id,
            submitted_by: "worker-b".into(),
            field_data: Some(valid_field_data()),
        },
    )
    .await
    .unwrap();

    SubmissionRepo::submit(&pool, sub2.id, &tid).await.unwrap();

    // List all
    let all = SubmissionRepo::list(
        &pool,
        &ListSubmissionsQuery {
            tenant_id: tid.clone(),
            template_id: None,
            status: None,
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 2);

    // Filter by status
    let drafts = SubmissionRepo::list(
        &pool,
        &ListSubmissionsQuery {
            tenant_id: tid.clone(),
            template_id: None,
            status: Some("draft".into()),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].status, "draft");

    let submitted = SubmissionRepo::list(
        &pool,
        &ListSubmissionsQuery {
            tenant_id: tid.clone(),
            template_id: None,
            status: Some("submitted".into()),
            limit: None,
            offset: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0].status, "submitted");
}

#[tokio::test]
#[serial]
async fn test_tenant_isolation_submissions() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid_a.clone(),
            template_id: tmpl_id,
            submitted_by: "worker".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    // Tenant B cannot see, autosave, or submit tenant A's submission
    assert!(SubmissionRepo::find_by_id(&pool, sub.id, &tid_b)
        .await
        .unwrap()
        .is_none());

    let err = SubmissionRepo::autosave(
        &pool,
        sub.id,
        &tid_b,
        &AutosaveRequest {
            field_data: serde_json::json!({"sneaky": true}),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SubmissionError::NotFound));

    let err = SubmissionRepo::submit(&pool, sub.id, &tid_b)
        .await
        .unwrap_err();
    assert!(matches!(err, SubmissionError::NotFound));

    // Tenant B cannot create submission on tenant A's template
    let err = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid_b.clone(),
            template_id: tmpl_id,
            submitted_by: "sneaky".into(),
            field_data: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SubmissionError::TemplateNotFound));
}
