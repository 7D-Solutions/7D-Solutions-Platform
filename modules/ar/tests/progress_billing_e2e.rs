mod common;

use ar_rs::progress_billing::{
    add_milestone, bill_milestone, create_contract, AddMilestoneRequest, BillMilestoneRequest,
    BillMilestoneResult, CreateContractRequest, ProgressBillingError,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Helper: create a contract with milestones and a seeded customer.
async fn setup_contract_with_milestones(
    pool: &PgPool,
    app_id: &str,
    total: i64,
    milestones: &[(Uuid, &str, i32, i64)],
) -> Uuid {
    let contract_id = Uuid::new_v4();
    let (_customer_id, _email, _ext) = common::seed_customer(pool, app_id).await;

    create_contract(
        pool,
        CreateContractRequest {
            contract_id,
            app_id: app_id.to_string(),
            customer_id: "cust-pb".to_string(),
            description: "Test contract".to_string(),
            total_amount_minor: total,
            currency: "usd".to_string(),
        },
    )
    .await
    .expect("create contract");

    for &(milestone_id, name, pct, amt) in milestones {
        add_milestone(
            pool,
            AddMilestoneRequest {
                milestone_id,
                app_id: app_id.to_string(),
                contract_id,
                name: name.to_string(),
                percentage: pct,
                amount_minor: amt,
            },
        )
        .await
        .expect("add milestone");
    }

    contract_id
}

// ============================================================================
// 1. Milestone invoice E2E
// ============================================================================

#[tokio::test]
async fn milestone_invoice_e2e() {
    let pool = common::setup_pool().await;
    let app_id = "tenant-pb-e2e";
    let m1 = Uuid::new_v4();
    let contract_id = setup_contract_with_milestones(
        &pool,
        app_id,
        100000, // $1000 contract
        &[(m1, "Foundation", 30, 30000)],
    )
    .await;

    let result = bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m1,
            idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-pb-e2e".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("bill milestone");

    match result {
        BillMilestoneResult::Invoiced {
            invoice_id,
            milestone_id,
            amount_minor,
            ..
        } => {
            assert!(invoice_id > 0);
            assert_eq!(milestone_id, m1);
            assert_eq!(amount_minor, 30000);

            // Verify invoice was created in ar_invoices
            let inv_amount: i64 = sqlx::query_scalar(
                "SELECT amount_cents FROM ar_invoices WHERE id = $1 AND app_id = $2",
            )
            .bind(invoice_id)
            .bind(app_id)
            .fetch_one(&pool)
            .await
            .expect("fetch invoice");
            assert_eq!(inv_amount, 30000);

            // Verify milestone reference in invoice metadata
            let metadata: serde_json::Value =
                sqlx::query_scalar("SELECT metadata FROM ar_invoices WHERE id = $1")
                    .bind(invoice_id)
                    .fetch_one(&pool)
                    .await
                    .expect("fetch metadata");
            assert_eq!(metadata["milestone_name"], "Foundation");
        }
        _ => panic!("Expected Invoiced result"),
    }
}

// ============================================================================
// 2. Progress billing E2E (30% + 50%)
// ============================================================================

#[tokio::test]
async fn progress_billing_cumulative() {
    let pool = common::setup_pool().await;
    let app_id = "tenant-pb-cumulative";
    let m1 = Uuid::new_v4();
    let m2 = Uuid::new_v4();
    let contract_id = setup_contract_with_milestones(
        &pool,
        app_id,
        100000, // $1000 contract
        &[
            (m1, "Phase 1 - 30%", 30, 30000),
            (m2, "Phase 2 - 50%", 50, 50000),
        ],
    )
    .await;

    // Bill 30%
    let r1 = bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m1,
            idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-cumul-1".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("bill 30%");

    match &r1 {
        BillMilestoneResult::Invoiced {
            amount_minor,
            cumulative_billed_minor,
            ..
        } => {
            assert_eq!(*amount_minor, 30000);
            assert_eq!(*cumulative_billed_minor, 30000);
        }
        _ => panic!("Expected Invoiced"),
    }

    // Bill 50%
    let r2 = bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m2,
            idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-cumul-2".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("bill 50%");

    match &r2 {
        BillMilestoneResult::Invoiced {
            amount_minor,
            cumulative_billed_minor,
            ..
        } => {
            assert_eq!(*amount_minor, 50000);
            assert_eq!(*cumulative_billed_minor, 80000); // 30k + 50k
        }
        _ => panic!("Expected Invoiced"),
    }

    // Verify two invoices exist for this specific contract
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1 AND metadata->>'contract_id' = $2",
    )
    .bind(app_id)
    .bind(contract_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count invoices");
    assert_eq!(count, 2);
}

// ============================================================================
// 3. Over-billing guard test
// ============================================================================

#[tokio::test]
async fn over_billing_guard() {
    let pool = common::setup_pool().await;
    let app_id = "tenant-pb-overbill";
    let m1 = Uuid::new_v4();
    let m2 = Uuid::new_v4();
    let contract_id = setup_contract_with_milestones(
        &pool,
        app_id,
        50000, // $500 contract
        &[
            (m1, "Phase 1", 60, 30000),
            (m2, "Phase 2", 60, 30000), // 60k total > 50k contract
        ],
    )
    .await;

    // Bill first milestone (30k of 50k) — should succeed
    bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m1,
            idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-over-1".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("first milestone should succeed");

    // Bill second milestone (30k more, total 60k > 50k) — should fail
    let err = bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m2,
            idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-over-2".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect_err("should reject over-billing");

    assert!(
        matches!(err, ProgressBillingError::OverBilling { .. }),
        "Expected OverBilling error, got: {:?}",
        err
    );
}

// ============================================================================
// 4. Tenant isolation test
// ============================================================================

#[tokio::test]
async fn tenant_isolation() {
    let pool = common::setup_pool().await;
    let app_a = "tenant-pb-iso-a";
    let app_b = "tenant-pb-iso-b";
    let m1 = Uuid::new_v4();

    // Create contract and milestone under tenant A
    let _cust_a = common::seed_customer(&pool, app_a).await;
    setup_contract_with_milestones(&pool, app_a, 100000, &[(m1, "Milestone A", 50, 50000)]).await;

    // Query milestones as tenant B — should find nothing
    let contract_b_result = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM ar_progress_billing_contracts WHERE app_id = $1",
    )
    .bind(app_b)
    .fetch_one(&pool)
    .await
    .expect("count contracts for tenant b");
    assert_eq!(contract_b_result, 0, "tenant B should see zero contracts");

    let milestones_b_result = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM ar_progress_billing_milestones WHERE app_id = $1",
    )
    .bind(app_b)
    .fetch_one(&pool)
    .await
    .expect("count milestones for tenant b");
    assert_eq!(
        milestones_b_result, 0,
        "tenant B should see zero milestones"
    );
}

// ============================================================================
// 5. Idempotency test
// ============================================================================

#[tokio::test]
async fn idempotency_no_duplicate_invoice() {
    let pool = common::setup_pool().await;
    let app_id = "tenant-pb-idem";
    let m1 = Uuid::new_v4();
    let idem_key = Uuid::new_v4();
    let contract_id =
        setup_contract_with_milestones(&pool, app_id, 100000, &[(m1, "Milestone 1", 30, 30000)])
            .await;

    // First billing
    let r1 = bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m1,
            idempotency_key: idem_key,
            correlation_id: "corr-idem-1".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("first billing");
    assert!(matches!(r1, BillMilestoneResult::Invoiced { .. }));

    // Second billing with same idempotency key — should be AlreadyProcessed
    let r2 = bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m1,
            idempotency_key: idem_key,
            correlation_id: "corr-idem-2".to_string(),
            causation_id: None,
        },
    )
    .await
    .expect("idempotent billing");
    assert!(
        matches!(r2, BillMilestoneResult::AlreadyProcessed { .. }),
        "Expected AlreadyProcessed, got: {:?}",
        r2
    );

    // Verify only one invoice was created
    let invoice_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ar_invoices WHERE app_id = $1 AND metadata->>'contract_id' = $2",
    )
    .bind(app_id)
    .bind(contract_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("count invoices");
    assert_eq!(
        invoice_count, 1,
        "should have exactly 1 invoice, not a duplicate"
    );
}

// ============================================================================
// 6. Outbox event test
// ============================================================================

#[tokio::test]
async fn outbox_event_emitted() {
    let pool = common::setup_pool().await;
    let app_id = "tenant-pb-outbox";
    let m1 = Uuid::new_v4();
    let contract_id = setup_contract_with_milestones(
        &pool,
        app_id,
        100000,
        &[(m1, "Outbox Test Milestone", 40, 40000)],
    )
    .await;

    bill_milestone(
        &pool,
        BillMilestoneRequest {
            app_id: app_id.to_string(),
            contract_id,
            milestone_id: m1,
            idempotency_key: Uuid::new_v4(),
            correlation_id: "corr-outbox-1".to_string(),
            causation_id: Some("test.trigger".to_string()),
        },
    )
    .await
    .expect("bill milestone for outbox test");

    // Verify outbox contains the event
    let outbox_event: (String, i64, String) = sqlx::query_as(
        "SELECT event_type, \
                (payload->'payload'->>'amount_minor')::BIGINT, \
                COALESCE(tenant_id, '') \
         FROM events_outbox \
         WHERE aggregate_id = $1 \
           AND event_type = 'ar.milestone_invoice_created' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(contract_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("fetch outbox event");

    assert_eq!(outbox_event.0, "ar.milestone_invoice_created");
    assert_eq!(outbox_event.1, 40000);
    assert_eq!(outbox_event.2, app_id);
}
