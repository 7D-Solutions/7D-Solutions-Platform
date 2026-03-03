mod common;

use ar_rs::credit_notes::{
    approve_credit_memo, create_credit_memo, issue_credit_memo, ApproveCreditMemoRequest,
    CreateCreditMemoRequest, CreditNoteError, IssueCreditMemoRequest,
};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_invoice(pool: &PgPool, app_id: &str) -> (i32, i32) {
    let (customer_id, _, _) = common::seed_customer(pool, app_id).await;
    let invoice_id: i32 = sqlx::query_scalar(
        r#"
        INSERT INTO ar_invoices (
            app_id, tilled_invoice_id, ar_customer_id, status, amount_cents, currency,
            created_at, updated_at
        ) VALUES ($1, $2, $3, 'open', 10000, 'usd', NOW(), NOW())
        RETURNING id
        "#,
    )
    .bind(app_id)
    .bind(format!("inv_{}", Uuid::new_v4()))
    .bind(customer_id)
    .fetch_one(pool)
    .await
    .expect("seed invoice");
    (customer_id, invoice_id)
}

#[tokio::test]
async fn credit_memo_lifecycle_create_approve_issue_with_idempotency() {
    let pool = common::setup_pool().await;
    let app_id = "tenant-credit-memo-lifecycle";
    let (customer_id, invoice_id) = seed_invoice(&pool, app_id).await;
    let credit_note_id = Uuid::new_v4();
    let create_key = Uuid::new_v4();
    let issue_key = Uuid::new_v4();

    let created = create_credit_memo(
        &pool,
        CreateCreditMemoRequest {
            credit_note_id,
            app_id: app_id.to_string(),
            customer_id: customer_id.to_string(),
            invoice_id,
            amount_minor: 2500,
            currency: "usd".to_string(),
            reason: "rma_disposition".to_string(),
            reference_id: Some("rma-100".to_string()),
            created_by: Some("qa@tenant.local".to_string()),
            create_idempotency_key: create_key,
            correlation_id: "corr-create-1".to_string(),
            causation_id: Some("rma.received.100".to_string()),
        },
    )
    .await
    .expect("create credit memo");
    assert!(matches!(
        created,
        ar_rs::credit_notes::CreateCreditMemoResult::Created { .. }
    ));

    let status_after_create: String =
        sqlx::query_scalar("SELECT status FROM ar_credit_notes WHERE credit_note_id = $1")
            .bind(credit_note_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status after create");
    assert_eq!(status_after_create, "draft");

    let created_again = create_credit_memo(
        &pool,
        CreateCreditMemoRequest {
            credit_note_id: Uuid::new_v4(),
            app_id: app_id.to_string(),
            customer_id: customer_id.to_string(),
            invoice_id,
            amount_minor: 2500,
            currency: "usd".to_string(),
            reason: "rma_disposition".to_string(),
            reference_id: Some("rma-100".to_string()),
            created_by: Some("qa@tenant.local".to_string()),
            create_idempotency_key: create_key,
            correlation_id: "corr-create-1".to_string(),
            causation_id: Some("rma.received.100".to_string()),
        },
    )
    .await
    .expect("idempotent create");
    assert!(matches!(
        created_again,
        ar_rs::credit_notes::CreateCreditMemoResult::AlreadyProcessed { .. }
    ));

    let approved = approve_credit_memo(
        &pool,
        ApproveCreditMemoRequest {
            app_id: app_id.to_string(),
            credit_note_id,
            approved_by: Some("finance@tenant.local".to_string()),
            correlation_id: "corr-approve-1".to_string(),
            causation_id: Some("rma.review.approved".to_string()),
        },
    )
    .await
    .expect("approve credit memo");
    assert!(matches!(
        approved,
        ar_rs::credit_notes::ApproveCreditMemoResult::Approved { .. }
    ));

    let status_after_approve: String =
        sqlx::query_scalar("SELECT status FROM ar_credit_notes WHERE credit_note_id = $1")
            .bind(credit_note_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status after approve");
    assert_eq!(status_after_approve, "approved");

    let issued = issue_credit_memo(
        &pool,
        IssueCreditMemoRequest {
            app_id: app_id.to_string(),
            credit_note_id,
            issued_by: Some("finance@tenant.local".to_string()),
            issue_idempotency_key: issue_key,
            correlation_id: "corr-issue-1".to_string(),
            causation_id: Some("rma.credit.authorized".to_string()),
        },
    )
    .await
    .expect("issue credit memo");
    assert!(matches!(
        issued,
        ar_rs::credit_notes::IssueCreditMemoResult::Issued { .. }
    ));

    let issued_again = issue_credit_memo(
        &pool,
        IssueCreditMemoRequest {
            app_id: app_id.to_string(),
            credit_note_id,
            issued_by: Some("finance@tenant.local".to_string()),
            issue_idempotency_key: issue_key,
            correlation_id: "corr-issue-1".to_string(),
            causation_id: Some("rma.credit.authorized".to_string()),
        },
    )
    .await
    .expect("idempotent issue");
    assert!(matches!(
        issued_again,
        ar_rs::credit_notes::IssueCreditMemoResult::AlreadyProcessed { .. }
    ));

    let status_after_issue: String =
        sqlx::query_scalar("SELECT status FROM ar_credit_notes WHERE credit_note_id = $1")
            .bind(credit_note_id)
            .fetch_one(&pool)
            .await
            .expect("fetch status after issue");
    assert_eq!(status_after_issue, "issued");

    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events_outbox
         WHERE aggregate_id = $1
           AND event_type IN ('ar.credit_memo_created', 'ar.credit_memo_approved', 'ar.credit_note_issued')",
    )
    .bind(credit_note_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count outbox events");
    assert_eq!(event_count, 3, "create/approve/issue should emit 3 events");
}

#[tokio::test]
async fn credit_memo_tenant_scope_enforced() {
    let pool = common::setup_pool().await;
    let app_a = "tenant-credit-memo-a";
    let (_customer_id, invoice_id) = seed_invoice(&pool, app_a).await;
    let credit_note_id = Uuid::new_v4();

    create_credit_memo(
        &pool,
        CreateCreditMemoRequest {
            credit_note_id,
            app_id: app_a.to_string(),
            customer_id: "cust-a".to_string(),
            invoice_id,
            amount_minor: 1000,
            currency: "usd".to_string(),
            reason: "rma_disposition".to_string(),
            reference_id: Some("rma-a".to_string()),
            created_by: Some("ops@tenant-a.local".to_string()),
            create_idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-a".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("create memo tenant a");

    let err = approve_credit_memo(
        &pool,
        ApproveCreditMemoRequest {
            app_id: "tenant-credit-memo-b".to_string(),
            credit_note_id,
            approved_by: Some("ops@tenant-b.local".to_string()),
            correlation_id: "corr-b".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect_err("tenant b should not be able to approve tenant a memo");

    assert!(matches!(err, CreditNoteError::CreditMemoNotFound { .. }));
}
