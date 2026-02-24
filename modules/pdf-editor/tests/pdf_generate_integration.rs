//! Integration tests for PDF generation from submissions (bd-1j5x).
//!
//! Covers: generate_filled_pdf with real DB data + real PDF,
//! event emission to outbox, status validation.

mod submission_helpers;

use pdf_editor_rs::domain::forms::{CreateFieldRequest, FieldRepo, TemplateRepo};
use pdf_editor_rs::domain::forms::CreateTemplateRequest;
use pdf_editor_rs::domain::generate::{generate_filled_pdf, validate_pdf, GenerateError};
use pdf_editor_rs::domain::submissions::{CreateSubmissionRequest, SubmissionRepo};
use pdf_editor_rs::event_bus::{create_pdf_editor_envelope, enqueue_event};
use serial_test::serial;
use submission_helpers::{setup_db, unique_tenant, valid_field_data};
use uuid::Uuid;

/// Load the test PDF fixture.
fn test_pdf_bytes() -> Vec<u8> {
    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/test.pdf"))
        .expect("test.pdf fixture must exist")
}

/// Create a template with fields that have pdf_position set.
async fn create_template_with_positions(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    let tmpl = TemplateRepo::create(
        pool,
        &CreateTemplateRequest {
            tenant_id: tid.to_string(),
            name: "Generate Test Form".into(),
            description: Some("Template for generate tests".into()),
            created_by: "test-user".into(),
        },
    )
    .await
    .unwrap();

    let fields = vec![
        ("company_name", "Company Name", "text", pos(100.0, 700.0, 1, 14.0)),
        ("mileage", "Current Mileage", "number", pos(100.0, 680.0, 1, 12.0)),
        ("inspection_date", "Inspection Date", "date", pos(100.0, 660.0, 1, 12.0)),
        ("vehicle_type", "Vehicle Type", "dropdown", pos(100.0, 640.0, 1, 12.0)),
        ("passed", "Passed Inspection", "checkbox", pos(100.0, 620.0, 1, 12.0)),
    ];

    for (key, label, ft, pdf_pos) in fields {
        FieldRepo::create(pool, tmpl.id, tid, &CreateFieldRequest {
            field_key: key.into(),
            field_label: label.into(),
            field_type: ft.into(),
            validation_rules: Some(serde_json::json!({"required": true})),
            pdf_position: Some(pdf_pos),
        })
        .await
        .unwrap();
    }

    tmpl.id
}

fn pos(x: f32, y: f32, page: u32, font_size: f32) -> serde_json::Value {
    serde_json::json!({ "x": x, "y": y, "page": page, "font_size": font_size })
}

#[tokio::test]
#[serial]
async fn test_generate_filled_pdf_roundtrip() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_template_with_positions(&pool, &tid).await;

    // Create and submit
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

    // Load fields from DB
    let fields = FieldRepo::list_by_template(&pool, tmpl_id).await.unwrap();
    let pdf_bytes = test_pdf_bytes();

    // Generate
    let result = generate_filled_pdf(&pdf_bytes, &fields, &valid_field_data());
    assert!(result.is_ok(), "generate_filled_pdf failed: {:?}", result.err());

    let output = result.unwrap();
    // Output should be a valid PDF (starts with %PDF-)
    assert!(output.starts_with(b"%PDF-"), "Output is not a PDF");
    // Output should be larger than input (text was added)
    assert!(
        output.len() > pdf_bytes.len(),
        "Output PDF ({} bytes) should be larger than input ({} bytes)",
        output.len(),
        pdf_bytes.len()
    );
}

#[tokio::test]
#[serial]
async fn test_generate_emits_event() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_template_with_positions(&pool, &tid).await;

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

    // Simulate the event emission the route handler does
    let payload = serde_json::json!({
        "tenant_id": tid,
        "submission_id": sub.id,
        "template_id": tmpl_id,
    });
    let envelope = create_pdf_editor_envelope(
        Uuid::new_v4(),
        tid.clone(),
        "pdf.form.generated".to_string(),
        None,
        None,
        "DATA_MUTATION".to_string(),
        payload,
    );

    let mut tx = pool.begin().await.unwrap();
    enqueue_event(&mut tx, "pdf.form.generated", &envelope)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Verify event in outbox
    let event: Option<(String, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT event_type, payload
        FROM events_outbox
        WHERE tenant_id = $1 AND event_type = 'pdf.form.generated'
        ORDER BY created_at DESC LIMIT 1
        "#,
    )
    .bind(&tid)
    .fetch_optional(&pool)
    .await
    .unwrap();

    let (event_type, payload) = event.expect("pdf.form.generated event should be in outbox");
    assert_eq!(event_type, "pdf.form.generated");
    let inner = &payload["payload"];
    assert_eq!(inner["submission_id"], sub.id.to_string());
    assert_eq!(inner["template_id"], tmpl_id.to_string());
    assert_eq!(inner["tenant_id"], tid);
}

#[tokio::test]
#[serial]
async fn test_generate_skips_fields_without_position() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create template with one field WITH position and one WITHOUT
    let tmpl = TemplateRepo::create(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tid.clone(),
            name: "Partial Position Form".into(),
            description: None,
            created_by: "test-user".into(),
        },
    )
    .await
    .unwrap();

    FieldRepo::create(&pool, tmpl.id, &tid, &CreateFieldRequest {
        field_key: "with_pos".into(),
        field_label: "Has Position".into(),
        field_type: "text".into(),
        validation_rules: None,
        pdf_position: Some(pos(50.0, 500.0, 1, 12.0)),
    })
    .await
    .unwrap();

    // No pdf_position (defaults to {})
    FieldRepo::create(&pool, tmpl.id, &tid, &CreateFieldRequest {
        field_key: "no_pos".into(),
        field_label: "No Position".into(),
        field_type: "text".into(),
        validation_rules: None,
        pdf_position: None,
    })
    .await
    .unwrap();

    let fields = FieldRepo::list_by_template(&pool, tmpl.id).await.unwrap();
    let data = serde_json::json!({
        "with_pos": "Hello",
        "no_pos": "Skipped",
    });

    let pdf_bytes = test_pdf_bytes();
    let result = generate_filled_pdf(&pdf_bytes, &fields, &data);
    assert!(result.is_ok(), "Should succeed, skipping fields without valid position");
}

#[test]
fn test_validate_pdf_rejects_non_pdf() {
    assert!(matches!(
        validate_pdf(b"not a pdf"),
        Err(GenerateError::InvalidMagic)
    ));
}

#[test]
fn test_validate_pdf_accepts_valid() {
    let bytes = test_pdf_bytes();
    assert!(validate_pdf(&bytes).is_ok());
}

#[tokio::test]
#[serial]
async fn test_generate_with_empty_field_data() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_template_with_positions(&pool, &tid).await;

    let fields = FieldRepo::list_by_template(&pool, tmpl_id).await.unwrap();
    let pdf_bytes = test_pdf_bytes();

    // Empty field data — no fields matched, output should still be valid PDF
    let result = generate_filled_pdf(&pdf_bytes, &fields, &serde_json::json!({}));
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.starts_with(b"%PDF-"));
}
