//! External refs service — Guard→Mutation→Outbox atomicity.
//!
//! Operations:
//! - create_external_ref: upsert on (app_id, system, external_id), emit external_ref.created
//! - update_external_ref: update label/metadata, emit external_ref.updated
//! - delete_external_ref: hard delete, emit external_ref.deleted
//! - get_external_ref: fetch by id scoped to app_id
//! - list_by_entity: all refs for a given entity_type + entity_id
//! - get_by_external: lookup by system + external_id

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_external_ref_created_envelope, build_external_ref_deleted_envelope,
    build_external_ref_updated_envelope, ExternalRefCreatedPayload, ExternalRefDeletedPayload,
    ExternalRefUpdatedPayload, EVENT_TYPE_EXTERNAL_REF_CREATED, EVENT_TYPE_EXTERNAL_REF_DELETED,
    EVENT_TYPE_EXTERNAL_REF_UPDATED,
};
use crate::outbox::enqueue_event_tx;

use super::guards::{validate_create, validate_update};
use super::models::{
    CreateExternalRefRequest, ExternalRef, ExternalRefError, UpdateExternalRefRequest,
};

// ============================================================================
// Reads
// ============================================================================

/// Fetch a single external ref by id, scoped to app_id.
pub async fn get_external_ref(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
) -> Result<Option<ExternalRef>, ExternalRefError> {
    let row = sqlx::query_as::<_, ExternalRef>(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(ref_id)
    .bind(app_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// List all external refs for a given internal entity.
pub async fn list_by_entity(
    pool: &PgPool,
    app_id: &str,
    entity_type: &str,
    entity_id: &str,
) -> Result<Vec<ExternalRef>, ExternalRefError> {
    let rows = sqlx::query_as::<_, ExternalRef>(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE app_id = $1 AND entity_type = $2 AND entity_id = $3
        ORDER BY system, external_id
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .bind(entity_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Look up a ref by external system + external_id, scoped to app_id.
pub async fn get_by_external(
    pool: &PgPool,
    app_id: &str,
    system: &str,
    external_id: &str,
) -> Result<Option<ExternalRef>, ExternalRefError> {
    let row = sqlx::query_as::<_, ExternalRef>(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE app_id = $1 AND system = $2 AND external_id = $3
        "#,
    )
    .bind(app_id)
    .bind(system)
    .bind(external_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ============================================================================
// Writes
// ============================================================================

/// Create or update an external ref.
///
/// Idempotent: on (app_id, system, external_id) conflict, updates label and
/// metadata in place. The entity_type and entity_id of the existing mapping
/// are preserved — to remap an external ID to a different entity, delete and
/// recreate.
///
/// Emits `external_ref.created` via the transactional outbox.
pub async fn create_external_ref(
    pool: &PgPool,
    app_id: &str,
    req: &CreateExternalRefRequest,
    correlation_id: String,
) -> Result<ExternalRef, ExternalRefError> {
    validate_create(req)?;

    let event_id = Uuid::new_v4();

    let mut tx = pool.begin().await?;

    // Mutation: upsert
    let row: ExternalRef = sqlx::query_as(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, label, metadata,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO UPDATE SET
            label      = COALESCE(EXCLUDED.label, integrations_external_refs.label),
            metadata   = COALESCE(EXCLUDED.metadata, integrations_external_refs.metadata),
            updated_at = NOW()
        RETURNING id, app_id, entity_type, entity_id, system, external_id,
                  label, metadata, created_at, updated_at
        "#,
    )
    .bind(app_id)
    .bind(req.entity_type.trim())
    .bind(req.entity_id.trim())
    .bind(req.system.trim())
    .bind(req.external_id.trim())
    .bind(&req.label)
    .bind(&req.metadata)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: external_ref.created
    let payload = ExternalRefCreatedPayload {
        ref_id: row.id,
        app_id: app_id.to_string(),
        entity_type: row.entity_type.clone(),
        entity_id: row.entity_id.clone(),
        system: row.system.clone(),
        external_id: row.external_id.clone(),
        label: row.label.clone(),
        created_at: row.created_at,
    };
    let envelope = build_external_ref_created_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_EXTERNAL_REF_CREATED,
        "external_ref",
        &row.id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(row)
}

/// Update label and/or metadata on an existing external ref.
///
/// Guard: ref must exist and belong to app_id.
/// Emits `external_ref.updated` via the transactional outbox.
pub async fn update_external_ref(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
    req: &UpdateExternalRefRequest,
    correlation_id: String,
) -> Result<ExternalRef, ExternalRefError> {
    validate_update(req)?;

    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: fetch + lock
    let existing: Option<ExternalRef> = sqlx::query_as(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE id = $1 AND app_id = $2
        FOR UPDATE
        "#,
    )
    .bind(ref_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = existing.ok_or(ExternalRefError::NotFound(ref_id))?;

    let new_label = if req.label.is_some() { req.label.clone() } else { current.label.clone() };
    let new_meta =
        if req.metadata.is_some() { req.metadata.clone() } else { current.metadata.clone() };

    // Mutation
    let updated: ExternalRef = sqlx::query_as(
        r#"
        UPDATE integrations_external_refs
        SET label = $1, metadata = $2, updated_at = $3
        WHERE id = $4 AND app_id = $5
        RETURNING id, app_id, entity_type, entity_id, system, external_id,
                  label, metadata, created_at, updated_at
        "#,
    )
    .bind(&new_label)
    .bind(&new_meta)
    .bind(now)
    .bind(ref_id)
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?;

    // Outbox: external_ref.updated
    let payload = ExternalRefUpdatedPayload {
        ref_id: updated.id,
        app_id: app_id.to_string(),
        entity_type: updated.entity_type.clone(),
        entity_id: updated.entity_id.clone(),
        system: updated.system.clone(),
        external_id: updated.external_id.clone(),
        label: updated.label.clone(),
        updated_at: updated.updated_at,
    };
    let envelope = build_external_ref_updated_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_EXTERNAL_REF_UPDATED,
        "external_ref",
        &ref_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

/// Hard-delete an external ref.
///
/// Guard: ref must exist and belong to app_id.
/// Emits `external_ref.deleted` via the transactional outbox.
pub async fn delete_external_ref(
    pool: &PgPool,
    app_id: &str,
    ref_id: i64,
    correlation_id: String,
) -> Result<(), ExternalRefError> {
    let event_id = Uuid::new_v4();
    let now = Utc::now();

    let mut tx = pool.begin().await?;

    // Guard: verify existence
    let row: Option<ExternalRef> = sqlx::query_as(
        r#"
        SELECT id, app_id, entity_type, entity_id, system, external_id,
               label, metadata, created_at, updated_at
        FROM integrations_external_refs
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(ref_id)
    .bind(app_id)
    .fetch_optional(&mut *tx)
    .await?;

    let current = row.ok_or(ExternalRefError::NotFound(ref_id))?;

    // Mutation: delete
    sqlx::query("DELETE FROM integrations_external_refs WHERE id = $1 AND app_id = $2")
        .bind(ref_id)
        .bind(app_id)
        .execute(&mut *tx)
        .await?;

    // Outbox: external_ref.deleted
    let payload = ExternalRefDeletedPayload {
        ref_id: current.id,
        app_id: app_id.to_string(),
        entity_type: current.entity_type,
        entity_id: current.entity_id,
        system: current.system,
        external_id: current.external_id,
        deleted_at: now,
    };
    let envelope = build_external_ref_deleted_envelope(
        event_id,
        app_id.to_string(),
        correlation_id,
        None,
        payload,
    );
    enqueue_event_tx(
        &mut tx,
        event_id,
        EVENT_TYPE_EXTERNAL_REF_DELETED,
        "external_ref",
        &ref_id.to_string(),
        app_id,
        &envelope,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

// ============================================================================
// Integrated Tests (real DB)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    const TEST_APP: &str = "test-external-refs";

    fn test_db_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db"
                .to_string()
        })
    }

    async fn test_pool() -> PgPool {
        let pool = sqlx::PgPool::connect(&test_db_url())
            .await
            .expect("Failed to connect to integrations test database");
        sqlx::migrate!("./db/migrations")
            .run(&pool)
            .await
            .expect("Migrations failed");
        pool
    }

    async fn cleanup(pool: &PgPool) {
        sqlx::query(
            "DELETE FROM integrations_outbox WHERE app_id = $1",
        )
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
        sqlx::query(
            "DELETE FROM integrations_external_refs WHERE app_id = $1",
        )
        .bind(TEST_APP)
        .execute(pool)
        .await
        .ok();
    }

    fn sample_req(entity_type: &str, entity_id: &str, system: &str, ext_id: &str) -> CreateExternalRefRequest {
        CreateExternalRefRequest {
            entity_type: entity_type.to_string(),
            entity_id: entity_id.to_string(),
            system: system.to_string(),
            external_id: ext_id.to_string(),
            label: Some("Test Label".to_string()),
            metadata: None,
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_create_and_get() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("invoice", "inv-001", "stripe", "in_abc123");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
            .await
            .expect("create_external_ref failed");

        assert_eq!(created.app_id, TEST_APP);
        assert_eq!(created.entity_type, "invoice");
        assert_eq!(created.entity_id, "inv-001");
        assert_eq!(created.system, "stripe");
        assert_eq!(created.external_id, "in_abc123");
        assert_eq!(created.label.as_deref(), Some("Test Label"));

        let fetched = get_external_ref(&pool, TEST_APP, created.id)
            .await
            .expect("get_external_ref failed");
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().id, created.id);

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_idempotent_create() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("invoice", "inv-002", "stripe", "in_idem456");

        let first = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
            .await
            .expect("first create failed");

        // Same request again — should return same id (upsert, no error)
        let second = create_external_ref(&pool, TEST_APP, &req, "corr-2".to_string())
            .await
            .expect("second create failed");

        assert_eq!(first.id, second.id);
        assert_eq!(second.external_id, "in_idem456");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_list_by_entity() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let r1 = sample_req("customer", "cust-1", "stripe", "cus_111");
        let r2 = sample_req("customer", "cust-1", "quickbooks", "QB-222");
        let r3 = sample_req("customer", "cust-2", "stripe", "cus_999");

        create_external_ref(&pool, TEST_APP, &r1, "c1".to_string()).await.expect("create r1");
        create_external_ref(&pool, TEST_APP, &r2, "c2".to_string()).await.expect("create r2");
        create_external_ref(&pool, TEST_APP, &r3, "c3".to_string()).await.expect("create r3");

        let refs = list_by_entity(&pool, TEST_APP, "customer", "cust-1")
            .await
            .expect("list_by_entity failed");

        assert_eq!(refs.len(), 2);
        assert!(refs.iter().all(|r| r.entity_id == "cust-1"));

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_get_by_external() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("order", "ord-55", "salesforce", "SF-LEAD-789");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-x".to_string())
            .await
            .expect("create failed");

        let found = get_by_external(&pool, TEST_APP, "salesforce", "SF-LEAD-789")
            .await
            .expect("get_by_external failed");

        assert!(found.is_some());
        assert_eq!(found.unwrap().id, created.id);

        // Wrong app_id returns None
        let not_found = get_by_external(&pool, "other-app", "salesforce", "SF-LEAD-789")
            .await
            .expect("get_by_external failed");
        assert!(not_found.is_none());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_update() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("invoice", "inv-010", "xero", "XERO-INV-1");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
            .await
            .expect("create failed");

        let upd_req = UpdateExternalRefRequest {
            label: Some("Updated Label".to_string()),
            metadata: None,
        };
        let updated = update_external_ref(&pool, TEST_APP, created.id, &upd_req, "corr-2".to_string())
            .await
            .expect("update failed");

        assert_eq!(updated.label.as_deref(), Some("Updated Label"));
        assert_eq!(updated.external_id, "XERO-INV-1");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_delete() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("party", "party-42", "hubspot", "HS-CONTACT-42");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
            .await
            .expect("create failed");

        delete_external_ref(&pool, TEST_APP, created.id, "corr-2".to_string())
            .await
            .expect("delete failed");

        let gone = get_external_ref(&pool, TEST_APP, created.id)
            .await
            .expect("get after delete failed");
        assert!(gone.is_none());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_outbox_event_on_create() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("invoice", "inv-outbox", "stripe", "in_outbox_test");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-out".to_string())
            .await
            .expect("create failed");

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM integrations_outbox WHERE aggregate_type = 'external_ref' AND aggregate_id = $1 AND app_id = $2",
        )
        .bind(created.id.to_string())
        .bind(TEST_APP)
        .fetch_one(&pool)
        .await
        .expect("outbox query failed");

        assert!(count.0 >= 1, "expected outbox event for created ref");

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_tenant_isolation() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("invoice", "inv-iso", "stripe", "in_iso_999");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
            .await
            .expect("create failed");

        // Different app_id cannot see this ref
        let not_found = get_external_ref(&pool, "other-tenant", created.id)
            .await
            .expect("tenant isolation get failed");
        assert!(not_found.is_none());

        // Update from wrong tenant should fail
        let upd_req = UpdateExternalRefRequest { label: Some("Hacked".to_string()), metadata: None };
        let err = update_external_ref(&pool, "other-tenant", created.id, &upd_req, "c".to_string()).await;
        assert!(err.is_err());

        cleanup(&pool).await;
    }

    #[tokio::test]
    #[serial]
    async fn test_external_refs_delete_wrong_tenant_fails() {
        let pool = test_pool().await;
        cleanup(&pool).await;

        let req = sample_req("invoice", "inv-del-iso", "stripe", "in_del_iso");
        let created = create_external_ref(&pool, TEST_APP, &req, "corr-1".to_string())
            .await
            .expect("create failed");

        let err = delete_external_ref(&pool, "other-tenant", created.id, "corr-2".to_string()).await;
        assert!(err.is_err());

        // Still exists for correct tenant
        let still_there = get_external_ref(&pool, TEST_APP, created.id).await.expect("get failed");
        assert!(still_there.is_some());

        cleanup(&pool).await;
    }
}
