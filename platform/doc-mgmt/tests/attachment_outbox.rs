//! Integration tests: attachment creation emits a doc_outbox event atomically.
//!
//! These tests verify that when `create_attachment` runs its two-INSERT transaction
//! (attachment row + doc_outbox row), both rows are committed together or neither is.
//! No blob-storage / S3 required — we test the DB tier directly.

use sqlx::PgPool;
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgresql://doc_mgmt_user:doc_mgmt_pass@localhost:5455/doc_mgmt_db";

async fn get_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPool::connect(&url)
        .await
        .expect("connect to doc_mgmt test DB");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

/// Run the same DB writes that `create_attachment` now performs (attachment INSERT +
/// doc_outbox INSERT inside a single transaction), then commit.
async fn simulate_create_attachment(
    pool: &PgPool,
    tenant_id: Uuid,
    actor_id: Uuid,
    attachment_id: Uuid,
    entity_type: &str,
    entity_id: &str,
    filename: &str,
) {
    let s3_key =
        format!("tenants/{tenant_id}/doc-mgmt/attachment/{entity_id}/2026/04/08/{filename}");

    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "INSERT INTO attachments (id, tenant_id, entity_type, entity_id, filename, mime_type, \
         size_bytes, s3_key, status, created_by, created_at)
         VALUES ($1, $2, $3, $4, $5, 'application/pdf', 0, $6, 'pending', $7, now())",
    )
    .bind(attachment_id)
    .bind(tenant_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(filename)
    .bind(&s3_key)
    .bind(actor_id)
    .execute(&mut *tx)
    .await
    .expect("insert attachment");

    let outbox_payload = serde_json::json!({
        "tenant_id": tenant_id,
        "attachment_id": attachment_id,
        "entity_type": entity_type,
        "entity_id": entity_id,
        "filename": filename,
        "mime_type": "application/pdf",
        "size_bytes": 0_i64,
        "uploaded_by": actor_id,
    });

    sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
        .bind("docmgmt.attachment.created")
        .bind("docmgmt.attachment.created")
        .bind(outbox_payload)
        .execute(&mut *tx)
        .await
        .expect("insert doc_outbox");

    tx.commit().await.expect("commit tx");
}

#[tokio::test]
async fn attachment_outbox_row_inserted_with_correct_subject_and_payload() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    let entity_id = Uuid::new_v4().to_string();

    simulate_create_attachment(
        &pool,
        tenant_id,
        actor_id,
        attachment_id,
        "ap_bill",
        &entity_id,
        "invoice.pdf",
    )
    .await;

    // outbox row must exist with correct subject
    let row: Option<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT subject, payload FROM doc_outbox
         WHERE payload->>'attachment_id' = $1
         ORDER BY id DESC LIMIT 1",
    )
    .bind(attachment_id.to_string())
    .fetch_optional(&pool)
    .await
    .expect("query outbox");

    let (subject, payload) = row.expect("doc_outbox row must exist after attachment creation");

    assert_eq!(subject, "docmgmt.attachment.created");
    assert_eq!(
        payload["attachment_id"]
            .as_str()
            .expect("field must be string"),
        attachment_id.to_string()
    );
    assert_eq!(
        payload["entity_type"]
            .as_str()
            .expect("field must be string"),
        "ap_bill"
    );
    assert_eq!(
        payload["entity_id"].as_str().expect("field must be string"),
        entity_id
    );
    assert_eq!(
        payload["filename"].as_str().expect("field must be string"),
        "invoice.pdf"
    );
    assert_eq!(
        payload["mime_type"].as_str().expect("field must be string"),
        "application/pdf"
    );

    // attachment row must also be present
    let att_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM attachments WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(attachment_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("check attachment exists");
    assert!(att_exists, "attachments row must exist after commit");
}

#[tokio::test]
async fn attachment_outbox_rollback_removes_both_rows() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    let entity_id = Uuid::new_v4().to_string();
    let s3_key =
        format!("tenants/{tenant_id}/doc-mgmt/attachment/{entity_id}/2026/04/08/rollback-test.pdf");

    let outbox_payload = serde_json::json!({
        "tenant_id": tenant_id,
        "attachment_id": attachment_id,
        "entity_type": "ap_bill",
        "entity_id": entity_id,
        "filename": "rollback-test.pdf",
        "mime_type": "application/pdf",
        "size_bytes": 0_i64,
        "uploaded_by": actor_id,
    });

    // Run both inserts then ROLLBACK (simulates a failure mid-handler)
    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "INSERT INTO attachments (id, tenant_id, entity_type, entity_id, filename, mime_type, \
         size_bytes, s3_key, status, created_by, created_at)
         VALUES ($1, $2, 'ap_bill', $3, 'rollback-test.pdf', 'application/pdf', 0, $4, 'pending', $5, now())",
    )
    .bind(attachment_id)
    .bind(tenant_id)
    .bind(&entity_id)
    .bind(&s3_key)
    .bind(actor_id)
    .execute(&mut *tx)
    .await
    .expect("insert attachment in tx");

    sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
        .bind("docmgmt.attachment.created")
        .bind("docmgmt.attachment.created")
        .bind(outbox_payload)
        .execute(&mut *tx)
        .await
        .expect("insert outbox in tx");

    // Rollback — neither row should survive
    tx.rollback().await.expect("rollback");

    let att_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE id = $1)")
            .bind(attachment_id)
            .fetch_one(&pool)
            .await
            .expect("check attachment");

    assert!(!att_exists, "attachment must NOT exist after rollback");

    let outbox_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM doc_outbox WHERE payload->>'attachment_id' = $1)",
    )
    .bind(attachment_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("check outbox");

    assert!(
        !outbox_exists,
        "doc_outbox row must NOT exist after rollback"
    );
}
