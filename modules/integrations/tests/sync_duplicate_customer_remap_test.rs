//! Integration tests for the duplicate-customer remap policy (bd-5cn7z).
//!
//! Invariant: a new QBO customer that shares normalized fields (email / phone /
//! tax_id) with an internally-mapped entity MUST NOT trigger auto-remap.
//! All remaps are explicit, via `execute_customer_remap`, and tombstone the old
//! mapping before installing the new one.
//!
//! Run:
//!   ./scripts/cargo-slot.sh test -p integrations-rs --test sync_duplicate_customer_remap_test

use std::time::Duration;

use integrations_rs::domain::sync::conflicts_repo::get_conflict;
use integrations_rs::domain::sync::resolve_customer::{
    execute_customer_remap, normalize_email, normalize_phone, normalize_tax_id,
    raise_creation_conflict, CustomerCreationConflictRequest, CustomerRemapError,
    CustomerRemapRequest,
};
use serde_json::json;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// ── DB setup ──────────────────────────────────────────────────────────────────

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    });
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(90))
        .connect(&url)
        .await
        .expect("Failed to connect to integrations test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    pool
}

fn app() -> String {
    format!("dcr-test-{}", Uuid::new_v4().simple())
}

async fn cleanup(pool: &sqlx::PgPool, app_id: &str) {
    let _ = sqlx::query("DELETE FROM integrations_sync_conflicts WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_external_refs WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
    let _ = sqlx::query("DELETE FROM integrations_outbox WHERE app_id = $1")
        .bind(app_id)
        .execute(pool)
        .await;
}

/// Seed an external_ref row with normalized fields stored in metadata.
async fn seed_external_ref(
    pool: &sqlx::PgPool,
    app_id: &str,
    entity_type: &str,
    entity_id: &str,
    system: &str,
    external_id: &str,
    normalized_email: Option<&str>,
    normalized_phone: Option<&str>,
    normalized_tax_id: Option<&str>,
) -> i64 {
    let metadata = json!({
        "normalized_email":  normalized_email,
        "normalized_phone":  normalized_phone,
        "normalized_tax_id": normalized_tax_id,
    });
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO integrations_external_refs
            (app_id, entity_type, entity_id, system, external_id, label, metadata,
             created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, 'active', $6, NOW(), NOW())
        ON CONFLICT (app_id, system, external_id) DO UPDATE SET
            metadata = EXCLUDED.metadata, updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(system)
    .bind(external_id)
    .bind(&metadata)
    .fetch_one(pool)
    .await
    .expect("seed_external_ref");
    row.0
}

// ── Normalization unit tests ──────────────────────────────────────────────────

#[test]
fn normalize_email_lowercases_and_trims() {
    assert_eq!(normalize_email("  FOO@BAR.COM  "), "foo@bar.com");
    assert_eq!(normalize_email("User@Example.org"), "user@example.org");
}

#[test]
fn normalize_phone_keeps_digits_only() {
    assert_eq!(normalize_phone("+1 (555) 867-5309"), "15558675309");
    assert_eq!(normalize_phone("555.867.5309"), "5558675309");
}

#[test]
fn normalize_tax_id_alphanumeric_uppercase() {
    assert_eq!(normalize_tax_id("12-3456789"), "123456789");
    assert_eq!(normalize_tax_id("ab-cd ef"), "ABCDEF");
}

// ── raise_creation_conflict ───────────────────────────────────────────────────

/// Happy path: stale external_ref exists + new external customer shares email.
/// Expect a pending creation conflict with candidate hints.
#[tokio::test]
#[serial]
async fn raise_creation_conflict_opens_pending_creation_conflict() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    let old_qbo_id = format!("qbo-{}", Uuid::new_v4().simple());

    seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &old_qbo_id,
        Some("alice@example.com"),
        None,
        None,
    )
    .await;

    let new_qbo_id = format!("qbo-{}", Uuid::new_v4().simple());
    let req = CustomerCreationConflictRequest {
        app_id: app_id.clone(),
        provider: "quickbooks".to_string(),
        entity_type: "customer".to_string(),
        entity_id: entity_id.clone(),
        new_external_id: new_qbo_id.clone(),
        normalized_email: Some(normalize_email("alice@example.com")),
        normalized_phone: None,
        normalized_tax_id: None,
        external_value: json!({ "Id": new_qbo_id, "DisplayName": "Alice Corp" }),
        internal_value: json!({ "entity_id": entity_id }),
    };

    let outcome = raise_creation_conflict(&pool, &req).await.expect("raise");

    assert_eq!(outcome.conflict.conflict_class, "creation");
    assert_eq!(outcome.conflict.status, "pending");
    assert_eq!(outcome.conflict.entity_id, entity_id);
    assert_eq!(outcome.conflict.detected_by, "customer_resolver");

    cleanup(&pool, &app_id).await;
}

/// Candidate hints contain the matching entity when email matches.
#[tokio::test]
#[serial]
async fn raise_creation_conflict_includes_candidate_hints_on_email_match() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &format!("qbo-{}", Uuid::new_v4().simple()),
        Some("bob@acme.com"),
        None,
        None,
    )
    .await;

    let req = CustomerCreationConflictRequest {
        app_id: app_id.clone(),
        provider: "quickbooks".to_string(),
        entity_type: "customer".to_string(),
        entity_id: entity_id.clone(),
        new_external_id: format!("qbo-{}", Uuid::new_v4().simple()),
        normalized_email: Some(normalize_email("bob@acme.com")),
        normalized_phone: None,
        normalized_tax_id: None,
        external_value: json!({ "Id": "new-qbo", "DisplayName": "Bob ACME" }),
        internal_value: json!({ "entity_id": entity_id }),
    };

    let outcome = raise_creation_conflict(&pool, &req).await.expect("raise");

    assert!(
        !outcome.candidates.is_empty(),
        "must have at least one candidate hint"
    );
    let hint = &outcome.candidates[0];
    assert_eq!(hint.entity_id, entity_id);
    assert!(hint.matched_on.contains(&"email".to_string()));

    // Verify candidate_hints are embedded in internal_value.
    let stored_iv = outcome
        .conflict
        .internal_value
        .as_ref()
        .expect("internal_value");
    assert!(
        stored_iv["candidate_hints"].is_array(),
        "internal_value must have candidate_hints array"
    );

    cleanup(&pool, &app_id).await;
}

/// When no normalized fields match, candidates list is empty — no guessing.
#[tokio::test]
#[serial]
async fn raise_creation_conflict_no_candidates_when_fields_diverge() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    // Seed ref with a different email.
    seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &format!("qbo-{}", Uuid::new_v4().simple()),
        Some("other@domain.com"),
        None,
        None,
    )
    .await;

    let req = CustomerCreationConflictRequest {
        app_id: app_id.clone(),
        provider: "quickbooks".to_string(),
        entity_type: "customer".to_string(),
        entity_id: entity_id.clone(),
        new_external_id: format!("qbo-{}", Uuid::new_v4().simple()),
        normalized_email: Some(normalize_email("newcomer@example.com")),
        normalized_phone: None,
        normalized_tax_id: None,
        external_value: json!({ "Id": "new-qbo", "DisplayName": "New Co" }),
        internal_value: json!({ "entity_id": entity_id }),
    };

    let outcome = raise_creation_conflict(&pool, &req).await.expect("raise");

    assert!(
        outcome.candidates.is_empty(),
        "no candidates must be returned when fields do not match; got: {:?}",
        outcome.candidates
    );

    cleanup(&pool, &app_id).await;
}

/// Candidate search matches on phone when email is absent.
#[tokio::test]
#[serial]
async fn raise_creation_conflict_candidate_hint_on_phone_match() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &format!("qbo-{}", Uuid::new_v4().simple()),
        None,
        Some("15558675309"),
        None,
    )
    .await;

    let req = CustomerCreationConflictRequest {
        app_id: app_id.clone(),
        provider: "quickbooks".to_string(),
        entity_type: "customer".to_string(),
        entity_id: entity_id.clone(),
        new_external_id: format!("qbo-{}", Uuid::new_v4().simple()),
        normalized_email: None,
        normalized_phone: Some(normalize_phone("+1 (555) 867-5309")),
        normalized_tax_id: None,
        external_value: json!({ "Id": "new-qbo" }),
        internal_value: json!({ "entity_id": entity_id }),
    };

    let outcome = raise_creation_conflict(&pool, &req).await.expect("raise");

    let phone_hint = outcome
        .candidates
        .iter()
        .find(|h| h.matched_on.contains(&"phone".to_string()));
    assert!(
        phone_hint.is_some(),
        "phone-matched candidate must be returned"
    );

    cleanup(&pool, &app_id).await;
}

// ── execute_customer_remap ────────────────────────────────────────────────────

/// Full remap flow: old ref is tombstoned, new ref is created, conflict resolved.
#[tokio::test]
#[serial]
async fn execute_remap_tombstones_old_and_creates_new_ref() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    let old_qbo_id = format!("qbo-old-{}", Uuid::new_v4().simple());
    let new_qbo_id = format!("qbo-new-{}", Uuid::new_v4().simple());

    let old_ref_id = seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &old_qbo_id,
        Some("charlie@example.com"),
        None,
        None,
    )
    .await;

    // Raise creation conflict first.
    let conflict_outcome = raise_creation_conflict(
        &pool,
        &CustomerCreationConflictRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".to_string(),
            entity_type: "customer".to_string(),
            entity_id: entity_id.clone(),
            new_external_id: new_qbo_id.clone(),
            normalized_email: Some(normalize_email("charlie@example.com")),
            normalized_phone: None,
            normalized_tax_id: None,
            external_value: json!({ "Id": new_qbo_id }),
            internal_value: json!({ "entity_id": entity_id }),
        },
    )
    .await
    .expect("raise");

    // Execute the remap.
    let remap = execute_customer_remap(
        &pool,
        &CustomerRemapRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".to_string(),
            entity_type: "customer".to_string(),
            conflict_id: conflict_outcome.conflict.id,
            old_ref_id,
            new_external_id: new_qbo_id.clone(),
            resolved_by: "operator".to_string(),
            resolution_note: Some("remapped to deduped QBO customer".to_string()),
        },
    )
    .await
    .expect("remap");

    // Conflict must be resolved.
    assert_eq!(remap.resolved_conflict.status, "resolved");

    // Old ref must be tombstoned (label = 'tombstoned', metadata has tombstoned=true).
    let old_ref: Option<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT label, metadata FROM integrations_external_refs WHERE id = $1 AND app_id = $2",
    )
    .bind(old_ref_id)
    .bind(&app_id)
    .fetch_optional(&pool)
    .await
    .expect("fetch old ref");

    let (label, meta) = old_ref.expect("old ref must still exist (tombstoned, not deleted)");
    assert_eq!(label, "tombstoned");
    assert_eq!(
        meta["tombstoned"].as_bool(),
        Some(true),
        "metadata must have tombstoned=true"
    );
    assert_eq!(
        meta["remapped_to"].as_str(),
        Some(new_qbo_id.as_str()),
        "metadata must record remapped_to target"
    );

    // New ref must exist for entity → new_qbo_id.
    let new_ref: Option<(String,)> = sqlx::query_as(
        "SELECT entity_id FROM integrations_external_refs
         WHERE app_id = $1 AND system = $2 AND external_id = $3",
    )
    .bind(&app_id)
    .bind("quickbooks")
    .bind(&new_qbo_id)
    .fetch_optional(&pool)
    .await
    .expect("fetch new ref");

    let (mapped_entity_id,) = new_ref.expect("new external_ref must exist");
    assert_eq!(mapped_entity_id, entity_id);

    cleanup(&pool, &app_id).await;
}

/// Remap resolves the conflict and records audit fields.
#[tokio::test]
#[serial]
async fn execute_remap_resolves_conflict_with_audit_fields() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    let old_qbo_id = format!("qbo-{}", Uuid::new_v4().simple());
    let new_qbo_id = format!("qbo-{}", Uuid::new_v4().simple());

    let old_ref_id = seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &old_qbo_id,
        None,
        None,
        Some("123456789"),
    )
    .await;

    let conflict_outcome = raise_creation_conflict(
        &pool,
        &CustomerCreationConflictRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".to_string(),
            entity_type: "customer".to_string(),
            entity_id: entity_id.clone(),
            new_external_id: new_qbo_id.clone(),
            normalized_email: None,
            normalized_phone: None,
            normalized_tax_id: Some(normalize_tax_id("12-3456789")),
            external_value: json!({ "Id": new_qbo_id }),
            internal_value: json!({}),
        },
    )
    .await
    .expect("raise");

    let remap = execute_customer_remap(
        &pool,
        &CustomerRemapRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".to_string(),
            entity_type: "customer".to_string(),
            conflict_id: conflict_outcome.conflict.id,
            old_ref_id,
            new_external_id: new_qbo_id.clone(),
            resolved_by: "admin-user".to_string(),
            resolution_note: Some("tax_id match confirmed".to_string()),
        },
    )
    .await
    .expect("remap");

    assert_eq!(remap.resolved_conflict.status, "resolved");
    assert_eq!(
        remap.resolved_conflict.resolved_by.as_deref(),
        Some("admin-user")
    );
    assert!(remap.resolved_conflict.resolved_at.is_some());
    assert_eq!(
        remap.resolved_conflict.resolution_note.as_deref(),
        Some("tax_id match confirmed")
    );

    // Verify via independent DB read.
    let fetched = get_conflict(&pool, &app_id, conflict_outcome.conflict.id)
        .await
        .expect("get")
        .expect("must exist");
    assert_eq!(fetched.status, "resolved");
    assert_eq!(fetched.internal_id.as_deref(), Some(entity_id.as_str()));

    cleanup(&pool, &app_id).await;
}

/// Remap must be rejected when the conflict class is not `creation`.
#[tokio::test]
#[serial]
async fn execute_remap_blocked_if_conflict_not_creation_class() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    // Create an edit-class conflict directly.
    let edit_conflict: (uuid::Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO integrations_sync_conflicts
            (app_id, provider, entity_type, entity_id, conflict_class, detected_by,
             internal_value, external_value)
        VALUES ($1, 'quickbooks', 'customer', 'e1', 'edit', 'test',
                '{"v":1}'::jsonb, '{"v":2}'::jsonb)
        RETURNING id
        "#,
    )
    .bind(&app_id)
    .fetch_one(&pool)
    .await
    .expect("create edit conflict");

    let old_ref_id = seed_external_ref(
        &pool,
        &app_id,
        "customer",
        "e1",
        "quickbooks",
        "qbo-stale",
        None,
        None,
        None,
    )
    .await;

    let result = execute_customer_remap(
        &pool,
        &CustomerRemapRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".to_string(),
            entity_type: "customer".to_string(),
            conflict_id: edit_conflict.0,
            old_ref_id,
            new_external_id: "qbo-new".to_string(),
            resolved_by: "op".to_string(),
            resolution_note: None,
        },
    )
    .await;

    assert!(
        matches!(result, Err(CustomerRemapError::InvalidConflictState(_, _))),
        "must reject remap on non-creation conflict; got: {:?}",
        result
    );

    cleanup(&pool, &app_id).await;
}

/// Remap must be rejected when the conflict is already resolved.
#[tokio::test]
#[serial]
async fn execute_remap_blocked_if_conflict_already_resolved() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    let old_qbo_id = format!("qbo-{}", Uuid::new_v4().simple());
    let new_qbo_id = format!("qbo-{}", Uuid::new_v4().simple());

    let old_ref_id = seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &old_qbo_id,
        Some("dave@example.com"),
        None,
        None,
    )
    .await;

    let conflict_outcome = raise_creation_conflict(
        &pool,
        &CustomerCreationConflictRequest {
            app_id: app_id.clone(),
            provider: "quickbooks".to_string(),
            entity_type: "customer".to_string(),
            entity_id: entity_id.clone(),
            new_external_id: new_qbo_id.clone(),
            normalized_email: Some(normalize_email("dave@example.com")),
            normalized_phone: None,
            normalized_tax_id: None,
            external_value: json!({ "Id": new_qbo_id }),
            internal_value: json!({}),
        },
    )
    .await
    .expect("raise");

    let remap_req = CustomerRemapRequest {
        app_id: app_id.clone(),
        provider: "quickbooks".to_string(),
        entity_type: "customer".to_string(),
        conflict_id: conflict_outcome.conflict.id,
        old_ref_id,
        new_external_id: new_qbo_id.clone(),
        resolved_by: "op".to_string(),
        resolution_note: None,
    };

    // First remap succeeds.
    execute_customer_remap(&pool, &remap_req)
        .await
        .expect("first remap");

    // Second remap on the same already-resolved conflict must fail.
    let result = execute_customer_remap(&pool, &remap_req).await;
    assert!(
        matches!(
            result,
            Err(CustomerRemapError::InvalidConflictState(_, _))
                | Err(CustomerRemapError::ConflictNotFound)
        ),
        "second remap must be rejected; got: {:?}",
        result
    );

    cleanup(&pool, &app_id).await;
}

/// Tombstoned external_refs are excluded from candidate hints.
#[tokio::test]
#[serial]
async fn tombstoned_refs_excluded_from_candidate_hints() {
    let pool = setup_db().await;
    let app_id = app();
    cleanup(&pool, &app_id).await;

    let entity_id = format!("cust-{}", Uuid::new_v4().simple());
    let ref_id = seed_external_ref(
        &pool,
        &app_id,
        "customer",
        &entity_id,
        "quickbooks",
        &format!("qbo-{}", Uuid::new_v4().simple()),
        Some("eve@example.com"),
        None,
        None,
    )
    .await;

    // Manually tombstone the ref.
    sqlx::query(
        "UPDATE integrations_external_refs
         SET metadata = metadata || '{\"tombstoned\": true}'::jsonb, label = 'tombstoned'
         WHERE id = $1",
    )
    .bind(ref_id)
    .execute(&pool)
    .await
    .expect("tombstone");

    let req = CustomerCreationConflictRequest {
        app_id: app_id.clone(),
        provider: "quickbooks".to_string(),
        entity_type: "customer".to_string(),
        entity_id: entity_id.clone(),
        new_external_id: format!("qbo-{}", Uuid::new_v4().simple()),
        normalized_email: Some(normalize_email("eve@example.com")),
        normalized_phone: None,
        normalized_tax_id: None,
        external_value: json!({ "Id": "new" }),
        internal_value: json!({}),
    };

    let outcome = raise_creation_conflict(&pool, &req).await.expect("raise");

    assert!(
        outcome.candidates.is_empty(),
        "tombstoned ref must not appear as a candidate hint"
    );

    cleanup(&pool, &app_id).await;
}
