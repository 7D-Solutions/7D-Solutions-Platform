//! Tenant boundary tests for pdf-editor (bd-bxozo).
//!
//! Proves that tenant isolation is enforced at the repo layer for all
//! entity types: templates, fields, and submissions. Each test creates
//! data under tenant A and verifies tenant B cannot read, modify, or
//! list that data.

mod submission_helpers;

use pdf_editor::domain::forms::{
    CreateFieldRequest, CreateTemplateRequest, FieldRepo, FormError, ListTemplatesQuery,
    ReorderFieldsRequest, TemplateRepo, UpdateFieldRequest, UpdateTemplateRequest,
};
use pdf_editor::domain::submissions::{
    AutosaveRequest, CreateSubmissionRequest, ListSubmissionsQuery, SubmissionError, SubmissionRepo,
};
use serial_test::serial;
use submission_helpers::{
    create_test_template_with_fields, setup_db, unique_tenant, valid_field_data,
};

// ============================================================================
// Template isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_b_cannot_read_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = TemplateRepo::create(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tid_a.clone(),
            name: "Secret Template".into(),
            description: None,
            created_by: "admin-a".into(),
        },
    )
    .await
    .unwrap()
    .id;

    // find_by_id returns None for wrong tenant
    assert!(TemplateRepo::find_by_id(&pool, tmpl_id, &tid_b)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_list_tenant_a_templates() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    TemplateRepo::create(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tid_a.clone(),
            name: "Hidden".into(),
            description: None,
            created_by: "admin-a".into(),
        },
    )
    .await
    .unwrap();

    let (list, _) = TemplateRepo::list(
        &pool,
        &ListTemplatesQuery {
            tenant_id: tid_b,
            page: None,
            page_size: None,
        },
    )
    .await
    .unwrap();

    assert!(list.is_empty(), "Tenant B should see zero templates");
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_update_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = TemplateRepo::create(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tid_a.clone(),
            name: "Original".into(),
            description: None,
            created_by: "admin-a".into(),
        },
    )
    .await
    .unwrap()
    .id;

    let err = TemplateRepo::update(
        &pool,
        tmpl_id,
        &tid_b,
        &UpdateTemplateRequest {
            name: Some("Hijacked".into()),
            description: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, FormError::TemplateNotFound));

    // Verify original is unchanged
    let original = TemplateRepo::find_by_id(&pool, tmpl_id, &tid_a)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.name, "Original");
}

// ============================================================================
// Field isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_b_cannot_create_field_on_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let err = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid_b,
        &CreateFieldRequest {
            field_key: "injected".into(),
            field_label: "Injected Field".into(),
            field_type: "text".into(),
            validation_rules: None,
            pdf_position: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, FormError::TemplateNotFound));
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_list_fields_on_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let err = FieldRepo::list(&pool, tmpl_id, &tid_b).await.unwrap_err();
    assert!(matches!(err, FormError::TemplateNotFound));
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_update_field_on_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let fields = FieldRepo::list(&pool, tmpl_id, &tid_a).await.unwrap();
    let field_id = fields[0].id;

    let err = FieldRepo::update(
        &pool,
        field_id,
        tmpl_id,
        &tid_b,
        &UpdateFieldRequest {
            field_label: Some("Hijacked Label".into()),
            field_type: None,
            validation_rules: None,
            pdf_position: None,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, FormError::TemplateNotFound));

    // Verify original field unchanged
    let fields_after = FieldRepo::list(&pool, tmpl_id, &tid_a).await.unwrap();
    assert_ne!(fields_after[0].field_label, "Hijacked Label");
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_reorder_fields_on_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let fields = FieldRepo::list(&pool, tmpl_id, &tid_a).await.unwrap();
    let reversed_ids: Vec<_> = fields.iter().rev().map(|f| f.id).collect();

    let err = FieldRepo::reorder(
        &pool,
        tmpl_id,
        &tid_b,
        &ReorderFieldsRequest {
            field_ids: reversed_ids,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, FormError::TemplateNotFound));
}

// ============================================================================
// Submission isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn tenant_b_cannot_read_tenant_a_submission() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid_a.clone(),
            template_id: tmpl_id,
            submitted_by: "worker-a".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    assert!(SubmissionRepo::find_by_id(&pool, sub.id, &tid_b)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_autosave_tenant_a_submission() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid_a.clone(),
            template_id: tmpl_id,
            submitted_by: "worker-a".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::autosave(
        &pool,
        sub.id,
        &tid_b,
        &AutosaveRequest {
            field_data: serde_json::json!({"company_name": "Injected"}),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(err, SubmissionError::NotFound));
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_submit_tenant_a_submission() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    let sub = SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid_a.clone(),
            template_id: tmpl_id,
            submitted_by: "worker-a".into(),
            field_data: Some(valid_field_data()),
        },
    )
    .await
    .unwrap();

    let err = SubmissionRepo::submit(&pool, sub.id, &tid_b)
        .await
        .unwrap_err();

    assert!(matches!(err, SubmissionError::NotFound));
}

#[tokio::test]
#[serial]
async fn tenant_b_cannot_create_submission_on_tenant_a_template() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

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

#[tokio::test]
#[serial]
async fn tenant_b_cannot_list_tenant_a_submissions() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template_with_fields(&pool, &tid_a).await;

    SubmissionRepo::create(
        &pool,
        &CreateSubmissionRequest {
            tenant_id: tid_a.clone(),
            template_id: tmpl_id,
            submitted_by: "worker-a".into(),
            field_data: None,
        },
    )
    .await
    .unwrap();

    let (list, _) = SubmissionRepo::list(
        &pool,
        &ListSubmissionsQuery {
            tenant_id: tid_b,
            template_id: None,
            status: None,
            page: None,
            page_size: None,
        },
    )
    .await
    .unwrap();

    assert!(list.is_empty(), "Tenant B should see zero submissions");
}
