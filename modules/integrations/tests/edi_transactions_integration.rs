//! EDI transaction set integration tests — real Postgres, no mocks.
//!
//! Required test categories:
//! 1. Inbound EDI E2E (ingested → parsed → validated → accepted)
//! 2. Validation rejection (ingest invalid → rejected with error details)
//! 3. Outbound EDI E2E (created → validated → emitted)
//! 4. Tenant isolation (tenant_A invisible to tenant_B)
//! 5. Idempotency (same key = no duplicate)
//! 6. Outbox events (correct event_type and status after each pipeline step)

use integrations_rs::domain::edi_transactions::{
    CreateOutboundEdiRequest, EdiTransactionService, IngestEdiRequest, TransitionEdiRequest,
};
use serial_test::serial;
use sqlx::PgPool;

const TENANT_A: &str = "test-edi-tenant-a";
const TENANT_B: &str = "test-edi-tenant-b";

fn test_db_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://integrations_user:integrations_pass@localhost:5449/integrations_db".to_string()
    })
}

async fn test_pool() -> PgPool {
    let pool = PgPool::connect(&test_db_url())
        .await
        .expect("Failed to connect to integrations test database");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("Migrations failed");
    pool
}

async fn cleanup(pool: &PgPool) {
    sqlx::query("DELETE FROM integrations_outbox WHERE app_id IN ($1, $2)")
        .bind(TENANT_A)
        .bind(TENANT_B)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM integrations_edi_transactions WHERE tenant_id IN ($1, $2)")
        .bind(TENANT_A)
        .bind(TENANT_B)
        .execute(pool)
        .await
        .ok();
}

/// Sample X12 850 Purchase Order (simplified).
fn sample_edi_850() -> &'static str {
    "ISA*00*          *00*          *ZZ*SENDER         *ZZ*RECEIVER       *\
     210101*1253*U*00401*000000001*0*P*>~\
     GS*PO*SENDER*RECEIVER*20210101*1253*1*X*004010~\
     ST*850*0001~\
     BEG*00*NE*PO-12345**20210101~\
     PO1*1*10*EA*25.00**VP*WIDGET-A~\
     CTT*1~\
     SE*5*0001~\
     GE*1*1~\
     IEA*1*000000001~"
}

/// Invalid EDI — missing required segments.
fn invalid_edi() -> &'static str {
    "ISA*00*BROKEN~"
}

// ============================================================================
// 1. Inbound EDI E2E: ingested → parsed → validated → accepted
// ============================================================================

#[tokio::test]
#[serial]
async fn test_inbound_edi_e2e() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = EdiTransactionService::new(pool.clone());

    // Ingest
    let txn = svc
        .ingest(IngestEdiRequest {
            tenant_id: TENANT_A.to_string(),
            transaction_type: "850".to_string(),
            version: "004010".to_string(),
            raw_payload: sample_edi_850().to_string(),
            idempotency_key: None,
        })
        .await
        .expect("ingest failed");

    assert_eq!(txn.validation_status, "ingested");
    assert_eq!(txn.direction, "inbound");
    assert_eq!(txn.transaction_type, "850");
    assert_eq!(txn.version, "004010");
    assert!(txn.raw_payload.is_some());
    assert!(txn.parsed_payload.is_none());

    // Transition: ingested → parsed (simulate parse success, attach parsed payload)
    let parsed_payload = serde_json::json!({
        "transaction_set": "850",
        "segments": ["ISA", "GS", "ST", "BEG", "PO1", "CTT", "SE", "GE", "IEA"],
        "line_items": [{"item": "WIDGET-A", "qty": 10, "price": 25.00}]
    });
    let parsed = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "parsed".to_string(),
            parsed_payload: Some(parsed_payload.clone()),
            error_details: None,
        })
        .await
        .expect("transition to parsed failed");

    assert_eq!(parsed.validation_status, "parsed");
    assert_eq!(parsed.parsed_payload, Some(parsed_payload));
    assert!(parsed.updated_at >= txn.updated_at);

    // Transition: parsed → validated
    let validated = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "validated".to_string(),
            parsed_payload: None,
            error_details: None,
        })
        .await
        .expect("transition to validated failed");

    assert_eq!(validated.validation_status, "validated");

    // Transition: validated → accepted
    let accepted = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "accepted".to_string(),
            parsed_payload: None,
            error_details: None,
        })
        .await
        .expect("transition to accepted failed");

    assert_eq!(accepted.validation_status, "accepted");
    assert!(accepted.updated_at >= validated.updated_at);

    // Verify via get
    let fetched = svc
        .get(TENANT_A, txn.id)
        .await
        .expect("get failed")
        .expect("transaction should exist");
    assert_eq!(fetched.validation_status, "accepted");

    cleanup(&pool).await;
}

// ============================================================================
// 2. Validation rejection: ingest invalid EDI → rejected with error details
// ============================================================================

#[tokio::test]
#[serial]
async fn test_validation_rejection() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = EdiTransactionService::new(pool.clone());

    // Ingest an invalid EDI document
    let txn = svc
        .ingest(IngestEdiRequest {
            tenant_id: TENANT_A.to_string(),
            transaction_type: "850".to_string(),
            version: "004010".to_string(),
            raw_payload: invalid_edi().to_string(),
            idempotency_key: None,
        })
        .await
        .expect("ingest failed");

    assert_eq!(txn.validation_status, "ingested");

    // Simulate parse failure → reject directly from ingested
    let error_msg = "Parse error: missing GS segment, incomplete ISA envelope";
    let rejected = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "rejected".to_string(),
            parsed_payload: None,
            error_details: Some(error_msg.to_string()),
        })
        .await
        .expect("transition to rejected failed");

    assert_eq!(rejected.validation_status, "rejected");
    assert_eq!(rejected.error_details.as_deref(), Some(error_msg));

    // Verify persisted
    let fetched = svc
        .get(TENANT_A, txn.id)
        .await
        .expect("get failed")
        .expect("transaction should exist");
    assert_eq!(fetched.validation_status, "rejected");
    assert_eq!(fetched.error_details.as_deref(), Some(error_msg));

    cleanup(&pool).await;
}

// ============================================================================
// 3. Outbound EDI E2E: created → validated → emitted
// ============================================================================

#[tokio::test]
#[serial]
async fn test_outbound_edi_e2e() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = EdiTransactionService::new(pool.clone());

    let payload = serde_json::json!({
        "transaction_set": "810",
        "invoice_number": "INV-2026-001",
        "line_items": [{"item": "WIDGET-A", "qty": 10, "price": 25.00}],
        "total": 250.00
    });

    // Create outbound
    let txn = svc
        .create_outbound(CreateOutboundEdiRequest {
            tenant_id: TENANT_A.to_string(),
            transaction_type: "810".to_string(),
            version: "004010".to_string(),
            parsed_payload: payload.clone(),
            idempotency_key: None,
        })
        .await
        .expect("create outbound failed");

    assert_eq!(txn.validation_status, "created");
    assert_eq!(txn.direction, "outbound");
    assert_eq!(txn.transaction_type, "810");
    assert_eq!(txn.parsed_payload, Some(payload));
    assert!(txn.raw_payload.is_none());

    // Transition: created → validated
    let validated = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "validated".to_string(),
            parsed_payload: None,
            error_details: None,
        })
        .await
        .expect("transition to validated failed");

    assert_eq!(validated.validation_status, "validated");

    // Transition: validated → emitted
    let emitted = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "emitted".to_string(),
            parsed_payload: None,
            error_details: None,
        })
        .await
        .expect("transition to emitted failed");

    assert_eq!(emitted.validation_status, "emitted");
    assert!(emitted.updated_at >= validated.updated_at);

    // Verify via get
    let fetched = svc
        .get(TENANT_A, txn.id)
        .await
        .expect("get failed")
        .expect("transaction should exist");
    assert_eq!(fetched.validation_status, "emitted");

    cleanup(&pool).await;
}

// ============================================================================
// 4. Tenant isolation: tenant_A invisible to tenant_B
// ============================================================================

#[tokio::test]
#[serial]
async fn test_edi_tenant_isolation() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = EdiTransactionService::new(pool.clone());

    // Ingest under tenant A
    let txn = svc
        .ingest(IngestEdiRequest {
            tenant_id: TENANT_A.to_string(),
            transaction_type: "850".to_string(),
            version: "004010".to_string(),
            raw_payload: sample_edi_850().to_string(),
            idempotency_key: None,
        })
        .await
        .expect("ingest failed");

    // tenant_B cannot see tenant_A's transaction via get
    let invisible = svc
        .get(TENANT_B, txn.id)
        .await
        .expect("get should not error");
    assert!(
        invisible.is_none(),
        "tenant_B should not see tenant_A transactions"
    );

    // tenant_B list returns zero results
    let list_b = svc.list(TENANT_B).await.expect("list failed");
    assert_eq!(list_b.len(), 0, "tenant_B list should be empty");

    // tenant_A list returns the transaction
    let list_a = svc.list(TENANT_A).await.expect("list failed");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].id, txn.id);

    // tenant_B cannot transition tenant_A's transaction
    let err = svc
        .transition(TransitionEdiRequest {
            transaction_id: txn.id,
            tenant_id: TENANT_B.to_string(),
            new_status: "parsed".to_string(),
            parsed_payload: None,
            error_details: None,
        })
        .await;
    assert!(
        err.is_err(),
        "tenant_B should not transition tenant_A transactions"
    );

    cleanup(&pool).await;
}

// ============================================================================
// 5. Idempotency: same key = no duplicate
// ============================================================================

#[tokio::test]
#[serial]
async fn test_edi_idempotency() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = EdiTransactionService::new(pool.clone());

    let req = IngestEdiRequest {
        tenant_id: TENANT_A.to_string(),
        transaction_type: "850".to_string(),
        version: "004010".to_string(),
        raw_payload: sample_edi_850().to_string(),
        idempotency_key: Some("edi-import-batch-2026-03-03".to_string()),
    };

    let first = svc.ingest(req.clone()).await.expect("first ingest failed");

    // Second ingest with same key returns same transaction
    let second = svc.ingest(req.clone()).await.expect("second ingest failed");

    assert_eq!(
        first.id, second.id,
        "idempotent ingest should return same transaction"
    );

    // Verify only one row exists
    let list = svc.list(TENANT_A).await.expect("list failed");
    assert_eq!(list.len(), 1, "should be exactly one transaction");

    cleanup(&pool).await;
}

// ============================================================================
// 6. Outbox events: correct event_type, status, and tenant_id after each step
// ============================================================================

#[tokio::test]
#[serial]
async fn test_edi_outbox_events() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = EdiTransactionService::new(pool.clone());

    // Ingest
    let txn = svc
        .ingest(IngestEdiRequest {
            tenant_id: TENANT_A.to_string(),
            transaction_type: "850".to_string(),
            version: "004010".to_string(),
            raw_payload: sample_edi_850().to_string(),
            idempotency_key: None,
        })
        .await
        .expect("ingest failed");

    // Verify edi_transaction.created outbox event
    let created_count: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'edi_transaction'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'edi_transaction.created'"#,
    )
    .bind(txn.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(
        created_count.0, 1,
        "expected one edi_transaction.created event"
    );

    // Transition: ingested → parsed
    svc.transition(TransitionEdiRequest {
        transaction_id: txn.id,
        tenant_id: TENANT_A.to_string(),
        new_status: "parsed".to_string(),
        parsed_payload: Some(serde_json::json!({"segments": ["ISA","GS","ST"]})),
        error_details: None,
    })
    .await
    .expect("transition to parsed failed");

    // Verify status_changed event for parsed
    let parsed_count: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'edi_transaction'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'edi_transaction.status_changed'"#,
    )
    .bind(txn.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(
        parsed_count.0, 1,
        "expected one status_changed event after parsed"
    );

    // Transition: parsed → validated
    svc.transition(TransitionEdiRequest {
        transaction_id: txn.id,
        tenant_id: TENANT_A.to_string(),
        new_status: "validated".to_string(),
        parsed_payload: None,
        error_details: None,
    })
    .await
    .expect("transition to validated failed");

    // Transition: validated → accepted
    svc.transition(TransitionEdiRequest {
        transaction_id: txn.id,
        tenant_id: TENANT_A.to_string(),
        new_status: "accepted".to_string(),
        parsed_payload: None,
        error_details: None,
    })
    .await
    .expect("transition to accepted failed");

    // Should now have 3 status_changed events total (parsed + validated + accepted)
    let all_changed: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'edi_transaction'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'edi_transaction.status_changed'"#,
    )
    .bind(txn.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(
        all_changed.0, 3,
        "expected three status_changed events (parsed + validated + accepted)"
    );

    // Total outbox events: 1 created + 3 status_changed = 4
    let total: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'edi_transaction'
             AND aggregate_id = $1
             AND app_id = $2"#,
    )
    .bind(txn.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(
        total.0, 4,
        "expected 4 total outbox events for this transaction"
    );

    // Verify payload contains correct tenant_id and transaction_id
    let payload: (serde_json::Value,) = sqlx::query_as(
        r#"SELECT payload FROM integrations_outbox
           WHERE aggregate_type = 'edi_transaction'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'edi_transaction.created'
           LIMIT 1"#,
    )
    .bind(txn.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("payload query failed");

    let envelope = &payload.0;
    let inner_payload = &envelope["payload"];
    assert_eq!(
        inner_payload["tenant_id"].as_str(),
        Some(TENANT_A),
        "event payload must carry tenant_id"
    );
    assert_eq!(
        inner_payload["transaction_id"].as_str(),
        Some(txn.id.to_string()).as_deref(),
        "event payload must carry transaction_id"
    );

    // Verify a status_changed event payload has correct previous/new status
    let changed_payload: (serde_json::Value,) = sqlx::query_as(
        r#"SELECT payload FROM integrations_outbox
           WHERE aggregate_type = 'edi_transaction'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'edi_transaction.status_changed'
           ORDER BY created_at ASC
           LIMIT 1"#,
    )
    .bind(txn.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("changed payload query failed");

    let changed_inner = &changed_payload.0["payload"];
    assert_eq!(
        changed_inner["previous_status"].as_str(),
        Some("ingested"),
        "first status_changed should transition from ingested"
    );
    assert_eq!(
        changed_inner["new_status"].as_str(),
        Some("parsed"),
        "first status_changed should transition to parsed"
    );

    cleanup(&pool).await;
}
