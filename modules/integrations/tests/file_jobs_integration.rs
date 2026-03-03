//! File jobs integration tests — real Postgres, no mocks.
//!
//! Required test categories:
//! 1. Job lifecycle E2E (created → processing → completed)
//! 2. Failed job (created → processing → failed with error details)
//! 3. Tenant isolation (tenant_A jobs invisible to tenant_B)
//! 4. Idempotency (same key = no duplicate)
//! 5. Outbox events (correct event_type, job_id, tenant_id after transitions)

use integrations_rs::domain::file_jobs::{
    CreateFileJobRequest, FileJobService, TransitionFileJobRequest,
};
use serial_test::serial;
use sqlx::PgPool;

const TENANT_A: &str = "test-file-jobs-tenant-a";
const TENANT_B: &str = "test-file-jobs-tenant-b";

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
    sqlx::query("DELETE FROM integrations_file_jobs WHERE tenant_id IN ($1, $2)")
        .bind(TENANT_A)
        .bind(TENANT_B)
        .execute(pool)
        .await
        .ok();
}

// ============================================================================
// 1. Job lifecycle E2E: created → processing → completed
// ============================================================================

#[tokio::test]
#[serial]
async fn test_file_job_lifecycle_completed() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = FileJobService::new(pool.clone());

    // Create
    let job = svc
        .create(CreateFileJobRequest {
            tenant_id: TENANT_A.to_string(),
            file_ref: "s3://imports/batch-001.csv".to_string(),
            parser_type: "csv".to_string(),
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    assert_eq!(job.status, "created");
    assert_eq!(job.tenant_id, TENANT_A);
    assert_eq!(job.parser_type, "csv");
    assert!(job.error_details.is_none());

    // Transition: created → processing
    let processing = svc
        .transition(TransitionFileJobRequest {
            job_id: job.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "processing".to_string(),
            error_details: None,
        })
        .await
        .expect("transition to processing failed");

    assert_eq!(processing.status, "processing");
    assert!(processing.updated_at >= job.updated_at);

    // Transition: processing → completed
    let completed = svc
        .transition(TransitionFileJobRequest {
            job_id: job.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "completed".to_string(),
            error_details: None,
        })
        .await
        .expect("transition to completed failed");

    assert_eq!(completed.status, "completed");
    assert!(completed.updated_at >= processing.updated_at);

    // Verify via get
    let fetched = svc
        .get(TENANT_A, job.id)
        .await
        .expect("get failed")
        .expect("job should exist");
    assert_eq!(fetched.status, "completed");

    cleanup(&pool).await;
}

// ============================================================================
// 2. Failed job: created → processing → failed with error_details
// ============================================================================

#[tokio::test]
#[serial]
async fn test_file_job_lifecycle_failed() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = FileJobService::new(pool.clone());

    let job = svc
        .create(CreateFileJobRequest {
            tenant_id: TENANT_A.to_string(),
            file_ref: "s3://imports/bad-file.csv".to_string(),
            parser_type: "csv".to_string(),
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    // created → processing
    svc.transition(TransitionFileJobRequest {
        job_id: job.id,
        tenant_id: TENANT_A.to_string(),
        new_status: "processing".to_string(),
        error_details: None,
    })
    .await
    .expect("transition to processing failed");

    // processing → failed
    let error_msg = "Row 42: invalid date format in column 'due_date'";
    let failed = svc
        .transition(TransitionFileJobRequest {
            job_id: job.id,
            tenant_id: TENANT_A.to_string(),
            new_status: "failed".to_string(),
            error_details: Some(error_msg.to_string()),
        })
        .await
        .expect("transition to failed");

    assert_eq!(failed.status, "failed");
    assert_eq!(failed.error_details.as_deref(), Some(error_msg));

    // Verify persisted
    let fetched = svc
        .get(TENANT_A, job.id)
        .await
        .expect("get failed")
        .expect("job should exist");
    assert_eq!(fetched.error_details.as_deref(), Some(error_msg));

    cleanup(&pool).await;
}

// ============================================================================
// 3. Tenant isolation: tenant_A jobs invisible to tenant_B
// ============================================================================

#[tokio::test]
#[serial]
async fn test_file_job_tenant_isolation() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = FileJobService::new(pool.clone());

    // Create jobs under tenant A
    let job = svc
        .create(CreateFileJobRequest {
            tenant_id: TENANT_A.to_string(),
            file_ref: "s3://imports/tenant-a.csv".to_string(),
            parser_type: "csv".to_string(),
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    // tenant_B cannot see tenant_A's job via get
    let invisible = svc
        .get(TENANT_B, job.id)
        .await
        .expect("get should not error");
    assert!(invisible.is_none(), "tenant_B should not see tenant_A jobs");

    // tenant_B list returns zero results
    let list_b = svc.list(TENANT_B).await.expect("list failed");
    assert_eq!(list_b.len(), 0, "tenant_B list should be empty");

    // tenant_A list returns the job
    let list_a = svc.list(TENANT_A).await.expect("list failed");
    assert_eq!(list_a.len(), 1);
    assert_eq!(list_a[0].id, job.id);

    // tenant_B cannot transition tenant_A's job
    let err = svc
        .transition(TransitionFileJobRequest {
            job_id: job.id,
            tenant_id: TENANT_B.to_string(),
            new_status: "processing".to_string(),
            error_details: None,
        })
        .await;
    assert!(err.is_err(), "tenant_B should not transition tenant_A jobs");

    cleanup(&pool).await;
}

// ============================================================================
// 4. Idempotency: same key = no duplicate
// ============================================================================

#[tokio::test]
#[serial]
async fn test_file_job_idempotency() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = FileJobService::new(pool.clone());

    let req = CreateFileJobRequest {
        tenant_id: TENANT_A.to_string(),
        file_ref: "s3://imports/idem-test.csv".to_string(),
        parser_type: "csv".to_string(),
        idempotency_key: Some("import-batch-2026-03-03".to_string()),
    };

    let first = svc.create(req.clone()).await.expect("first create failed");

    // Second create with same key returns same job
    let second = svc.create(req.clone()).await.expect("second create failed");

    assert_eq!(first.id, second.id, "idempotent create should return same job");

    // Verify only one row exists
    let list = svc.list(TENANT_A).await.expect("list failed");
    assert_eq!(list.len(), 1, "should be exactly one job");

    cleanup(&pool).await;
}

// ============================================================================
// 5. Outbox events: correct event_type, job_id, tenant_id
// ============================================================================

#[tokio::test]
#[serial]
async fn test_file_job_outbox_events() {
    let pool = test_pool().await;
    cleanup(&pool).await;

    let svc = FileJobService::new(pool.clone());

    // Create
    let job = svc
        .create(CreateFileJobRequest {
            tenant_id: TENANT_A.to_string(),
            file_ref: "s3://imports/outbox-test.csv".to_string(),
            parser_type: "csv".to_string(),
            idempotency_key: None,
        })
        .await
        .expect("create failed");

    // Verify file_job.created outbox event
    let created_count: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'file_job'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'file_job.created'"#,
    )
    .bind(job.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(created_count.0, 1, "expected one file_job.created event");

    // Transition: created → processing
    svc.transition(TransitionFileJobRequest {
        job_id: job.id,
        tenant_id: TENANT_A.to_string(),
        new_status: "processing".to_string(),
        error_details: None,
    })
    .await
    .expect("transition to processing failed");

    // Verify file_job.status_changed outbox event for processing
    let processing_count: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'file_job'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'file_job.status_changed'"#,
    )
    .bind(job.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(
        processing_count.0, 1,
        "expected one status_changed event after processing"
    );

    // Transition: processing → completed
    svc.transition(TransitionFileJobRequest {
        job_id: job.id,
        tenant_id: TENANT_A.to_string(),
        new_status: "completed".to_string(),
        error_details: None,
    })
    .await
    .expect("transition to completed failed");

    // Should now have 2 status_changed events total
    let all_changed: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'file_job'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'file_job.status_changed'"#,
    )
    .bind(job.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(
        all_changed.0, 2,
        "expected two status_changed events (processing + completed)"
    );

    // Total outbox events for this job: 1 created + 2 status_changed = 3
    let total: (i64,) = sqlx::query_as(
        r#"SELECT COUNT(*) FROM integrations_outbox
           WHERE aggregate_type = 'file_job'
             AND aggregate_id = $1
             AND app_id = $2"#,
    )
    .bind(job.id.to_string())
    .bind(TENANT_A)
    .fetch_one(&pool)
    .await
    .expect("outbox query failed");
    assert_eq!(total.0, 3, "expected 3 total outbox events for this job");

    // Verify payload contains correct tenant_id and job_id
    let payload: (serde_json::Value,) = sqlx::query_as(
        r#"SELECT payload FROM integrations_outbox
           WHERE aggregate_type = 'file_job'
             AND aggregate_id = $1
             AND app_id = $2
             AND event_type = 'file_job.created'
           LIMIT 1"#,
    )
    .bind(job.id.to_string())
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
        inner_payload["job_id"].as_str(),
        Some(job.id.to_string()).as_deref(),
        "event payload must carry job_id"
    );

    cleanup(&pool).await;
}
