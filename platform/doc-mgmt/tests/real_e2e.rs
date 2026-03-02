//! End-to-end integration tests for the doc_mgmt service.
//!
//! These tests run against a real Postgres database. No mocks, no stubs.
//! Default connection: `postgresql://doc_mgmt_user:doc_mgmt_pass@localhost:5455/doc_mgmt_db`

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

const DEFAULT_DB_URL: &str =
    "postgresql://doc_mgmt_user:doc_mgmt_pass@localhost:5455/doc_mgmt_db";

async fn get_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to doc_mgmt test DB");

    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

// ── Test helpers ─────────────────────────────────────────────────────

/// Create a test document directly in the database, returning (doc_id, tenant_id).
async fn insert_test_doc(
    pool: &PgPool,
    tenant_id: Uuid,
    doc_number: &str,
    status: &str,
) -> Uuid {
    let doc_id = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Test Doc', 'spec', $4, $5, $6, $6)",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(doc_number)
    .bind(status)
    .bind(actor)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert test doc");

    // Always create at least one revision
    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, created_by, created_at)
         VALUES ($1, $2, $3, 1, '{}'::jsonb, 'Initial', $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert test revision");

    doc_id
}

// ── Schema tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn schema_tables_exist() {
    let pool = get_pool().await;

    // Verify all expected tables exist
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = 'public'
         AND table_name IN ('documents', 'revisions', 'doc_outbox', 'doc_idempotency_keys')
         ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .expect("query tables");

    assert_eq!(
        tables,
        vec!["doc_idempotency_keys", "doc_outbox", "documents", "revisions"]
    );
}

#[tokio::test]
async fn schema_unique_constraint_on_doc_number() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("UNIQ-{}", Uuid::new_v4());

    insert_test_doc(&pool, tenant_id, &doc_number, "draft").await;

    // Second insert with same tenant + doc_number should fail
    let result = sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Dup', 'spec', 'draft', $4, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(&doc_number)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert!(result.is_err(), "duplicate doc_number should be rejected");

    // Same doc_number but different tenant should succeed
    let different_tenant = Uuid::new_v4();
    let result = sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Different Tenant', 'spec', 'draft', $4, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(different_tenant)
    .bind(&doc_number)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "same doc_number in different tenant should succeed"
    );
}

// ── Lifecycle tests ──────────────────────────────────────────────────

#[tokio::test]
async fn lifecycle_draft_to_released() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("LC-{}", Uuid::new_v4());

    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "draft").await;

    // Verify it's draft
    let status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "draft");

    // Release it
    let result = sqlx::query(
        "UPDATE documents SET status = 'released', updated_at = now()
         WHERE id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("release doc");

    assert_eq!(result.rows_affected(), 1);

    // Verify it's released
    let status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "released");

    // Trying to release again should affect 0 rows (idempotent guard)
    let result = sqlx::query(
        "UPDATE documents SET status = 'released', updated_at = now()
         WHERE id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await
    .expect("re-release doc");

    assert_eq!(
        result.rows_affected(),
        0,
        "already released doc should not match"
    );
}

// ── Revision tests ───────────────────────────────────────────────────

#[tokio::test]
async fn revisions_increment_correctly() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("REV-{}", Uuid::new_v4());

    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "draft").await;

    // Initial revision is 1 (created by insert_test_doc)
    let max_rev: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(revision_number) FROM revisions WHERE document_id = $1",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("query max rev");
    assert_eq!(max_rev, Some(1));

    // Add revision 2
    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, created_by, created_at)
         VALUES ($1, $2, $3, 2, '{\"updated\": true}'::jsonb, 'Second revision', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert rev 2");

    // Add revision 3
    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, created_by, created_at)
         VALUES ($1, $2, $3, 3, '{\"final\": true}'::jsonb, 'Third revision', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("insert rev 3");

    let max_rev: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(revision_number) FROM revisions WHERE document_id = $1",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("query max rev");
    assert_eq!(max_rev, Some(3));

    // Duplicate revision_number should fail (unique constraint)
    let dup = sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, created_by, created_at)
         VALUES ($1, $2, $3, 2, '{}'::jsonb, 'Dup', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert!(dup.is_err(), "duplicate revision_number should be rejected");
}

// ── Tenant isolation tests ───────────────────────────────────────────

#[tokio::test]
async fn tenant_isolation_enforced() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let doc_number_a = format!("ISO-A-{}", Uuid::new_v4());
    let doc_number_b = format!("ISO-B-{}", Uuid::new_v4());

    let doc_a = insert_test_doc(&pool, tenant_a, &doc_number_a, "draft").await;
    let _doc_b = insert_test_doc(&pool, tenant_b, &doc_number_b, "draft").await;

    // Tenant B should not see tenant A's document
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_a)
    .bind(tenant_b)
    .fetch_one(&pool)
    .await
    .expect("cross-tenant query");

    assert_eq!(count, 0, "tenant B must not see tenant A's document");

    // Tenant A can see its own document
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_a)
    .bind(tenant_a)
    .fetch_one(&pool)
    .await
    .expect("own-tenant query");

    assert_eq!(count, 1, "tenant A must see its own document");
}

// ── Outbox tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn outbox_event_written_atomically() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();

    // Simulate a mutation that writes doc + outbox atomically
    let mut tx = pool.begin().await.expect("begin tx");

    let doc_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Outbox Test', 'spec', 'draft', $4, now(), now())",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(format!("OBX-{}", Uuid::new_v4()))
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("insert doc");

    let event_payload = serde_json::json!({
        "event_type": "document.created",
        "document_id": doc_id,
        "tenant_id": tenant_id,
    });

    sqlx::query(
        "INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)",
    )
    .bind("document.created")
    .bind("doc_mgmt.events.document.created")
    .bind(&event_payload)
    .execute(&mut *tx)
    .await
    .expect("insert outbox");

    tx.commit().await.expect("commit tx");

    // Verify both exist
    let doc_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM documents WHERE id = $1)",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("check doc");
    assert!(doc_exists);

    let outbox_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM doc_outbox WHERE payload->>'document_id' = $1)",
    )
    .bind(doc_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("check outbox");
    assert!(outbox_exists);
}

// ── Idempotency tests ───────────────────────────────────────────────

#[tokio::test]
async fn idempotency_key_prevents_duplicates() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4().to_string();
    let idem_key = format!("doc:create:{}:test-{}", tenant_id, Uuid::new_v4());

    let response_body = serde_json::json!({"document_id": Uuid::new_v4()});

    // First insert
    sqlx::query(
        "INSERT INTO doc_idempotency_keys (app_id, idempotency_key, response_body, status_code, expires_at)
         VALUES ($1, $2, $3, 201, now() + interval '24 hours')",
    )
    .bind(&tenant_id)
    .bind(&idem_key)
    .bind(&response_body)
    .execute(&pool)
    .await
    .expect("first insert");

    // Fetch it back
    #[derive(sqlx::FromRow)]
    #[allow(dead_code)]
    struct IdemRow {
        status_code: i32,
        response_body: serde_json::Value,
    }

    let cached = sqlx::query_as::<_, IdemRow>(
        "SELECT status_code, response_body FROM doc_idempotency_keys
         WHERE app_id = $1 AND idempotency_key = $2 AND expires_at > now()",
    )
    .bind(&tenant_id)
    .bind(&idem_key)
    .fetch_optional(&pool)
    .await
    .expect("check idempotency");

    assert!(cached.is_some());
    let cached = cached.unwrap();
    assert_eq!(cached.status_code, 201);

    // Duplicate insert (ON CONFLICT DO NOTHING)
    let dup = sqlx::query(
        "INSERT INTO doc_idempotency_keys (app_id, idempotency_key, response_body, status_code, expires_at)
         VALUES ($1, $2, $3, 200, now() + interval '24 hours')
         ON CONFLICT (app_id, idempotency_key) DO NOTHING",
    )
    .bind(&tenant_id)
    .bind(&idem_key)
    .bind(serde_json::json!({"different": "response"}))
    .execute(&pool)
    .await
    .expect("dup insert");

    assert_eq!(dup.rows_affected(), 0, "duplicate should be a no-op");
}

// ── EventEnvelope tests ──────────────────────────────────────────────

#[tokio::test]
async fn event_envelope_validates_and_serializes() {
    use event_bus::outbox::validate_and_serialize_envelope;
    use platform_contracts::{mutation_classes, EventEnvelope};

    let tenant_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.created".to_string(),
        doc_mgmt::models::DocumentCreatedPayload {
            document_id: doc_id,
            doc_number: "DOC-001".to_string(),
            title: "Test Document".to_string(),
            doc_type: "spec".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(Uuid::new_v4(), "User".to_string());

    let result = validate_and_serialize_envelope(&envelope);
    assert!(result.is_ok(), "envelope should validate: {:?}", result.err());

    let payload = result.unwrap();
    assert_eq!(payload["source_module"], "doc_mgmt");
    assert_eq!(payload["event_type"], "document.created");
    assert_eq!(payload["tenant_id"], tenant_id.to_string());
    assert_eq!(payload["mutation_class"], "LIFECYCLE");
    assert!(payload["event_id"].is_string());
    assert!(payload["occurred_at"].is_string());

    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["doc_number"], "DOC-001");
}

// ── Guard → Mutation → Outbox atomicity test ─────────────────────────

#[tokio::test]
async fn guard_mutation_outbox_atomicity_on_rollback() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();

    // Start a transaction and insert doc + outbox, then rollback
    let mut tx = pool.begin().await.expect("begin tx");

    let doc_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Rollback Test', 'spec', 'draft', $4, now(), now())",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(format!("RB-{}", Uuid::new_v4()))
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("insert doc in tx");

    sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
        .bind("document.created")
        .bind("doc_mgmt.events.document.created")
        .bind(serde_json::json!({"document_id": doc_id}))
        .execute(&mut *tx)
        .await
        .expect("insert outbox in tx");

    // Rollback
    tx.rollback().await.expect("rollback");

    // Neither doc nor outbox event should exist
    let doc_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM documents WHERE id = $1)")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("check doc after rollback");
    assert!(!doc_exists, "doc should not exist after rollback");

    let outbox_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM doc_outbox WHERE payload->>'document_id' = $1)",
    )
    .bind(doc_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("check outbox after rollback");
    assert!(!outbox_exists, "outbox event should not exist after rollback");
}

// ══════════════════════════════════════════════════════════════════════
// DOC1 — Revision immutability + supersede linkage
// ══════════════════════════════════════════════════════════════════════

/// Helper: insert a test doc, release it, verify the revision is now 'released'.
async fn insert_and_release_doc(
    pool: &PgPool,
    tenant_id: Uuid,
    doc_number: &str,
) -> (Uuid, Uuid) {
    let doc_id = insert_test_doc(pool, tenant_id, doc_number, "draft").await;

    // Release document
    sqlx::query(
        "UPDATE documents SET status = 'released', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("release doc");

    // Mark revisions as released (like release_document handler does)
    sqlx::query(
        "UPDATE revisions SET status = 'released'
         WHERE document_id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(pool)
    .await
    .expect("mark revisions released");

    // Get revision id
    let rev_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE document_id = $1 AND tenant_id = $2 LIMIT 1",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_one(pool)
    .await
    .expect("get revision id");

    (doc_id, rev_id)
}

// ── DB-enforced immutability: UPDATE on released revision must FAIL ──

#[tokio::test]
async fn released_revision_update_blocked_by_trigger() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("IMMUT-{}", Uuid::new_v4());

    let (_doc_id, rev_id) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    // Verify revision is released
    let status: String =
        sqlx::query_scalar("SELECT status FROM revisions WHERE id = $1")
            .bind(rev_id)
            .fetch_one(&pool)
            .await
            .expect("fetch revision status");
    assert_eq!(status, "released");

    // Try to update body — must fail
    let result = sqlx::query("UPDATE revisions SET body = '{\"tampered\": true}'::jsonb WHERE id = $1")
        .bind(rev_id)
        .execute(&pool)
        .await;

    assert!(
        result.is_err(),
        "UPDATE on released revision body must be rejected by trigger"
    );

    // Try to update change_summary — must fail
    let result = sqlx::query("UPDATE revisions SET change_summary = 'hacked' WHERE id = $1")
        .bind(rev_id)
        .execute(&pool)
        .await;

    assert!(
        result.is_err(),
        "UPDATE on released revision change_summary must be rejected by trigger"
    );
}

// ── DB-enforced immutability: DELETE on released revision must FAIL ──

#[tokio::test]
async fn released_revision_delete_blocked_by_trigger() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("IMDEL-{}", Uuid::new_v4());

    let (_doc_id, rev_id) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    let result = sqlx::query("DELETE FROM revisions WHERE id = $1")
        .bind(rev_id)
        .execute(&pool)
        .await;

    assert!(
        result.is_err(),
        "DELETE on released revision must be rejected by trigger"
    );
}

// ── Draft revisions CAN still be updated ─────────────────────────────

#[tokio::test]
async fn draft_revision_update_allowed() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("DRAFT-{}", Uuid::new_v4());

    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "draft").await;

    let rev_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE document_id = $1 LIMIT 1",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("get revision id");

    // Draft revision: update should succeed
    let result = sqlx::query("UPDATE revisions SET body = '{\"edited\": true}'::jsonb WHERE id = $1")
        .bind(rev_id)
        .execute(&pool)
        .await;

    assert!(result.is_ok(), "UPDATE on draft revision should be allowed");
}

// ── Supersede: creates new doc, marks old as superseded ──────────────

#[tokio::test]
async fn supersede_creates_new_doc_and_links() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let old_doc_number = format!("SS-OLD-{}", Uuid::new_v4());
    let new_doc_number = format!("SS-NEW-{}", Uuid::new_v4());

    let (old_doc_id, _) = insert_and_release_doc(&pool, tenant_id, &old_doc_number).await;

    // Supersede: create new doc, mark old as superseded
    let new_doc_id = Uuid::new_v4();
    let new_rev_id = Uuid::new_v4();
    let now = Utc::now();
    let actor = Uuid::new_v4();

    let mut tx = pool.begin().await.expect("begin tx");

    // Insert new document
    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Superseded Doc v2', 'spec', 'draft', $4, $5, $5)",
    )
    .bind(new_doc_id)
    .bind(tenant_id)
    .bind(&new_doc_number)
    .bind(actor)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert new doc");

    // Insert initial revision for new doc
    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 1, '{}'::jsonb, 'Supersedes old', 'draft', $4, $5)",
    )
    .bind(new_rev_id)
    .bind(new_doc_id)
    .bind(tenant_id)
    .bind(actor)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert new revision");

    // Mark old document as superseded
    sqlx::query(
        "UPDATE documents SET status = 'superseded', superseded_by = $1, updated_at = $2
         WHERE id = $3 AND tenant_id = $4",
    )
    .bind(new_doc_id)
    .bind(now)
    .bind(old_doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .expect("mark old doc superseded");

    // Outbox event
    sqlx::query(
        "INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)",
    )
    .bind("document.superseded")
    .bind("doc_mgmt.events.document.superseded")
    .bind(serde_json::json!({
        "old_document_id": old_doc_id.to_string(),
        "new_document_id": new_doc_id.to_string(),
    }))
    .execute(&mut *tx)
    .await
    .expect("insert outbox");

    tx.commit().await.expect("commit tx");

    // Verify old doc is superseded with correct linkage
    let (old_status, superseded_by): (String, Option<Uuid>) = sqlx::query_as(
        "SELECT status, superseded_by FROM documents WHERE id = $1",
    )
    .bind(old_doc_id)
    .fetch_one(&pool)
    .await
    .expect("fetch old doc");

    assert_eq!(old_status, "superseded");
    assert_eq!(superseded_by, Some(new_doc_id));

    // Verify new doc exists as draft
    let new_status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(new_doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch new doc status");
    assert_eq!(new_status, "draft");

    // Verify outbox event
    let outbox_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM doc_outbox WHERE event_type = 'document.superseded' AND payload->>'old_document_id' = $1)",
    )
    .bind(old_doc_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("check outbox");
    assert!(outbox_exists, "supersede event should be in outbox");
}

// ── Supersede only works on released documents ───────────────────────

#[tokio::test]
async fn supersede_rejects_draft_document() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("SS-DRF-{}", Uuid::new_v4());

    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "draft").await;

    // Check the document is draft
    let status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "draft", "document must be in draft for this test");

    // A draft document should NOT be supersedable. Guard check in app code,
    // but we verify at data level that only released docs get superseded_by set.
    let result = sqlx::query(
        "UPDATE documents SET status = 'superseded', superseded_by = $1
         WHERE id = $2 AND status = 'released'",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .execute(&pool)
    .await
    .expect("guard: status check");

    assert_eq!(
        result.rows_affected(),
        0,
        "draft doc should not match the supersede WHERE clause"
    );
}

// ── Revision status propagates on release ────────────────────────────

#[tokio::test]
async fn release_marks_revisions_as_released() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("REL-REV-{}", Uuid::new_v4());

    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "draft").await;

    // Add a second revision (draft)
    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 2, '{\"v2\": true}'::jsonb, 'Second revision', 'draft', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("add second revision");

    // Verify all revisions are draft
    let draft_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revisions WHERE document_id = $1 AND status = 'draft'",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("count draft revisions");
    assert_eq!(draft_count, 2);

    // Release document + revisions (mimicking handler behaviour)
    sqlx::query("UPDATE documents SET status = 'released', updated_at = now() WHERE id = $1")
        .bind(doc_id)
        .execute(&pool)
        .await
        .expect("release doc");

    sqlx::query(
        "UPDATE revisions SET status = 'released' WHERE document_id = $1 AND status = 'draft'",
    )
    .bind(doc_id)
    .execute(&pool)
    .await
    .expect("release revisions");

    // All revisions should now be released
    let released_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM revisions WHERE document_id = $1 AND status = 'released'",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("count released revisions");
    assert_eq!(released_count, 2, "all revisions should be released");

    // And now immutable — cannot update
    let rev_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE document_id = $1 LIMIT 1",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("get any revision");

    let result = sqlx::query("UPDATE revisions SET body = '{\"tampered\": true}'::jsonb WHERE id = $1")
        .bind(rev_id)
        .execute(&pool)
        .await;

    assert!(result.is_err(), "released revision must be immutable");
}

// ── Revision immutability: content_ref-style column test ─────────────
// The bead verification command tests: UPDATE doc_revisions SET content_ref='x'
// Our schema uses 'body' instead of 'content_ref', so we test that.

#[tokio::test]
async fn released_revision_body_immutable_verification() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("VER-{}", Uuid::new_v4());

    let (_doc_id, rev_id) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    // This is the core bead verification: UPDATE must fail at DB layer
    let result = sqlx::query("UPDATE revisions SET body = '{\"content_ref\": \"x\"}'::jsonb WHERE id = $1")
        .bind(rev_id)
        .execute(&pool)
        .await;

    assert!(
        result.is_err(),
        "UPDATE on released revision body must fail at DB layer (trigger enforcement)"
    );

    // Verify the error is from our trigger (check_violation)
    if let Err(sqlx::Error::Database(ref db_err)) = result {
        let msg = db_err.message();
        assert!(
            msg.contains("Cannot modify a released revision"),
            "Error should come from our trigger, got: {}",
            msg
        );
    }
}
