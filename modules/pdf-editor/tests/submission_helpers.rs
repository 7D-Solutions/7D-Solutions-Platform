//! Shared helpers for submission integration tests.

#![allow(dead_code)]

use pdf_editor::domain::forms::{
    CreateFieldRequest, CreateTemplateRequest, FieldRepo, TemplateRepo,
};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

pub async fn setup_db() -> sqlx::PgPool {
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

pub fn unique_tenant() -> String {
    format!("sub-test-{}", Uuid::new_v4().simple())
}

/// Create a template with fields for testing submissions.
pub async fn create_test_template_with_fields(pool: &sqlx::PgPool, tid: &str) -> Uuid {
    let tmpl = TemplateRepo::create(
        pool,
        &CreateTemplateRequest {
            tenant_id: tid.to_string(),
            name: "Inspection Form".into(),
            description: Some("Test form".into()),
            created_by: "test-user".into(),
        },
    )
    .await
    .unwrap();

    let fields = vec![
        (
            "company_name",
            "Company Name",
            "text",
            serde_json::json!({"required": true}),
        ),
        (
            "mileage",
            "Current Mileage",
            "number",
            serde_json::json!({"required": true, "min": 0, "max": 999999}),
        ),
        (
            "inspection_date",
            "Inspection Date",
            "date",
            serde_json::json!({"required": true}),
        ),
        (
            "vehicle_type",
            "Vehicle Type",
            "dropdown",
            serde_json::json!({"required": true, "options": ["truck", "van", "car"]}),
        ),
        (
            "passed",
            "Passed Inspection",
            "checkbox",
            serde_json::json!({"required": true}),
        ),
        ("notes", "Additional Notes", "text", serde_json::json!({})),
    ];

    for (key, label, ft, rules) in fields {
        FieldRepo::create(
            pool,
            tmpl.id,
            tid,
            &CreateFieldRequest {
                field_key: key.into(),
                field_label: label.into(),
                field_type: ft.into(),
                validation_rules: Some(rules),
                pdf_position: None,
            },
        )
        .await
        .unwrap();
    }

    tmpl.id
}

pub fn valid_field_data() -> serde_json::Value {
    serde_json::json!({
        "company_name": "Acme Corp",
        "mileage": 42000,
        "inspection_date": "2026-02-24",
        "vehicle_type": "truck",
        "passed": true,
    })
}
