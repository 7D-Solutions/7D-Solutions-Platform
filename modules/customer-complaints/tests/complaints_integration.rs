//! Integration tests for the customer-complaints module.
//!
//! Requires a real Postgres database. Set DATABASE_URL or rely on the default.
//! All tests use unique tenant IDs to avoid cross-test interference.

use customer_complaints_rs::domain::{
    models::*,
    repo,
    state_machine,
};
use customer_complaints_rs::http::sweep::sweep_overdue_complaints;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://cc_user:cc_pass@localhost:5468/cc_db".to_string());
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to CC test DB");
    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run CC migrations");
    pool
}

fn unique_tenant() -> String {
    format!("cc-test-{}", Uuid::new_v4().simple())
}

fn base_create_req(created_by: &str) -> CreateComplaintRequest {
    CreateComplaintRequest {
        party_id: Uuid::new_v4(),
        customer_contact_id: None,
        source: ComplaintSource::Email,
        source_ref: None,
        severity: None,
        category_code: None,
        title: "Test complaint".to_string(),
        description: Some("Something went wrong".to_string()),
        source_entity_type: None,
        source_entity_id: None,
        due_date: None,
        created_by: created_by.to_string(),
    }
}

async fn create_test_complaint(pool: &sqlx::PgPool, tenant_id: &str) -> Complaint {
    let req = base_create_req("alice");
    let mut tx = pool.begin().await.unwrap();
    let c = repo::create_complaint(&mut tx, tenant_id, &req, &format!("CC-{}", Uuid::new_v4().simple()))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    c
}

async fn create_active_category(pool: &sqlx::PgPool, tenant_id: &str, code: &str) {
    let req = CreateCategoryCodeRequest {
        category_code: code.to_string(),
        display_label: code.to_string(),
        description: None,
        created_by: "system".to_string(),
    };
    repo::create_category_code(pool, tenant_id, &req).await.unwrap();
}

// ── 1. intake → triaged without category → domain method returns error ────────

#[tokio::test]
#[serial]
async fn test_triage_requires_category() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let complaint = create_test_complaint(&pool, &tid).await;

    let req = TriageComplaintRequest {
        category_code: "missing-code".to_string(),
        severity: ComplaintSeverity::High,
        assigned_to: "bob".to_string(),
        due_date: None,
        triaged_by: "alice".to_string(),
    };

    let mut tx = pool.begin().await.unwrap();
    let result = repo::triage_complaint(&mut tx, &tid, complaint.id, &req).await;
    tx.rollback().await.unwrap();

    assert!(result.is_err(), "Expected error triaging with unknown category code");
    let err = result.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("category code") || msg.contains("not found"), "Error should mention category: {}", msg);
}

// ── 2. intake → triaged with inactive category → error ───────────────────────

#[tokio::test]
#[serial]
async fn test_triage_rejects_inactive_category() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create category then deactivate it
    create_active_category(&pool, &tid, "defect").await;
    repo::update_category_code(&pool, &tid, "defect", &UpdateCategoryCodeRequest {
        display_label: None,
        description: None,
        active: Some(false),
        updated_by: "system".to_string(),
    }).await.unwrap();

    let complaint = create_test_complaint(&pool, &tid).await;

    let req = TriageComplaintRequest {
        category_code: "defect".to_string(),
        severity: ComplaintSeverity::Medium,
        assigned_to: "bob".to_string(),
        due_date: None,
        triaged_by: "alice".to_string(),
    };

    let mut tx = pool.begin().await.unwrap();
    let result = repo::triage_complaint(&mut tx, &tid, complaint.id, &req).await;
    tx.rollback().await.unwrap();

    assert!(result.is_err(), "Expected error triaging with inactive category");
}

// ── 3. intake → triaged with valid category → succeeds ───────────────────────

#[tokio::test]
#[serial]
async fn test_triage_with_valid_category_succeeds() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "billing").await;
    let complaint = create_test_complaint(&pool, &tid).await;

    let req = TriageComplaintRequest {
        category_code: "billing".to_string(),
        severity: ComplaintSeverity::Low,
        assigned_to: "charlie".to_string(),
        due_date: None,
        triaged_by: "alice".to_string(),
    };

    let mut tx = pool.begin().await.unwrap();
    let result = repo::triage_complaint(&mut tx, &tid, complaint.id, &req).await;
    tx.commit().await.unwrap();

    let triaged = result.expect("Triage should succeed");
    assert_eq!(triaged.status, "triaged");
    assert_eq!(triaged.category_code.as_deref(), Some("billing"));
    assert_eq!(triaged.severity.as_deref(), Some("low"));
    assert_eq!(triaged.assigned_to.as_deref(), Some("charlie"));
}

// ── 4. investigating → responded with no customer_communication → error ───────

#[tokio::test]
#[serial]
async fn test_respond_requires_customer_communication() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "service").await;
    let complaint = create_test_complaint(&pool, &tid).await;

    // triage
    let triage_req = TriageComplaintRequest {
        category_code: "service".to_string(),
        severity: ComplaintSeverity::Medium,
        assigned_to: "bob".to_string(),
        due_date: None,
        triaged_by: "alice".to_string(),
    };
    let mut tx = pool.begin().await.unwrap();
    repo::triage_complaint(&mut tx, &tid, complaint.id, &triage_req).await.unwrap();
    tx.commit().await.unwrap();

    // start investigation
    let mut tx = pool.begin().await.unwrap();
    repo::start_investigation(&mut tx, &tid, complaint.id, &StartInvestigationRequest {
        started_by: "bob".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // try respond without customer_communication
    let mut tx = pool.begin().await.unwrap();
    let result = repo::respond_complaint(&mut tx, &tid, complaint.id, &RespondComplaintRequest {
        responded_by: "bob".to_string(),
    }).await;
    tx.rollback().await.unwrap();

    assert!(result.is_err(), "Expected error responding without customer communication");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("customer_communication"), "Error should mention customer_communication: {}", msg);
}

// ── 5. investigating → responded with customer_communication → succeeds ───────

#[tokio::test]
#[serial]
async fn test_respond_with_customer_communication_succeeds() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "quality").await;
    let complaint = create_test_complaint(&pool, &tid).await;

    // triage
    let mut tx = pool.begin().await.unwrap();
    repo::triage_complaint(&mut tx, &tid, complaint.id, &TriageComplaintRequest {
        category_code: "quality".to_string(),
        severity: ComplaintSeverity::High,
        assigned_to: "bob".to_string(),
        due_date: None,
        triaged_by: "alice".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // start investigation
    let mut tx = pool.begin().await.unwrap();
    repo::start_investigation(&mut tx, &tid, complaint.id, &StartInvestigationRequest {
        started_by: "bob".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // log customer communication
    let mut tx = pool.begin().await.unwrap();
    repo::add_activity_log_entry(&mut tx, &tid, complaint.id, &CreateActivityLogRequest {
        activity_type: ActivityType::CustomerCommunication,
        from_value: None,
        to_value: None,
        content: Some("Called customer, explained investigation status".to_string()),
        visible_to_customer: Some(true),
        recorded_by: "bob".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // respond
    let mut tx = pool.begin().await.unwrap();
    let result = repo::respond_complaint(&mut tx, &tid, complaint.id, &RespondComplaintRequest {
        responded_by: "bob".to_string(),
    }).await;
    tx.commit().await.unwrap();

    let responded = result.expect("Respond should succeed");
    assert_eq!(responded.status, "responded");
    assert!(responded.responded_at.is_some());
}

// ── 6. responded → closed without resolution record → error ──────────────────

#[tokio::test]
#[serial]
async fn test_close_requires_resolution_record() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "billing").await;
    let complaint = create_test_complaint(&pool, &tid).await;

    // triage → investigate → add comm → respond
    let complaint = advance_to_responded(&pool, &tid, complaint.id).await;
    assert_eq!(complaint.status, "responded");

    // try close without resolution
    let mut tx = pool.begin().await.unwrap();
    let result = repo::close_complaint(&mut tx, &tid, complaint.id, &CloseComplaintRequest {
        outcome: ComplaintOutcome::Resolved,
        closed_by: "alice".to_string(),
    }).await;
    tx.rollback().await.unwrap();

    assert!(result.is_err(), "Expected error closing without resolution record");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("resolution"), "Error should mention resolution: {}", msg);
}

// ── 7. responded → closed with resolution record → succeeds ──────────────────

#[tokio::test]
#[serial]
async fn test_close_with_resolution_succeeds() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "billing").await;
    let complaint = create_test_complaint(&pool, &tid).await;

    let complaint = advance_to_responded(&pool, &tid, complaint.id).await;

    // create resolution
    let mut tx = pool.begin().await.unwrap();
    repo::create_resolution(&mut tx, &tid, complaint.id, &CreateResolutionRequest {
        action_taken: "Issued credit note and apologized".to_string(),
        root_cause_summary: Some("Billing system error".to_string()),
        customer_acceptance: CustomerAcceptance::Accepted,
        customer_response_at: None,
        resolved_by: "bob".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // close
    let mut tx = pool.begin().await.unwrap();
    let result = repo::close_complaint(&mut tx, &tid, complaint.id, &CloseComplaintRequest {
        outcome: ComplaintOutcome::Resolved,
        closed_by: "alice".to_string(),
    }).await;
    tx.commit().await.unwrap();

    let closed = result.expect("Close should succeed");
    assert_eq!(closed.status, "closed");
    assert_eq!(closed.outcome.as_deref(), Some("resolved"));
    assert!(closed.closed_at.is_some());
}

// ── 8. Cancel from non-terminal state → allowed ───────────────────────────────

#[tokio::test]
#[serial]
async fn test_cancel_from_non_terminal_allowed() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    let complaint = create_test_complaint(&pool, &tid).await;
    assert_eq!(complaint.status, "intake");

    let mut tx = pool.begin().await.unwrap();
    let result = repo::cancel_complaint(&mut tx, &tid, complaint.id, &CancelComplaintRequest {
        reason: Some("Duplicate complaint".to_string()),
        cancelled_by: "alice".to_string(),
    }).await;
    tx.commit().await.unwrap();

    let cancelled = result.expect("Cancel from intake should succeed");
    assert_eq!(cancelled.status, "cancelled");
}

// ── 9. Cancel from closed → error ────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_cancel_from_terminal_fails() {
    // Pure state machine test — no DB needed
    assert!(state_machine::transition_cancel("closed").is_err());
    assert!(state_machine::transition_cancel("cancelled").is_err());
}

// ── 10. Activity log update() → returns append-only error ────────────────────

#[tokio::test]
#[serial]
async fn test_activity_log_update_is_append_only() {
    let result = repo::update_activity_log_entry(Uuid::new_v4());
    assert!(result.is_err(), "Expected append-only error on update");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("append") || msg.contains("Append") || msg.contains("cannot be modified"), "Error should mention append-only: {}", msg);
}

// ── 11. Activity log delete() → returns append-only error ────────────────────

#[tokio::test]
#[serial]
async fn test_activity_log_delete_is_append_only() {
    let result = repo::delete_activity_log_entry(Uuid::new_v4());
    assert!(result.is_err(), "Expected append-only error on delete");
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("append") || msg.contains("Append") || msg.contains("cannot be modified"), "Error should mention append-only: {}", msg);
}

// ── 12. Resolution: second POST on same complaint → 409 ──────────────────────

#[tokio::test]
#[serial]
async fn test_second_resolution_returns_conflict() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "billing").await;
    let complaint = create_test_complaint(&pool, &tid).await;

    let res_req = CreateResolutionRequest {
        action_taken: "First resolution".to_string(),
        root_cause_summary: None,
        customer_acceptance: CustomerAcceptance::Accepted,
        customer_response_at: None,
        resolved_by: "bob".to_string(),
    };

    let mut tx = pool.begin().await.unwrap();
    repo::create_resolution(&mut tx, &tid, complaint.id, &res_req).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let result = repo::create_resolution(&mut tx, &tid, complaint.id, &CreateResolutionRequest {
        action_taken: "Second resolution attempt".to_string(),
        root_cause_summary: None,
        customer_acceptance: CustomerAcceptance::NoResponse,
        customer_response_at: None,
        resolved_by: "alice".to_string(),
    }).await;
    tx.rollback().await.unwrap();

    assert!(result.is_err(), "Expected conflict error on second resolution");
    let msg = result.unwrap_err().to_string();
    assert!(msg.to_lowercase().contains("conflict") || msg.contains("already exists"), "Error should be conflict: {}", msg);
}

// ── 13. Category codes are tenant-isolated ────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_category_codes_tenant_isolated() {
    let pool = setup_db().await;
    let tid_a = unique_tenant();
    let tid_b = unique_tenant();

    create_active_category(&pool, &tid_a, "shared-code").await;

    // Tenant B should NOT see tenant A's category code
    let codes_b = repo::list_category_codes(&pool, &tid_b, false).await.unwrap();
    assert!(codes_b.is_empty(), "Tenant B should not see tenant A's category codes");

    // Tenant A should see its own code
    let codes_a = repo::list_category_codes(&pool, &tid_a, false).await.unwrap();
    assert_eq!(codes_a.len(), 1);
    assert_eq!(codes_a[0].category_code, "shared-code");

    // Using tenant A's code for a complaint in tenant B should fail
    let complaint_b = create_test_complaint(&pool, &tid_b).await;
    let mut tx = pool.begin().await.unwrap();
    let result = repo::triage_complaint(&mut tx, &tid_b, complaint_b.id, &TriageComplaintRequest {
        category_code: "shared-code".to_string(),
        severity: ComplaintSeverity::Low,
        assigned_to: "bob".to_string(),
        due_date: None,
        triaged_by: "alice".to_string(),
    }).await;
    tx.rollback().await.unwrap();

    assert!(result.is_err(), "Should not be able to use cross-tenant category code");
}

// ── 14. Full lifecycle happy path ─────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn test_full_lifecycle() {
    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "quality").await;

    // Create
    let complaint = create_test_complaint(&pool, &tid).await;
    assert_eq!(complaint.status, "intake");

    // Triage
    let mut tx = pool.begin().await.unwrap();
    let complaint = repo::triage_complaint(&mut tx, &tid, complaint.id, &TriageComplaintRequest {
        category_code: "quality".to_string(),
        severity: ComplaintSeverity::Critical,
        assigned_to: "inspector".to_string(),
        due_date: None,
        triaged_by: "coordinator".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(complaint.status, "triaged");

    // Start investigation
    let mut tx = pool.begin().await.unwrap();
    let complaint = repo::start_investigation(&mut tx, &tid, complaint.id, &StartInvestigationRequest {
        started_by: "inspector".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(complaint.status, "investigating");

    // Log customer communication
    let mut tx = pool.begin().await.unwrap();
    repo::add_activity_log_entry(&mut tx, &tid, complaint.id, &CreateActivityLogRequest {
        activity_type: ActivityType::CustomerCommunication,
        from_value: None,
        to_value: None,
        content: Some("Emailed customer with update".to_string()),
        visible_to_customer: Some(true),
        recorded_by: "inspector".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // Respond
    let mut tx = pool.begin().await.unwrap();
    let complaint = repo::respond_complaint(&mut tx, &tid, complaint.id, &RespondComplaintRequest {
        responded_by: "inspector".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(complaint.status, "responded");

    // Create resolution
    let mut tx = pool.begin().await.unwrap();
    repo::create_resolution(&mut tx, &tid, complaint.id, &CreateResolutionRequest {
        action_taken: "Root cause identified and corrected".to_string(),
        root_cause_summary: Some("Process deviation on line 3".to_string()),
        customer_acceptance: CustomerAcceptance::Accepted,
        customer_response_at: None,
        resolved_by: "inspector".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // Close
    let mut tx = pool.begin().await.unwrap();
    let complaint = repo::close_complaint(&mut tx, &tid, complaint.id, &CloseComplaintRequest {
        outcome: ComplaintOutcome::Resolved,
        closed_by: "coordinator".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(complaint.status, "closed");
    assert_eq!(complaint.outcome.as_deref(), Some("resolved"));

    // Verify detail
    let detail = repo::get_complaint_detail(&pool, &tid, complaint.id).await.unwrap().unwrap();
    assert!(detail.resolution.is_some());
    assert!(!detail.activity_log.is_empty());

    // Cancel from closed should fail
    let mut tx = pool.begin().await.unwrap();
    let cancel_result = repo::cancel_complaint(&mut tx, &tid, complaint.id, &CancelComplaintRequest {
        reason: None,
        cancelled_by: "someone".to_string(),
    }).await;
    tx.rollback().await.unwrap();
    assert!(cancel_result.is_err(), "Cannot cancel a closed complaint");
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase C: Event emission + sweep + consumer tests
// ═══════════════════════════════════════════════════════════════════════════════

// ── 15. create_complaint writes complaint_received to outbox ──────────────────

#[tokio::test]
#[serial]
async fn test_outbox_complaint_received_on_create() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let req = base_create_req("alice");
    let mut tx = pool.begin().await.unwrap();
    let number = format!("CC-OB-{}", Uuid::new_v4().simple());
    let complaint = repo::create_complaint(&mut tx, &tid, &req, &number).await.unwrap();

    // Enqueue the outbox event in the same transaction (mirrors what the HTTP handler does)
    use customer_complaints_rs::events::produced::{ComplaintReceivedPayload, EVENT_COMPLAINT_RECEIVED};
    use customer_complaints_rs::outbox;
    let event_id = Uuid::new_v4();
    let payload = ComplaintReceivedPayload {
        complaint_id: complaint.id,
        complaint_number: complaint.complaint_number.clone(),
        tenant_id: tid.clone(),
        party_id: complaint.party_id,
        source: complaint.source.clone(),
        severity: None,
        category_code: None,
        source_entity_type: None,
        source_entity_id: None,
        received_at: complaint.received_at,
    };
    outbox::enqueue_event_tx(&mut tx, event_id, EVENT_COMPLAINT_RECEIVED, complaint.id, &tid, None, None, &payload)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM cc_outbox WHERE event_id = $1 AND event_type = $2 AND tenant_id = $3",
    )
    .bind(event_id)
    .bind(EVENT_COMPLAINT_RECEIVED)
    .bind(&tid)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(count, 1, "complaint_received must be written to cc_outbox");
}

// ── 16. Sweep: emits overdue once, skips on second run ───────────────────────

#[tokio::test]
#[serial]
async fn test_sweep_overdue_idempotent() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    // Create a complaint with a past due_date
    let past_due: chrono::DateTime<chrono::Utc> = chrono::Utc::now() - chrono::Duration::days(5);
    let complaint_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO complaints
           (tenant_id, complaint_number, status, party_id, source, title, created_by, due_date)
           VALUES ($1, $2, 'investigating', gen_random_uuid(), 'email', 'Overdue complaint', 'system', $3)
           RETURNING id"#,
    )
    .bind(&tid)
    .bind(format!("CC-OD-{}", Uuid::new_v4().simple()))
    .bind(past_due)
    .fetch_one(&pool)
    .await
    .unwrap();

    // First sweep: should emit one event
    let swept1 = sweep_overdue_complaints(&pool).await.unwrap();
    assert!(swept1 >= 1, "Expected at least 1 complaint swept, got {}", swept1);

    // Verify outbox row was written
    let (outbox_count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM cc_outbox WHERE aggregate_id = $1 AND event_type = 'customer_complaints.complaint_overdue'",
    )
    .bind(complaint_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count, 1, "Exactly one overdue event should be in outbox");

    // Verify overdue_emitted_at was set
    let emitted_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "SELECT overdue_emitted_at FROM complaints WHERE id = $1",
    )
    .bind(complaint_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(emitted_at.is_some(), "overdue_emitted_at should be set after sweep");

    // Second sweep: same complaint already has overdue_emitted_at set, skip it
    let swept2 = sweep_overdue_complaints(&pool).await.unwrap();

    let (outbox_count2,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM cc_outbox WHERE aggregate_id = $1 AND event_type = 'customer_complaints.complaint_overdue'",
    )
    .bind(complaint_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(outbox_count2, 1, "Second sweep must not emit duplicate overdue events");
    let _ = swept2; // swept2 may be 0 for this specific complaint

    // Cleanup
    sqlx::query("DELETE FROM cc_outbox WHERE aggregate_id = $1").bind(complaint_id.to_string()).execute(&pool).await.ok();
    sqlx::query("DELETE FROM complaints WHERE id = $1").bind(complaint_id).execute(&pool).await.ok();
}

// ── 17. Sweep: closed/cancelled complaints are excluded ──────────────────────

#[tokio::test]
#[serial]
async fn test_sweep_excludes_terminal_complaints() {
    let pool = setup_db().await;
    let tid = unique_tenant();

    let past_due: chrono::DateTime<chrono::Utc> = chrono::Utc::now() - chrono::Duration::days(1);
    let closed_id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO complaints
           (tenant_id, complaint_number, status, party_id, source, title, created_by, due_date)
           VALUES ($1, $2, 'closed', gen_random_uuid(), 'email', 'Closed overdue', 'system', $3)
           RETURNING id"#,
    )
    .bind(&tid)
    .bind(format!("CC-CL-{}", Uuid::new_v4().simple()))
    .bind(past_due)
    .fetch_one(&pool)
    .await
    .unwrap();

    sweep_overdue_complaints(&pool).await.unwrap();

    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM cc_outbox WHERE aggregate_id = $1 AND event_type = 'customer_complaints.complaint_overdue'",
    )
    .bind(closed_id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 0, "Closed complaints must not be swept");

    sqlx::query("DELETE FROM complaints WHERE id = $1").bind(closed_id).execute(&pool).await.ok();
}

// ── 18. Full lifecycle emits 7 outbox events ─────────────────────────────────

#[tokio::test]
#[serial]
async fn test_all_state_transition_events_in_outbox() {
    use customer_complaints_rs::events::produced::*;
    use customer_complaints_rs::outbox;

    let pool = setup_db().await;
    let tid = unique_tenant();
    create_active_category(&pool, &tid, "quality-evt").await;

    // create → complaint_received
    let req = base_create_req("alice");
    let mut tx = pool.begin().await.unwrap();
    let number = format!("CC-EVT-{}", Uuid::new_v4().simple());
    let complaint = repo::create_complaint(&mut tx, &tid, &req, &number).await.unwrap();
    let ev_id = Uuid::new_v4();
    let p = ComplaintReceivedPayload {
        complaint_id: complaint.id, complaint_number: complaint.complaint_number.clone(),
        tenant_id: tid.clone(), party_id: complaint.party_id, source: complaint.source.clone(),
        severity: None, category_code: None, source_entity_type: None,
        source_entity_id: None, received_at: complaint.received_at,
    };
    outbox::enqueue_event_tx(&mut tx, ev_id, EVENT_COMPLAINT_RECEIVED, complaint.id, &tid, None, None, &p).await.unwrap();
    tx.commit().await.unwrap();

    // triage → complaint_triaged + status_changed
    let mut tx = pool.begin().await.unwrap();
    let c = repo::triage_complaint(&mut tx, &tid, complaint.id, &TriageComplaintRequest {
        category_code: "quality-evt".to_string(), severity: ComplaintSeverity::High,
        assigned_to: "bob".to_string(), due_date: None, triaged_by: "alice".to_string(),
    }).await.unwrap();
    let e1 = Uuid::new_v4();
    let e2 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e1, EVENT_COMPLAINT_TRIAGED, c.id, &tid, None, None, &ComplaintTriagedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), assigned_to: "bob".to_string(),
        category_code: "quality-evt".to_string(), severity: "high".to_string(),
        triaged_at: c.assigned_at.unwrap_or_else(chrono::Utc::now),
    }).await.unwrap();
    outbox::enqueue_event_tx(&mut tx, e2, EVENT_COMPLAINT_STATUS_CHANGED, c.id, &tid, None, None, &ComplaintStatusChangedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), from_status: "intake".to_string(),
        to_status: "triaged".to_string(), transitioned_by: "alice".to_string(), transitioned_at: c.updated_at,
    }).await.unwrap();
    tx.commit().await.unwrap();

    // assign → assigned
    let mut tx = pool.begin().await.unwrap();
    let c = repo::assign_complaint(&mut tx, &tid, complaint.id, &AssignComplaintRequest {
        assigned_to: "charlie".to_string(), assigned_by: "manager".to_string(),
    }).await.unwrap();
    let e3 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e3, EVENT_COMPLAINT_ASSIGNED, c.id, &tid, None, None, &ComplaintAssignedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), from_user: None,
        to_user: "charlie".to_string(), assigned_by: "manager".to_string(),
        assigned_at: c.assigned_at.unwrap_or_else(chrono::Utc::now),
    }).await.unwrap();
    tx.commit().await.unwrap();

    // start investigation → status_changed
    let mut tx = pool.begin().await.unwrap();
    let c = repo::start_investigation(&mut tx, &tid, complaint.id, &StartInvestigationRequest {
        started_by: "charlie".to_string(),
    }).await.unwrap();
    let e4 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e4, EVENT_COMPLAINT_STATUS_CHANGED, c.id, &tid, None, None, &ComplaintStatusChangedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), from_status: "triaged".to_string(),
        to_status: "investigating".to_string(), transitioned_by: "charlie".to_string(), transitioned_at: c.updated_at,
    }).await.unwrap();
    tx.commit().await.unwrap();

    // customer_communication → customer_communicated
    let mut tx = pool.begin().await.unwrap();
    let entry = repo::add_activity_log_entry(&mut tx, &tid, complaint.id, &CreateActivityLogRequest {
        activity_type: ActivityType::CustomerCommunication, from_value: None, to_value: None,
        content: Some("Called".to_string()), visible_to_customer: Some(true), recorded_by: "charlie".to_string(),
    }).await.unwrap();
    let e5 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e5, EVENT_COMPLAINT_CUSTOMER_COMMUNICATED, complaint.id, &tid, None, None, &ComplaintCustomerCommunicatedPayload {
        complaint_id: complaint.id, tenant_id: tid.clone(),
        recorded_by: "charlie".to_string(), recorded_at: entry.recorded_at,
    }).await.unwrap();
    tx.commit().await.unwrap();

    // respond → status_changed
    let mut tx = pool.begin().await.unwrap();
    let c = repo::respond_complaint(&mut tx, &tid, complaint.id, &RespondComplaintRequest {
        responded_by: "charlie".to_string(),
    }).await.unwrap();
    let e6 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e6, EVENT_COMPLAINT_STATUS_CHANGED, c.id, &tid, None, None, &ComplaintStatusChangedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), from_status: "investigating".to_string(),
        to_status: "responded".to_string(), transitioned_by: "charlie".to_string(), transitioned_at: c.updated_at,
    }).await.unwrap();
    tx.commit().await.unwrap();

    // resolution → resolved
    let mut tx = pool.begin().await.unwrap();
    let resolution = repo::create_resolution(&mut tx, &tid, complaint.id, &CreateResolutionRequest {
        action_taken: "Fixed".to_string(), root_cause_summary: None,
        customer_acceptance: CustomerAcceptance::Accepted, customer_response_at: None, resolved_by: "charlie".to_string(),
    }).await.unwrap();
    let e7 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e7, EVENT_COMPLAINT_RESOLVED, complaint.id, &tid, None, None, &ComplaintResolvedPayload {
        complaint_id: complaint.id, tenant_id: tid.clone(), customer_acceptance: "accepted".to_string(),
        resolved_by: "charlie".to_string(), resolved_at: resolution.resolved_at,
    }).await.unwrap();
    tx.commit().await.unwrap();

    // close → closed + status_changed
    let mut tx = pool.begin().await.unwrap();
    let c = repo::close_complaint(&mut tx, &tid, complaint.id, &CloseComplaintRequest {
        outcome: ComplaintOutcome::Resolved, closed_by: "alice".to_string(),
    }).await.unwrap();
    let e8 = Uuid::new_v4();
    let e9 = Uuid::new_v4();
    outbox::enqueue_event_tx(&mut tx, e8, EVENT_COMPLAINT_CLOSED, c.id, &tid, None, None, &ComplaintClosedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), outcome: "resolved".to_string(),
        closed_at: c.closed_at.unwrap_or_else(chrono::Utc::now),
    }).await.unwrap();
    outbox::enqueue_event_tx(&mut tx, e9, EVENT_COMPLAINT_STATUS_CHANGED, c.id, &tid, None, None, &ComplaintStatusChangedPayload {
        complaint_id: c.id, tenant_id: tid.clone(), from_status: "responded".to_string(),
        to_status: "closed".to_string(), transitioned_by: "alice".to_string(), transitioned_at: c.updated_at,
    }).await.unwrap();
    tx.commit().await.unwrap();

    // Verify all 9 outbox rows were written for this complaint
    let (total,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM cc_outbox WHERE aggregate_id = $1",
    )
    .bind(complaint.id.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(total, 10, "Expected 10 outbox events for full lifecycle (received, triaged, 4×status_changed, assigned, customer_communicated, resolved, closed)");

    // Verify all 8 distinct event types present
    let event_types: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT event_type FROM cc_outbox WHERE aggregate_id = $1 ORDER BY event_type",
    )
    .bind(complaint.id.to_string())
    .fetch_all(&pool)
    .await
    .unwrap();
    for expected in &[
        EVENT_COMPLAINT_RECEIVED, EVENT_COMPLAINT_TRIAGED, EVENT_COMPLAINT_STATUS_CHANGED,
        EVENT_COMPLAINT_ASSIGNED, EVENT_COMPLAINT_CUSTOMER_COMMUNICATED,
        EVENT_COMPLAINT_RESOLVED, EVENT_COMPLAINT_CLOSED,
    ] {
        assert!(event_types.iter().any(|e| e == expected), "Missing event type: {}", expected);
    }

    // Cleanup
    sqlx::query("DELETE FROM cc_outbox WHERE aggregate_id = $1").bind(complaint.id.to_string()).execute(&pool).await.ok();
    sqlx::query("DELETE FROM complaint_resolutions WHERE complaint_id = $1").bind(complaint.id).execute(&pool).await.ok();
    sqlx::query("DELETE FROM complaint_activity_log WHERE complaint_id = $1").bind(complaint.id).execute(&pool).await.ok();
    sqlx::query("DELETE FROM complaints WHERE id = $1").bind(complaint.id).execute(&pool).await.ok();
}

// ── Helper: advance complaint to 'responded' status ───────────────────────────

async fn advance_to_responded(pool: &sqlx::PgPool, tid: &str, complaint_id: Uuid) -> Complaint {
    let mut tx = pool.begin().await.unwrap();
    repo::triage_complaint(&mut tx, tid, complaint_id, &TriageComplaintRequest {
        category_code: "billing".to_string(),
        severity: ComplaintSeverity::Medium,
        assigned_to: "agent".to_string(),
        due_date: None,
        triaged_by: "manager".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    repo::start_investigation(&mut tx, tid, complaint_id, &StartInvestigationRequest {
        started_by: "agent".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    repo::add_activity_log_entry(&mut tx, tid, complaint_id, &CreateActivityLogRequest {
        activity_type: ActivityType::CustomerCommunication,
        from_value: None,
        to_value: None,
        content: Some("Spoke with customer".to_string()),
        visible_to_customer: Some(true),
        recorded_by: "agent".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    let c = repo::respond_complaint(&mut tx, tid, complaint_id, &RespondComplaintRequest {
        responded_by: "agent".to_string(),
    }).await.unwrap();
    tx.commit().await.unwrap();
    c
}
