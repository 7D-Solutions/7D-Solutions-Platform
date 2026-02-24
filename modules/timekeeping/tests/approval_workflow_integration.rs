//! Integration tests for approval workflow (bd-3l2v).
//!
//! State machine: draft → submitted → approved / rejected
//!                           ↑ recall ↓
//!                          draft
//!
//! Covers:
//! 1. Submit approval — happy path
//! 2. Approve approval — submitted → approved
//! 3. Reject approval — submitted → rejected
//! 4. Recall approval — submitted → draft
//! 5. Double-approve rejected (approve an already-approved request)
//! 6. Approve a non-submitted (draft) request rejected
//! 7. Tenant isolation — approval invisible across app boundaries

use chrono::NaiveDate;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use timekeeping::domain::approvals::models::{
    ApprovalError, ApprovalStatus, RecallApprovalRequest, ReviewApprovalRequest,
    SubmitApprovalRequest,
};
use timekeeping::domain::approvals::service::{approve, get_approval, recall, reject, submit};
use timekeeping::domain::employees::models::CreateEmployeeRequest;
use timekeeping::domain::employees::service::EmployeeRepo;
use uuid::Uuid;

async fn setup_db() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://timekeeping_user:timekeeping_pass@localhost:5447/timekeeping_db".to_string()
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to timekeeping test DB");

    sqlx::migrate!("db/migrations")
        .run(&pool)
        .await
        .expect("Failed to run timekeeping migrations");

    pool
}

fn unique_app() -> String {
    format!("appr-test-{}", Uuid::new_v4().simple())
}

fn period_start() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()
}

fn period_end() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 2, 7).unwrap()
}

async fn create_test_employee(pool: &sqlx::PgPool, app_id: &str) -> Uuid {
    let emp = EmployeeRepo::create(
        pool,
        &CreateEmployeeRequest {
            app_id: app_id.to_string(),
            employee_code: format!("AE-{}", Uuid::new_v4().simple()),
            first_name: "Approval".to_string(),
            last_name: "Tester".to_string(),
            email: None,
            department: None,
            external_payroll_id: None,
            hourly_rate_minor: None,
            currency: None,
        },
    )
    .await
    .unwrap();
    emp.id
}

fn submit_req(app_id: &str, emp_id: Uuid, actor_id: Uuid) -> SubmitApprovalRequest {
    SubmitApprovalRequest {
        app_id: app_id.to_string(),
        employee_id: emp_id,
        period_start: period_start(),
        period_end: period_end(),
        actor_id,
    }
}

fn review_req(app_id: &str, approval_id: Uuid, actor_id: Uuid) -> ReviewApprovalRequest {
    ReviewApprovalRequest {
        app_id: app_id.to_string(),
        approval_id,
        actor_id,
        notes: None,
    }
}

// ============================================================================
// 1. Submit approval — happy path
// ============================================================================

#[tokio::test]
#[serial]
async fn test_submit_approval() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor_id = Uuid::new_v4();

    let approval = submit(&pool, &submit_req(&app_id, emp_id, actor_id))
        .await
        .unwrap();

    assert_eq!(approval.app_id, app_id);
    assert_eq!(approval.employee_id, emp_id);
    assert_eq!(approval.status, ApprovalStatus::Submitted);
    assert!(approval.submitted_at.is_some());

    // Verify outbox event
    let event: Option<(String,)> = sqlx::query_as(
        "SELECT event_type FROM events_outbox WHERE aggregate_id = $1",
    )
    .bind(approval.id.to_string())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(event.unwrap().0, "timesheet.submitted");
}

// ============================================================================
// 2. Approve approval — submitted → approved
// ============================================================================

#[tokio::test]
#[serial]
async fn test_approve_approval() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor_id = Uuid::new_v4();
    let reviewer_id = Uuid::new_v4();

    let submitted = submit(&pool, &submit_req(&app_id, emp_id, actor_id))
        .await
        .unwrap();

    let approved = approve(&pool, &review_req(&app_id, submitted.id, reviewer_id))
        .await
        .unwrap();

    assert_eq!(approved.status, ApprovalStatus::Approved);
    assert!(approved.reviewed_at.is_some());
    assert_eq!(approved.reviewer_id, Some(reviewer_id));

    // Fetch from DB to confirm persisted
    let fetched = get_approval(&pool, &app_id, approved.id).await.unwrap();
    assert_eq!(fetched.status, ApprovalStatus::Approved);
}

// ============================================================================
// 3. Reject approval — submitted → rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_reject_approval() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor_id = Uuid::new_v4();
    let reviewer_id = Uuid::new_v4();

    let submitted = submit(&pool, &submit_req(&app_id, emp_id, actor_id))
        .await
        .unwrap();

    let rejected = reject(
        &pool,
        &ReviewApprovalRequest {
            app_id: app_id.clone(),
            approval_id: submitted.id,
            actor_id: reviewer_id,
            notes: Some("Missing time entries".to_string()),
        },
    )
    .await
    .unwrap();

    assert_eq!(rejected.status, ApprovalStatus::Rejected);
    assert_eq!(
        rejected.reviewer_notes.as_deref(),
        Some("Missing time entries")
    );
}

// ============================================================================
// 4. Recall approval — submitted → draft
// ============================================================================

#[tokio::test]
#[serial]
async fn test_recall_approval() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor_id = Uuid::new_v4();

    let submitted = submit(&pool, &submit_req(&app_id, emp_id, actor_id))
        .await
        .unwrap();
    assert_eq!(submitted.status, ApprovalStatus::Submitted);

    let recalled = recall(
        &pool,
        &RecallApprovalRequest {
            app_id: app_id.clone(),
            approval_id: submitted.id,
            actor_id,
            notes: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(recalled.status, ApprovalStatus::Draft);
    assert!(recalled.submitted_at.is_none());
}

// ============================================================================
// 5. Double-approve rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_double_approve_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor_id = Uuid::new_v4();
    let reviewer_id = Uuid::new_v4();

    let submitted = submit(&pool, &submit_req(&app_id, emp_id, actor_id))
        .await
        .unwrap();

    // First approval succeeds
    approve(&pool, &review_req(&app_id, submitted.id, reviewer_id))
        .await
        .unwrap();

    // Second approval must fail (already approved, not submitted)
    let err = approve(&pool, &review_req(&app_id, submitted.id, reviewer_id))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ApprovalError::InvalidTransition { .. }),
        "Expected InvalidTransition on double-approve, got: {err:?}"
    );
}

// ============================================================================
// 6. Approve a draft (non-submitted) request rejected
// ============================================================================

#[tokio::test]
#[serial]
async fn test_approve_draft_rejected() {
    let pool = setup_db().await;
    let app_id = unique_app();
    let emp_id = create_test_employee(&pool, &app_id).await;
    let actor_id = Uuid::new_v4();
    let reviewer_id = Uuid::new_v4();

    // Submit then recall to get back to draft
    let submitted = submit(&pool, &submit_req(&app_id, emp_id, actor_id))
        .await
        .unwrap();
    let recalled = recall(
        &pool,
        &RecallApprovalRequest {
            app_id: app_id.clone(),
            approval_id: submitted.id,
            actor_id,
            notes: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(recalled.status, ApprovalStatus::Draft);

    // Approving a draft must fail
    let err = approve(&pool, &review_req(&app_id, submitted.id, reviewer_id))
        .await
        .unwrap_err();

    assert!(
        matches!(err, ApprovalError::InvalidTransition { .. }),
        "Expected InvalidTransition when approving a draft, got: {err:?}"
    );
}

// ============================================================================
// 7. Tenant isolation
// ============================================================================

#[tokio::test]
#[serial]
async fn test_approval_tenant_isolation() {
    let pool = setup_db().await;
    let app_a = unique_app();
    let app_b = unique_app();
    let emp_a = create_test_employee(&pool, &app_a).await;
    let actor_a = Uuid::new_v4();
    let actor_b = Uuid::new_v4();

    let approval_a = submit(&pool, &submit_req(&app_a, emp_a, actor_a))
        .await
        .unwrap();

    // App B cannot fetch app A's approval
    let err = get_approval(&pool, &app_b, approval_a.id)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ApprovalError::NotFound),
        "App B must not see app A's approval"
    );

    // App B cannot approve app A's approval
    let err = approve(
        &pool,
        &ReviewApprovalRequest {
            app_id: app_b.clone(),
            approval_id: approval_a.id,
            actor_id: actor_b,
            notes: None,
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, ApprovalError::NotFound),
        "App B must not be able to approve app A's approval"
    );
}
