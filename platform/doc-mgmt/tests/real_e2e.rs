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

// ══════════════════════════════════════════════════════════════════════
// DOC2 — Retention management + legal hold
// ══════════════════════════════════════════════════════════════════════

// ── Retention policy tests ───────────────────────────────────────────

#[tokio::test]
async fn retention_policy_upsert_and_read() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let now = Utc::now();

    // Insert a policy
    let policy_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, 'spec', 365, $3, $4, $4)",
    )
    .bind(policy_id)
    .bind(tenant_id)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert retention policy");

    // Read it back
    let days: i32 = sqlx::query_scalar(
        "SELECT retention_days FROM retention_policies WHERE tenant_id = $1 AND doc_type = 'spec'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("read retention policy");
    assert_eq!(days, 365);

    // Upsert (update retention_days)
    sqlx::query(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, 'spec', 730, $3, $4, $4)
         ON CONFLICT (tenant_id, doc_type) DO UPDATE
           SET retention_days = EXCLUDED.retention_days, updated_at = EXCLUDED.updated_at",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(actor)
    .bind(Utc::now())
    .execute(&pool)
    .await
    .expect("upsert retention policy");

    let days: i32 = sqlx::query_scalar(
        "SELECT retention_days FROM retention_policies WHERE tenant_id = $1 AND doc_type = 'spec'",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("read updated policy");
    assert_eq!(days, 730);
}

#[tokio::test]
async fn retention_policy_unique_per_tenant_doc_type() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, 'wo', 90, $3, $4, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await
    .expect("first insert");

    // Duplicate without ON CONFLICT should fail
    let result = sqlx::query(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, 'wo', 180, $3, $4, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "duplicate tenant+doc_type should violate unique constraint"
    );

    // Different tenant, same doc_type should succeed
    let result = sqlx::query(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, 'wo', 180, $3, $4, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(Uuid::new_v4())
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "different tenant should succeed");
}

// ── Legal hold tests ─────────────────────────────────────────────────

#[tokio::test]
async fn apply_hold_creates_active_record() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HOLD-{}", Uuid::new_v4());
    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "released").await;
    let actor = Uuid::new_v4();
    let now = Utc::now();

    let hold_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Litigation case #123', $4, $5)",
    )
    .bind(hold_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await
    .expect("apply hold");

    // Verify it's active (released_at IS NULL)
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds
         WHERE document_id = $1 AND tenant_id = $2 AND released_at IS NULL",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("count active holds");

    assert_eq!(active_count, 1);
}

#[tokio::test]
async fn hold_blocks_disposal_via_trigger() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HBLOCK-{}", Uuid::new_v4());

    let (doc_id, _) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    // Apply a legal hold
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Regulatory audit', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("apply hold");

    // Attempt to dispose — must fail due to DB trigger
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "disposal must be blocked while an active hold exists"
    );

    // Verify error message comes from our trigger
    if let Err(sqlx::Error::Database(ref db_err)) = result {
        let msg = db_err.message();
        assert!(
            msg.contains("active legal hold"),
            "Error should mention active legal hold, got: {}",
            msg
        );
    }
}

#[tokio::test]
async fn release_hold_then_disposal_succeeds() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HREL-{}", Uuid::new_v4());

    let (doc_id, _) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;
    let actor = Uuid::new_v4();

    // Apply hold
    let hold_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Case #456', $4, now())",
    )
    .bind(hold_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("apply hold");

    // Verify disposal blocked
    let blocked = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;
    assert!(blocked.is_err(), "should be blocked while hold is active");

    // Release the hold
    sqlx::query(
        "UPDATE legal_holds SET released_by = $1, released_at = now()
         WHERE id = $2 AND released_at IS NULL",
    )
    .bind(actor)
    .bind(hold_id)
    .execute(&pool)
    .await
    .expect("release hold");

    // Verify hold is released
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds
         WHERE document_id = $1 AND released_at IS NULL",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("count active holds");
    assert_eq!(active_count, 0);

    // Now disposal should succeed
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "disposal should succeed after hold released");
    assert_eq!(result.unwrap().rows_affected(), 1);

    // Verify document is disposed
    let status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "disposed");
}

#[tokio::test]
async fn duplicate_active_hold_same_reason_blocked() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HDUP-{}", Uuid::new_v4());
    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "released").await;
    let actor = Uuid::new_v4();

    // First hold
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Same reason', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("first hold");

    // Duplicate hold with same reason (active) should fail unique constraint
    let result = sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Same reason', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "duplicate active hold with same reason should be rejected"
    );
}

#[tokio::test]
async fn multiple_holds_different_reasons_allowed() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HMULTI-{}", Uuid::new_v4());
    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "released").await;
    let actor = Uuid::new_v4();

    // Hold 1
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Reason A', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("hold 1");

    // Hold 2 (different reason)
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Reason B', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("hold 2");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds
         WHERE document_id = $1 AND released_at IS NULL",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("count holds");

    assert_eq!(count, 2, "two holds with different reasons should coexist");

    // Disposal must still be blocked (both holds active)
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "disposal blocked with any active hold");
}

#[tokio::test]
async fn partial_hold_release_still_blocks_disposal() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HPART-{}", Uuid::new_v4());

    let (doc_id, _) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;
    let actor = Uuid::new_v4();

    // Two holds
    let hold_a = Uuid::new_v4();
    let hold_b = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Hold A', $4, now())",
    )
    .bind(hold_a)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("hold A");

    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Hold B', $4, now())",
    )
    .bind(hold_b)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("hold B");

    // Release only hold A
    sqlx::query(
        "UPDATE legal_holds SET released_by = $1, released_at = now()
         WHERE id = $2 AND released_at IS NULL",
    )
    .bind(actor)
    .bind(hold_a)
    .execute(&pool)
    .await
    .expect("release hold A");

    // Disposal should still be blocked (hold B still active)
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_err(),
        "disposal blocked while hold B still active"
    );

    // Release hold B
    sqlx::query(
        "UPDATE legal_holds SET released_by = $1, released_at = now()
         WHERE id = $2 AND released_at IS NULL",
    )
    .bind(actor)
    .bind(hold_b)
    .execute(&pool)
    .await
    .expect("release hold B");

    // Now disposal succeeds
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "disposal succeeds after all holds released");
}

#[tokio::test]
async fn re_hold_after_release_allowed() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HREHOLD-{}", Uuid::new_v4());
    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "released").await;
    let actor = Uuid::new_v4();

    // Apply hold
    let hold_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Audit hold', $4, now())",
    )
    .bind(hold_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("first hold");

    // Release it
    sqlx::query(
        "UPDATE legal_holds SET released_by = $1, released_at = now()
         WHERE id = $2 AND released_at IS NULL",
    )
    .bind(actor)
    .bind(hold_id)
    .execute(&pool)
    .await
    .expect("release hold");

    // Re-apply same reason — should succeed (old hold is released, new one is active)
    let result = sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Audit hold', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "re-applying hold after release should succeed"
    );

    // Should have 2 total records (1 released, 1 active)
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds WHERE document_id = $1",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("count all holds");
    assert_eq!(total, 2);

    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds
         WHERE document_id = $1 AND released_at IS NULL",
    )
    .bind(doc_id)
    .fetch_one(&pool)
    .await
    .expect("count active holds");
    assert_eq!(active, 1);
}

#[tokio::test]
async fn retention_period_blocks_disposal() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let doc_number = format!("RRET-{}", Uuid::new_v4());

    // Set a 365-day retention policy for 'spec' docs
    sqlx::query(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, 'spec', 365, $3, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(actor)
    .execute(&pool)
    .await
    .expect("insert retention policy");

    // Create a released doc with recent updated_at (retention not met)
    let (doc_id, _) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    // Check: the document was just released (updated_at is now-ish).
    // With 365-day retention, disposal should not be allowed.
    // We verify this at the APPLICATION level (checking retention_policies).
    // The DB trigger only checks holds, not retention. Retention is an app-level guard.

    let doc_type: String =
        sqlx::query_scalar("SELECT doc_type FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch doc_type");

    let updated_at: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT updated_at FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch updated_at");

    let retention_days: i32 = sqlx::query_scalar(
        "SELECT retention_days FROM retention_policies
         WHERE tenant_id = $1 AND doc_type = $2",
    )
    .bind(tenant_id)
    .bind(&doc_type)
    .fetch_one(&pool)
    .await
    .expect("fetch retention days");

    let eligible_after = updated_at + chrono::Duration::days(retention_days as i64);
    let now = Utc::now();

    assert!(
        now < eligible_after,
        "document should NOT be eligible for disposal yet (eligible_after: {}, now: {})",
        eligible_after, now
    );
}

#[tokio::test]
async fn no_retention_policy_allows_disposal() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("RNOPOL-{}", Uuid::new_v4());

    let (doc_id, _) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    // No retention policy for this tenant/doc_type → no retention requirement
    let policy_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM retention_policies WHERE tenant_id = $1 AND doc_type = 'spec')",
    )
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("check policy");
    assert!(!policy_exists, "should have no retention policy");

    // No holds either → disposal should succeed
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(
        result.is_ok(),
        "disposal should succeed with no policy and no holds"
    );

    let status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(doc_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status");
    assert_eq!(status, "disposed");
}

#[tokio::test]
async fn delete_document_with_active_hold_blocked() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("HDEL-{}", Uuid::new_v4());
    let doc_id = insert_test_doc(&pool, tenant_id, &doc_number, "released").await;

    // Apply hold
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Investigation', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("apply hold");

    // DELETE (not just status change) should also be blocked by trigger
    // First remove the FK constraint on revisions so we can try the delete
    // Actually, revisions have FK to documents, so we need to delete revisions first
    // But the point is: the trigger fires before the FK check

    let result = sqlx::query("DELETE FROM documents WHERE id = $1")
        .bind(doc_id)
        .execute(&pool)
        .await;

    assert!(
        result.is_err(),
        "DELETE on document with active hold must be blocked"
    );
}

#[tokio::test]
async fn disposal_of_superseded_document_with_hold_lifecycle() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let doc_number = format!("SSDIS-{}", Uuid::new_v4());

    // Create and release doc, then supersede it
    let (old_doc_id, _) = insert_and_release_doc(&pool, tenant_id, &doc_number).await;

    let new_doc_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Superseded v2', 'spec', 'draft', $4, now(), now())",
    )
    .bind(new_doc_id)
    .bind(tenant_id)
    .bind(format!("SSDIS-NEW-{}", Uuid::new_v4()))
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("insert new doc");

    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 1, '{}'::jsonb, 'Supersedes old', 'draft', $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(new_doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await
    .expect("insert new revision");

    sqlx::query(
        "UPDATE documents SET status = 'superseded', superseded_by = $1, updated_at = now()
         WHERE id = $2 AND tenant_id = $3",
    )
    .bind(new_doc_id)
    .bind(old_doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .expect("mark old superseded");

    tx.commit().await.expect("commit");

    // Old doc is now superseded. Apply a hold.
    let hold_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, 'Archival hold', $4, now())",
    )
    .bind(hold_id)
    .bind(old_doc_id)
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .expect("hold on superseded doc");

    // Disposal of superseded doc should fail (hold active)
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(old_doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;
    assert!(result.is_err(), "disposal blocked on superseded doc with hold");

    // Release hold
    sqlx::query(
        "UPDATE legal_holds SET released_by = $1, released_at = now()
         WHERE id = $2 AND released_at IS NULL",
    )
    .bind(Uuid::new_v4())
    .bind(hold_id)
    .execute(&pool)
    .await
    .expect("release hold");

    // Now disposal should succeed
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = now()
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(old_doc_id)
    .bind(tenant_id)
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "disposal of superseded doc after hold release");

    let status: String =
        sqlx::query_scalar("SELECT status FROM documents WHERE id = $1")
            .bind(old_doc_id)
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(status, "disposed");
}

// ══════════════════════════════════════════════════════════════════════
// DOC-EVENTS — Outbox envelope completeness for all lifecycle events
// ══════════════════════════════════════════════════════════════════════

/// Validate that an outbox payload contains all required EventEnvelope fields.
/// Returns the parsed payload for further inspection.
fn assert_envelope_complete(payload: &serde_json::Value, expected_event_type: &str) {
    // Required string fields
    assert!(
        payload["event_id"].is_string() && !payload["event_id"].as_str().unwrap().is_empty(),
        "event_id must be a non-empty string, got: {:?}",
        payload["event_id"]
    );
    assert_eq!(
        payload["event_type"].as_str().unwrap(),
        expected_event_type,
        "event_type mismatch"
    );
    assert!(
        payload["occurred_at"].is_string() && !payload["occurred_at"].as_str().unwrap().is_empty(),
        "occurred_at must be a non-empty string"
    );
    assert!(
        payload["tenant_id"].is_string() && !payload["tenant_id"].as_str().unwrap().is_empty(),
        "tenant_id must be a non-empty string"
    );
    assert_eq!(
        payload["source_module"].as_str().unwrap(),
        "doc_mgmt",
        "source_module must be doc_mgmt"
    );
    assert!(
        payload["source_version"].is_string()
            && !payload["source_version"].as_str().unwrap().is_empty(),
        "source_version must be a non-empty string"
    );
    assert!(
        payload["schema_version"].is_string()
            && !payload["schema_version"].as_str().unwrap().is_empty(),
        "schema_version must be a non-empty string"
    );
    assert!(
        payload["replay_safe"].is_boolean(),
        "replay_safe must be a boolean"
    );
    assert!(
        payload["mutation_class"].is_string()
            && !payload["mutation_class"].as_str().unwrap().is_empty(),
        "mutation_class must be a non-empty string"
    );
    assert!(
        payload["actor_id"].is_string() && !payload["actor_id"].as_str().unwrap().is_empty(),
        "actor_id must be a non-empty string"
    );
    assert!(
        payload["actor_type"].is_string() && !payload["actor_type"].as_str().unwrap().is_empty(),
        "actor_type must be a non-empty string"
    );
    assert!(
        payload["payload"].is_object(),
        "payload must be an object"
    );
}

/// Helper: write an envelope-complete outbox event the same way the handler does,
/// returning the outbox row id for later verification.
async fn write_envelope_to_outbox<T: serde::Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_type: &str,
    envelope: &event_bus::EventEnvelope<T>,
) -> i64 {
    let event_payload = event_bus::outbox::validate_and_serialize_envelope(envelope)
        .expect("envelope must validate");
    let subject = platform_contracts::event_naming::nats_subject("doc_mgmt", event_type);

    let row_id: i64 = sqlx::query_scalar(
        "INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(event_type)
    .bind(&subject)
    .bind(&event_payload)
    .fetch_one(&mut **tx)
    .await
    .expect("insert outbox");

    row_id
}

/// Fetch the outbox payload by row id.
async fn fetch_outbox_payload(pool: &PgPool, row_id: i64) -> serde_json::Value {
    sqlx::query_scalar("SELECT payload FROM doc_outbox WHERE id = $1")
        .bind(row_id)
        .fetch_one(pool)
        .await
        .expect("fetch outbox payload")
}

// ── document.created envelope ────────────────────────────────────────

#[tokio::test]
async fn event_envelope_document_created_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.created".to_string(),
        doc_mgmt::models::DocumentCreatedPayload {
            document_id: doc_id,
            doc_number: "EVT-DC-001".to_string(),
            title: "Envelope Test".to_string(),
            doc_type: "spec".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "document.created", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "document.created");

    // Verify inner payload
    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["doc_number"], "EVT-DC-001");
    assert_eq!(inner["title"], "Envelope Test");
    assert_eq!(inner["doc_type"], "spec");

    // Verify NATS subject format
    let subject: String =
        sqlx::query_scalar("SELECT subject FROM doc_outbox WHERE id = $1")
            .bind(row_id)
            .fetch_one(&pool)
            .await
            .expect("fetch subject");
    assert_eq!(subject, "doc_mgmt.events.document.created");
}

// ── revision.created envelope ────────────────────────────────────────

#[tokio::test]
async fn event_envelope_revision_created_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let rev_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "revision.created".to_string(),
        doc_mgmt::models::RevisionCreatedPayload {
            document_id: doc_id,
            revision_id: rev_id,
            revision_number: 2,
        },
    )
    .with_mutation_class(Some(mutation_classes::DATA_MUTATION.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "revision.created", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "revision.created");
    assert_eq!(payload["mutation_class"], "DATA_MUTATION");

    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["revision_id"], rev_id.to_string());
    assert_eq!(inner["revision_number"], 2);

    let subject: String =
        sqlx::query_scalar("SELECT subject FROM doc_outbox WHERE id = $1")
            .bind(row_id)
            .fetch_one(&pool)
            .await
            .expect("fetch subject");
    assert_eq!(subject, "doc_mgmt.events.revision.created");
}

// ── document.released envelope ───────────────────────────────────────

#[tokio::test]
async fn event_envelope_document_released_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.released".to_string(),
        doc_mgmt::models::DocumentReleasedPayload {
            document_id: doc_id,
            doc_number: "EVT-REL-001".to_string(),
            revision_number: 3,
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "document.released", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "document.released");
    assert_eq!(payload["mutation_class"], "LIFECYCLE");

    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["doc_number"], "EVT-REL-001");
    assert_eq!(inner["revision_number"], 3);

    let subject: String =
        sqlx::query_scalar("SELECT subject FROM doc_outbox WHERE id = $1")
            .bind(row_id)
            .fetch_one(&pool)
            .await
            .expect("fetch subject");
    assert_eq!(subject, "doc_mgmt.events.document.released");
}

// ── document.superseded envelope ─────────────────────────────────────

#[tokio::test]
async fn event_envelope_document_superseded_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let old_doc_id = Uuid::new_v4();
    let new_doc_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.superseded".to_string(),
        doc_mgmt::models::DocumentSupersededPayload {
            old_document_id: old_doc_id,
            new_document_id: new_doc_id,
            new_doc_number: "EVT-SS-002".to_string(),
            old_doc_number: "EVT-SS-001".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "document.superseded", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "document.superseded");

    let inner = &payload["payload"];
    assert_eq!(inner["old_document_id"], old_doc_id.to_string());
    assert_eq!(inner["new_document_id"], new_doc_id.to_string());
    assert_eq!(inner["new_doc_number"], "EVT-SS-002");
    assert_eq!(inner["old_doc_number"], "EVT-SS-001");
}

// ── legal_hold.applied envelope ──────────────────────────────────────

#[tokio::test]
async fn event_envelope_legal_hold_applied_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let hold_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "legal_hold.applied".to_string(),
        doc_mgmt::models::LegalHoldAppliedPayload {
            document_id: doc_id,
            hold_id,
            reason: "Regulatory investigation".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "legal_hold.applied", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "legal_hold.applied");

    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["hold_id"], hold_id.to_string());
    assert_eq!(inner["reason"], "Regulatory investigation");
}

// ── legal_hold.released envelope ─────────────────────────────────────

#[tokio::test]
async fn event_envelope_legal_hold_released_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let hold_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "legal_hold.released".to_string(),
        doc_mgmt::models::LegalHoldReleasedPayload {
            document_id: doc_id,
            hold_id,
            reason: "Investigation concluded".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "legal_hold.released", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "legal_hold.released");

    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["hold_id"], hold_id.to_string());
    assert_eq!(inner["reason"], "Investigation concluded");
}

// ── document.disposed envelope ───────────────────────────────────────

#[tokio::test]
async fn event_envelope_document_disposed_complete() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.disposed".to_string(),
        doc_mgmt::models::DocumentDisposedPayload {
            document_id: doc_id,
            doc_number: "EVT-DISP-001".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row_id = write_envelope_to_outbox(&mut tx, "document.disposed", &envelope).await;
    tx.commit().await.expect("commit");

    let payload = fetch_outbox_payload(&pool, row_id).await;
    assert_envelope_complete(&payload, "document.disposed");

    let inner = &payload["payload"];
    assert_eq!(inner["document_id"], doc_id.to_string());
    assert_eq!(inner["doc_number"], "EVT-DISP-001");
}

// ── Replay safety: event_id uniqueness across outbox rows ────────────

#[tokio::test]
async fn outbox_event_ids_are_unique_across_events() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();

    // Create two different events
    let envelope1 = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.created".to_string(),
        doc_mgmt::models::DocumentCreatedPayload {
            document_id: Uuid::new_v4(),
            doc_number: "DEDUP-001".to_string(),
            title: "First".to_string(),
            doc_type: "spec".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let envelope2 = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.created".to_string(),
        doc_mgmt::models::DocumentCreatedPayload {
            document_id: Uuid::new_v4(),
            doc_number: "DEDUP-002".to_string(),
            title: "Second".to_string(),
            doc_type: "spec".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let mut tx = pool.begin().await.expect("begin tx");
    let row1 = write_envelope_to_outbox(&mut tx, "document.created", &envelope1).await;
    let row2 = write_envelope_to_outbox(&mut tx, "document.created", &envelope2).await;
    tx.commit().await.expect("commit");

    let payload1 = fetch_outbox_payload(&pool, row1).await;
    let payload2 = fetch_outbox_payload(&pool, row2).await;

    let eid1 = payload1["event_id"].as_str().unwrap();
    let eid2 = payload2["event_id"].as_str().unwrap();

    assert_ne!(
        eid1, eid2,
        "each event must have a unique event_id for consumer dedup"
    );
}

// ── Full lifecycle: create → release produces outbox events atomically ──

#[tokio::test]
async fn full_lifecycle_outbox_events_atomic() {
    use event_bus::EventEnvelope;
    use platform_contracts::mutation_classes;

    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let doc_id = Uuid::new_v4();
    let rev_id = Uuid::new_v4();
    let now = Utc::now();
    let doc_number = format!("FULL-LC-{}", Uuid::new_v4());

    // ── Step 1: Create document + outbox (atomic) ─────────────────────
    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'Full Lifecycle', 'spec', 'draft', $4, $5, $5)",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&doc_number)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert doc");

    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 1, '{}'::jsonb, 'Initial', 'draft', $4, $5)",
    )
    .bind(rev_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert revision");

    let create_envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.created".to_string(),
        doc_mgmt::models::DocumentCreatedPayload {
            document_id: doc_id,
            doc_number: doc_number.clone(),
            title: "Full Lifecycle".to_string(),
            doc_type: "spec".to_string(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let create_row = write_envelope_to_outbox(&mut tx, "document.created", &create_envelope).await;
    tx.commit().await.expect("commit create");

    // ── Step 2: Release document + outbox (atomic) ────────────────────
    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "UPDATE documents SET status = 'released', updated_at = now()
         WHERE id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .expect("release doc");

    sqlx::query(
        "UPDATE revisions SET status = 'released'
         WHERE document_id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    .expect("release revisions");

    let release_envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.released".to_string(),
        doc_mgmt::models::DocumentReleasedPayload {
            document_id: doc_id,
            doc_number: doc_number.clone(),
            revision_number: 1,
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, "User".to_string());

    let release_row =
        write_envelope_to_outbox(&mut tx, "document.released", &release_envelope).await;
    tx.commit().await.expect("commit release");

    // ── Verify both outbox events ─────────────────────────────────────
    let create_payload = fetch_outbox_payload(&pool, create_row).await;
    assert_envelope_complete(&create_payload, "document.created");

    let release_payload = fetch_outbox_payload(&pool, release_row).await;
    assert_envelope_complete(&release_payload, "document.released");

    // Verify causality: different event_ids, same tenant
    assert_ne!(
        create_payload["event_id"].as_str().unwrap(),
        release_payload["event_id"].as_str().unwrap()
    );
    assert_eq!(
        create_payload["tenant_id"].as_str().unwrap(),
        release_payload["tenant_id"].as_str().unwrap()
    );

    // Both events published_at should be NULL (not yet relayed)
    let unpublished: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM doc_outbox WHERE id IN ($1, $2) AND published_at IS NULL",
    )
    .bind(create_row)
    .bind(release_row)
    .fetch_one(&pool)
    .await
    .expect("count unpublished");
    assert_eq!(unpublished, 2, "both events should be pending relay");
}

// ══════════════════════════════════════════════════════════════════════
// DOC3 — Template engine: template storage + render pipeline
// ══════════════════════════════════════════════════════════════════════

// ── Template helpers ────────────────────────────────────────────────

async fn insert_test_template(
    pool: &PgPool,
    tenant_id: Uuid,
    name: &str,
    doc_type: &str,
    body_template: serde_json::Value,
) -> Uuid {
    let template_id = Uuid::new_v4();
    let actor = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO doc_templates (id, tenant_id, name, doc_type, body_template, version, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 1, $6, $7, $7)",
    )
    .bind(template_id)
    .bind(tenant_id)
    .bind(name)
    .bind(doc_type)
    .bind(&body_template)
    .bind(actor)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert test template");

    template_id
}

// ── Schema: template tables exist ───────────────────────────────────

#[tokio::test]
async fn template_tables_exist() {
    let pool = get_pool().await;

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables
         WHERE table_schema = 'public'
         AND table_name IN ('doc_templates', 'render_artifacts')
         ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await
    .expect("query tables");

    assert_eq!(tables, vec!["doc_templates", "render_artifacts"]);
}

// ── Template CRUD ───────────────────────────────────────────────────

#[tokio::test]
async fn template_create_and_query() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let body_template = serde_json::json!({
        "header": "Work Order: {{wo_number}}",
        "description": "{{description}}",
        "priority": "{{priority}}"
    });

    let template_id = insert_test_template(
        &pool,
        tenant_id,
        "work-order-v1",
        "work_order",
        body_template.clone(),
    )
    .await;

    // Query it back
    let row: (Uuid, String, String, serde_json::Value, i32) = sqlx::query_as(
        "SELECT id, name, doc_type, body_template, version FROM doc_templates
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(template_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("fetch template");

    assert_eq!(row.0, template_id);
    assert_eq!(row.1, "work-order-v1");
    assert_eq!(row.2, "work_order");
    assert_eq!(row.3, body_template);
    assert_eq!(row.4, 1);
}

// ── Template version uniqueness ─────────────────────────────────────

#[tokio::test]
async fn template_unique_name_version_per_tenant() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();

    insert_test_template(
        &pool,
        tenant_id,
        "dup-template",
        "spec",
        serde_json::json!({}),
    )
    .await;

    // Same name + version + tenant should fail
    let result = sqlx::query(
        "INSERT INTO doc_templates (id, tenant_id, name, doc_type, body_template, version, created_by, created_at, updated_at)
         VALUES ($1, $2, 'dup-template', 'spec', '{}'::jsonb, 1, $3, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert!(result.is_err(), "duplicate template name+version should fail");
}

#[tokio::test]
async fn template_same_name_different_version_allowed() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();

    insert_test_template(
        &pool,
        tenant_id,
        "versioned-tmpl",
        "spec",
        serde_json::json!({"v": 1}),
    )
    .await;

    // Version 2 of same name should succeed
    let result = sqlx::query(
        "INSERT INTO doc_templates (id, tenant_id, name, doc_type, body_template, version, created_by, created_at, updated_at)
         VALUES ($1, $2, 'versioned-tmpl', 'spec', '{\"v\": 2}'::jsonb, 2, $3, now(), now())",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert!(result.is_ok(), "different version of same template should succeed");
}

// ── Render pipeline: template → artifact ────────────────────────────

#[tokio::test]
async fn render_produces_artifact_with_hashes() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();

    let body_template = serde_json::json!({
        "title": "WO-{{number}}",
        "body": "Priority: {{priority}}, Assigned: {{assignee}}"
    });

    let template_id = insert_test_template(
        &pool,
        tenant_id,
        "render-test-tmpl",
        "work_order",
        body_template,
    )
    .await;

    // Simulate render: apply template + insert artifact
    let input_data = serde_json::json!({
        "number": "1001",
        "priority": "HIGH",
        "assignee": "Alice"
    });

    let output = doc_mgmt::render::apply_template(
        &serde_json::json!({
            "title": "WO-{{number}}",
            "body": "Priority: {{priority}}, Assigned: {{assignee}}"
        }),
        &input_data,
    );

    let input_hash = doc_mgmt::render::compute_hash(&input_data);
    let output_hash = doc_mgmt::render::compute_hash(&output);

    let artifact_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO render_artifacts (id, tenant_id, template_id, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .bind(template_id)
    .bind(&input_hash)
    .bind(&output_hash)
    .bind(&output)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert artifact");

    // Query artifact back
    let row: (Uuid, String, String, serde_json::Value) = sqlx::query_as(
        "SELECT id, input_hash, output_hash, output FROM render_artifacts
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("fetch artifact");

    assert_eq!(row.0, artifact_id);
    assert_eq!(row.1, input_hash, "input hash persisted");
    assert_eq!(row.2, output_hash, "output hash persisted");
    assert_eq!(row.3["title"], "WO-1001");
    assert_eq!(row.3["body"], "Priority: HIGH, Assigned: Alice");
}

// ── Deterministic rendering: same inputs → same output hash ─────────

#[tokio::test]
async fn render_deterministic_same_inputs_same_hash() {
    let template_body = serde_json::json!({
        "doc": "{{doc_id}}",
        "rev": "{{rev}}"
    });
    let input = serde_json::json!({
        "doc_id": "DOC-42",
        "rev": "3"
    });

    let output1 = doc_mgmt::render::apply_template(&template_body, &input);
    let output2 = doc_mgmt::render::apply_template(&template_body, &input);

    let hash1 = doc_mgmt::render::compute_hash(&output1);
    let hash2 = doc_mgmt::render::compute_hash(&output2);

    assert_eq!(hash1, hash2, "same inputs must produce same output hash");
    assert_eq!(output1, output2, "same inputs must produce identical output");
}

// ── Idempotency: duplicate render key rejected ──────────────────────

#[tokio::test]
async fn render_idempotency_key_prevents_duplicates() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();

    let template_id = insert_test_template(
        &pool,
        tenant_id,
        "idem-tmpl",
        "spec",
        serde_json::json!({"x": "{{val}}"}),
    )
    .await;

    let idem_key = format!("render-{}", Uuid::new_v4());
    let now = Utc::now();

    // First render
    sqlx::query(
        "INSERT INTO render_artifacts (id, tenant_id, template_id, idempotency_key, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, $4, 'h1', 'h2', '{}'::jsonb, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(template_id)
    .bind(&idem_key)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await
    .expect("first render");

    // Duplicate idempotency key should fail
    let result = sqlx::query(
        "INSERT INTO render_artifacts (id, tenant_id, template_id, idempotency_key, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, $4, 'h1', 'h2', '{}'::jsonb, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(template_id)
    .bind(&idem_key)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await;

    assert!(result.is_err(), "duplicate idempotency key must be rejected");
}

// ── Render emits outbox event ───────────────────────────────────────

#[tokio::test]
async fn render_writes_outbox_event() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();

    let template_id = insert_test_template(
        &pool,
        tenant_id,
        "outbox-tmpl",
        "work_order",
        serde_json::json!({"label": "{{name}}"}),
    )
    .await;

    let input_data = serde_json::json!({"name": "Test"});
    let output = doc_mgmt::render::apply_template(
        &serde_json::json!({"label": "{{name}}"}),
        &input_data,
    );
    let input_hash = doc_mgmt::render::compute_hash(&input_data);
    let output_hash = doc_mgmt::render::compute_hash(&output);

    let artifact_id = Uuid::new_v4();
    let now = Utc::now();

    // Simulate the atomic transaction: artifact + outbox
    let mut tx = pool.begin().await.expect("begin tx");

    sqlx::query(
        "INSERT INTO render_artifacts (id, tenant_id, template_id, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .bind(template_id)
    .bind(&input_hash)
    .bind(&output_hash)
    .bind(&output)
    .bind(actor)
    .bind(now)
    .execute(&mut *tx)
    .await
    .expect("insert artifact");

    // Build and insert outbox event
    let envelope = platform_contracts::EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.rendered".to_string(),
        serde_json::json!({
            "artifact_id": artifact_id,
            "template_id": template_id,
            "output_hash": output_hash,
        }),
    )
    .with_mutation_class(Some(platform_contracts::mutation_classes::DATA_MUTATION.to_string()))
    .with_actor(actor, "User".to_string());

    let event_payload = event_bus::outbox::validate_and_serialize_envelope(&envelope)
        .expect("serialize envelope");
    let subject = platform_contracts::event_naming::nats_subject("doc_mgmt", "document.rendered");

    sqlx::query(
        "INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)",
    )
    .bind("document.rendered")
    .bind(&subject)
    .bind(&event_payload)
    .execute(&mut *tx)
    .await
    .expect("outbox insert");

    tx.commit().await.expect("commit");

    // Verify outbox event exists with correct payload
    let outbox_payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM doc_outbox WHERE event_type = 'document.rendered' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("fetch outbox event");

    assert_eq!(outbox_payload["event_type"], "document.rendered");
    assert_eq!(outbox_payload["source_module"], "doc_mgmt");
    assert!(outbox_payload["event_id"].as_str().is_some(), "event_id present");
    assert!(outbox_payload["tenant_id"].as_str().is_some(), "tenant_id present");
    assert!(outbox_payload["mutation_class"].as_str().is_some(), "mutation_class present");
}

// ── Full E2E: create template → render → query artifact ─────────────

#[tokio::test]
async fn template_full_e2e_create_render_query() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let actor = Uuid::new_v4();

    // 1. Create template
    let body_template = serde_json::json!({
        "work_order": {
            "number": "WO-{{wo_number}}",
            "description": "{{description}}",
            "parts": ["{{part1}}", "{{part2}}"]
        }
    });

    let template_id = insert_test_template(
        &pool,
        tenant_id,
        "full-e2e-tmpl",
        "work_order",
        body_template.clone(),
    )
    .await;

    // 2. Render with sample payload
    let input_data = serde_json::json!({
        "wo_number": "2024-001",
        "description": "Replace turbine blade",
        "part1": "TB-100A",
        "part2": "BOLT-M8"
    });

    let output = doc_mgmt::render::apply_template(&body_template, &input_data);
    let input_hash = doc_mgmt::render::compute_hash(&input_data);
    let output_hash = doc_mgmt::render::compute_hash(&output);

    let artifact_id = Uuid::new_v4();
    let idem_key = format!("e2e-render-{}", Uuid::new_v4());
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO render_artifacts (id, tenant_id, template_id, idempotency_key, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .bind(template_id)
    .bind(&idem_key)
    .bind(&input_hash)
    .bind(&output_hash)
    .bind(&output)
    .bind(actor)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert artifact");

    // 3. Query artifact back — verify content
    let row: (Uuid, Uuid, String, String, serde_json::Value) = sqlx::query_as(
        "SELECT id, template_id, input_hash, output_hash, output FROM render_artifacts
         WHERE id = $1 AND tenant_id = $2",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .fetch_one(&pool)
    .await
    .expect("fetch artifact");

    assert_eq!(row.0, artifact_id);
    assert_eq!(row.1, template_id);
    assert_eq!(row.2, input_hash);
    assert_eq!(row.3, output_hash);

    // Verify rendered output correctness
    let wo = &row.4["work_order"];
    assert_eq!(wo["number"], "WO-2024-001");
    assert_eq!(wo["description"], "Replace turbine blade");
    assert_eq!(wo["parts"][0], "TB-100A");
    assert_eq!(wo["parts"][1], "BOLT-M8");

    // 4. Verify idempotency key is queryable
    let idem_artifact: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM render_artifacts WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(&idem_key)
    .fetch_optional(&pool)
    .await
    .expect("idem query");

    assert_eq!(idem_artifact, Some(artifact_id), "artifact queryable by idempotency key");
}

// ── Tenant isolation: different tenants see different templates ──────

#[tokio::test]
async fn template_tenant_isolation() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();

    insert_test_template(
        &pool,
        tenant_a,
        "isolated-tmpl",
        "spec",
        serde_json::json!({}),
    )
    .await;

    // Tenant B should not see tenant A's template
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM doc_templates WHERE tenant_id = $1 AND name = 'isolated-tmpl'",
    )
    .bind(tenant_b)
    .fetch_one(&pool)
    .await
    .expect("count");

    assert_eq!(count, 0, "tenant B must not see tenant A's template");
}

// ── Artifact FK: artifact must reference valid template ─────────────

#[tokio::test]
async fn artifact_requires_valid_template() {
    let pool = get_pool().await;
    let tenant_id = Uuid::new_v4();
    let fake_template = Uuid::new_v4();

    let result = sqlx::query(
        "INSERT INTO render_artifacts (id, tenant_id, template_id, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, 'h', 'h', '{}'::jsonb, $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(tenant_id)
    .bind(fake_template)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await;

    assert!(result.is_err(), "artifact with non-existent template must fail FK check");
}
