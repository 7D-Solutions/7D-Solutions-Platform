//! Integration tests for form template + field CRUD (bd-2hrz).
//!
//! Covers: template CRUD, field CRUD with display_order, reorder,
//! validation (empty name, invalid type, duplicate key), tenant isolation.

use pdf_editor_rs::domain::forms::{
    CreateFieldRequest, CreateTemplateRequest, FieldRepo, FormError, ListTemplatesQuery,
    ReorderFieldsRequest, TemplateRepo, UpdateFieldRequest, UpdateTemplateRequest,
};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://pdf_editor_user:pdf_editor_pass@localhost:5453/pdf_editor_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to pdf_editor test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

fn unique_tenant() -> String {
    format!("form-test-{}", Uuid::new_v4().simple())
}

async fn create_test_template(pool: &sqlx::PgPool, tid: &str, name: &str) -> Uuid {
    TemplateRepo::create(
        pool,
        &CreateTemplateRequest {
            tenant_id: tid.to_string(),
            name: name.to_string(),
            description: Some("Test template".into()),
            created_by: "test-user".into(),
        },
    )
    .await
    .unwrap()
    .id
}

fn field_req(key: &str, label: &str, ft: &str) -> CreateFieldRequest {
    CreateFieldRequest {
        field_key: key.into(),
        field_label: label.into(),
        field_type: ft.into(),
        validation_rules: None,
        pdf_position: None,
    }
}

#[tokio::test]
#[serial]
async fn test_create_and_get_template() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl = TemplateRepo::create(
        &pool,
        &CreateTemplateRequest {
            tenant_id: tid.clone(),
            name: "Inspection Form".into(),
            description: Some("Annual vehicle inspection".into()),
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap();

    assert_eq!(tmpl.name, "Inspection Form");
    assert_eq!(
        tmpl.description.as_deref(),
        Some("Annual vehicle inspection")
    );
    assert_eq!(tmpl.created_by, "admin");

    let fetched = TemplateRepo::find_by_id(&pool, tmpl.id, &tid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.id, tmpl.id);
}

#[tokio::test]
#[serial]
async fn test_list_and_update_templates() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_test_template(&pool, &tid, "Template A").await;
    let tmpl_b_id = create_test_template(&pool, &tid, "Template B").await;

    let (list, total) = TemplateRepo::list(
        &pool,
        &ListTemplatesQuery {
            tenant_id: tid.clone(),
            page: None,
            page_size: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(total, 2);

    let updated = TemplateRepo::update(
        &pool,
        tmpl_b_id,
        &tid,
        &UpdateTemplateRequest {
            name: Some("Template B Revised".into()),
            description: Some("Updated".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.name, "Template B Revised");
    assert_eq!(updated.description.as_deref(), Some("Updated"));
}

#[tokio::test]
#[serial]
async fn test_create_fields_and_list_ordered() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid, "Field Order Test").await;

    let f1 = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid,
        &CreateFieldRequest {
            field_key: "company_name".into(),
            field_label: "Company Name".into(),
            field_type: "text".into(),
            validation_rules: Some(serde_json::json!({"required": true})),
            pdf_position: Some(
                serde_json::json!({"x": 50, "y": 100, "width": 200, "height": 30, "page": 1}),
            ),
        },
    )
    .await
    .unwrap();
    assert_eq!(f1.display_order, 0);

    let f2 = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid,
        &CreateFieldRequest {
            field_key: "inspection_date".into(),
            field_label: "Inspection Date".into(),
            field_type: "date".into(),
            validation_rules: None,
            pdf_position: Some(
                serde_json::json!({"x": 50, "y": 150, "width": 150, "height": 30, "page": 1}),
            ),
        },
    )
    .await
    .unwrap();
    assert_eq!(f2.display_order, 1);

    let f3 = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid,
        &CreateFieldRequest {
            field_key: "mileage".into(),
            field_label: "Current Mileage".into(),
            field_type: "number".into(),
            validation_rules: Some(serde_json::json!({"required": true, "min": 0})),
            pdf_position: Some(
                serde_json::json!({"x": 50, "y": 200, "width": 100, "height": 30, "page": 1}),
            ),
        },
    )
    .await
    .unwrap();
    assert_eq!(f3.display_order, 2);

    let fields = FieldRepo::list(&pool, tmpl_id, &tid).await.unwrap();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].field_key, "company_name");
    assert_eq!(fields[1].field_key, "inspection_date");
    assert_eq!(fields[2].field_key, "mileage");
    for (i, f) in fields.iter().enumerate() {
        assert_eq!(f.display_order, i as i32);
    }
}

#[tokio::test]
#[serial]
async fn test_reorder_fields() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid, "Reorder Test").await;

    let f1 = FieldRepo::create(&pool, tmpl_id, &tid, &field_req("alpha", "Alpha", "text"))
        .await
        .unwrap();
    let f2 = FieldRepo::create(&pool, tmpl_id, &tid, &field_req("beta", "Beta", "text"))
        .await
        .unwrap();
    let f3 = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid,
        &field_req("gamma", "Gamma", "checkbox"),
    )
    .await
    .unwrap();

    // Reorder: gamma, alpha, beta
    let reordered = FieldRepo::reorder(
        &pool,
        tmpl_id,
        &tid,
        &ReorderFieldsRequest {
            field_ids: vec![f3.id, f1.id, f2.id],
        },
    )
    .await
    .unwrap();

    assert_eq!(reordered.len(), 3);
    assert_eq!(reordered[0].field_key, "gamma");
    assert_eq!(reordered[0].display_order, 0);
    assert_eq!(reordered[1].field_key, "alpha");
    assert_eq!(reordered[1].display_order, 1);
    assert_eq!(reordered[2].field_key, "beta");
    assert_eq!(reordered[2].display_order, 2);
}

#[tokio::test]
#[serial]
async fn test_template_empty_name_rejected() {
    let pool = setup_db().await;
    let err = TemplateRepo::create(
        &pool,
        &CreateTemplateRequest {
            tenant_id: unique_tenant(),
            name: "".into(),
            description: None,
            created_by: "admin".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FormError::Validation(_)));
}

#[tokio::test]
#[serial]
async fn test_invalid_field_type_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid, "Bad Field Type").await;
    let err = FieldRepo::create(&pool, tmpl_id, &tid, &field_req("bad", "Bad", "textarea"))
        .await
        .unwrap_err();
    assert!(matches!(err, FormError::Validation(_)));
}

#[tokio::test]
#[serial]
async fn test_duplicate_field_key_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid, "Dup Key Test").await;
    FieldRepo::create(&pool, tmpl_id, &tid, &field_req("email", "Email", "text"))
        .await
        .unwrap();
    let err = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid,
        &field_req("email", "Email Again", "text"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FormError::DuplicateFieldKey));
}

#[tokio::test]
#[serial]
async fn test_tenant_isolation() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid_a, "Tenant A Template").await;

    // Tenant B cannot see tenant A's template
    assert!(TemplateRepo::find_by_id(&pool, tmpl_id, &tid_b)
        .await
        .unwrap()
        .is_none());
    let (list, _) = TemplateRepo::list(
        &pool,
        &ListTemplatesQuery {
            tenant_id: tid_b.clone(),
            page: None,
            page_size: None,
        },
    )
    .await
    .unwrap();
    assert!(list.is_empty());

    // Tenant B cannot create fields on tenant A's template
    let err = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid_b,
        &field_req("sneaky", "Sneaky", "text"),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FormError::TemplateNotFound));
}

#[tokio::test]
#[serial]
async fn test_update_field() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid, "Update Field Test").await;

    let field = FieldRepo::create(
        &pool,
        tmpl_id,
        &tid,
        &CreateFieldRequest {
            field_key: "notes".into(),
            field_label: "Notes".into(),
            field_type: "text".into(),
            validation_rules: None,
            pdf_position: Some(
                serde_json::json!({"x": 10, "y": 10, "width": 100, "height": 20, "page": 1}),
            ),
        },
    )
    .await
    .unwrap();

    let updated = FieldRepo::update(
        &pool,
        field.id,
        tmpl_id,
        &tid,
        &UpdateFieldRequest {
            field_label: Some("Additional Notes".into()),
            field_type: Some("dropdown".into()),
            validation_rules: Some(serde_json::json!({"options": ["a", "b"]})),
            pdf_position: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.field_label, "Additional Notes");
    assert_eq!(updated.field_type, "dropdown");
    assert_eq!(
        updated.pdf_position,
        serde_json::json!({"x": 10, "y": 10, "width": 100, "height": 20, "page": 1})
    );
}

#[tokio::test]
#[serial]
async fn test_reorder_incomplete_ids_rejected() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let tmpl_id = create_test_template(&pool, &tid, "Partial Reorder").await;
    let f1 = FieldRepo::create(&pool, tmpl_id, &tid, &field_req("one", "One", "text"))
        .await
        .unwrap();
    FieldRepo::create(&pool, tmpl_id, &tid, &field_req("two", "Two", "text"))
        .await
        .unwrap();

    let err = FieldRepo::reorder(
        &pool,
        tmpl_id,
        &tid,
        &ReorderFieldsRequest {
            field_ids: vec![f1.id],
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, FormError::Validation(_)));
}
