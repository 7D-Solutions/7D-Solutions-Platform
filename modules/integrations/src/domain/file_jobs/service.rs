//! File job service — Guard → Mutation → Outbox pattern.
//!
//! Manages the lifecycle of file import/export jobs:
//! - create: new job in "created" status (idempotent via idempotency_key)
//! - transition: move through created → processing → completed/failed
//! - get / list: tenant-scoped reads

use sqlx::PgPool;
use uuid::Uuid;

use crate::events::{
    build_file_job_created_envelope, build_file_job_status_changed_envelope, FileJobCreatedPayload,
    FileJobStatusChangedPayload, EVENT_TYPE_FILE_JOB_CREATED, EVENT_TYPE_FILE_JOB_STATUS_CHANGED,
};
use crate::outbox::enqueue_event_tx;

use super::guards::{validate_create, validate_transition};
use super::models::{
    CreateFileJobRequest, FileJob, FileJobError, TransitionFileJobRequest, STATUS_CREATED,
};

pub struct FileJobService {
    pool: PgPool,
}

impl FileJobService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    // ========================================================================
    // Reads
    // ========================================================================

    /// Get a single job by ID, scoped to tenant.
    pub async fn get(
        &self,
        tenant_id: &str,
        job_id: Uuid,
    ) -> Result<Option<FileJob>, FileJobError> {
        let row = sqlx::query_as::<_, FileJob>(
            r#"SELECT id, tenant_id, file_ref, parser_type, status,
                      error_details, idempotency_key, created_at, updated_at
               FROM integrations_file_jobs
               WHERE id = $1 AND tenant_id = $2"#,
        )
        .bind(job_id)
        .bind(tenant_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// List all jobs for a tenant.
    pub async fn list(&self, tenant_id: &str) -> Result<Vec<FileJob>, FileJobError> {
        let rows = sqlx::query_as::<_, FileJob>(
            r#"SELECT id, tenant_id, file_ref, parser_type, status,
                      error_details, idempotency_key, created_at, updated_at
               FROM integrations_file_jobs
               WHERE tenant_id = $1
               ORDER BY created_at DESC"#,
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    // ========================================================================
    // Create — Guard → Mutation → Outbox
    // ========================================================================

    /// Create a new file job. Idempotent: if a job with the same
    /// (tenant_id, idempotency_key) exists, the existing job is returned.
    pub async fn create(&self, req: CreateFileJobRequest) -> Result<FileJob, FileJobError> {
        // ── Guard ────────────────────────────────────────────────────
        validate_create(&req)?;

        let mut tx = self.pool.begin().await?;

        // Idempotency check
        if let Some(ref key) = req.idempotency_key {
            let existing = sqlx::query_as::<_, FileJob>(
                r#"SELECT id, tenant_id, file_ref, parser_type, status,
                          error_details, idempotency_key, created_at, updated_at
                   FROM integrations_file_jobs
                   WHERE tenant_id = $1 AND idempotency_key = $2"#,
            )
            .bind(&req.tenant_id)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;

            if let Some(job) = existing {
                tx.rollback().await?;
                return Ok(job);
            }
        }

        // ── Mutation ─────────────────────────────────────────────────
        let job_id = Uuid::new_v4();

        let job = sqlx::query_as::<_, FileJob>(
            r#"INSERT INTO integrations_file_jobs
                   (id, tenant_id, file_ref, parser_type, status, idempotency_key)
               VALUES ($1, $2, $3, $4, $5, $6)
               RETURNING id, tenant_id, file_ref, parser_type, status,
                         error_details, idempotency_key, created_at, updated_at"#,
        )
        .bind(job_id)
        .bind(&req.tenant_id)
        .bind(req.file_ref.trim())
        .bind(req.parser_type.trim())
        .bind(STATUS_CREATED)
        .bind(&req.idempotency_key)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ───────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_file_job_created_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            FileJobCreatedPayload {
                job_id: job.id,
                tenant_id: req.tenant_id.clone(),
                file_ref: job.file_ref.clone(),
                parser_type: job.parser_type.clone(),
                status: job.status.clone(),
                created_at: job.created_at,
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_FILE_JOB_CREATED,
            "file_job",
            &job.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;
        Ok(job)
    }

    // ========================================================================
    // Transition — Guard → Mutation → Outbox
    // ========================================================================

    /// Transition a job to a new status. Validates the transition is legal
    /// (created→processing, processing→completed, processing→failed).
    pub async fn transition(&self, req: TransitionFileJobRequest) -> Result<FileJob, FileJobError> {
        let mut tx = self.pool.begin().await?;

        // Fetch + lock
        let existing = sqlx::query_as::<_, FileJob>(
            r#"SELECT id, tenant_id, file_ref, parser_type, status,
                      error_details, idempotency_key, created_at, updated_at
               FROM integrations_file_jobs
               WHERE id = $1 AND tenant_id = $2
               FOR UPDATE"#,
        )
        .bind(req.job_id)
        .bind(&req.tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(FileJobError::NotFound)?;

        // ── Guard ────────────────────────────────────────────────────
        let previous_status = existing.status.clone();
        validate_transition(&previous_status, &req)?;

        // ── Mutation ─────────────────────────────────────────────────
        let updated = sqlx::query_as::<_, FileJob>(
            r#"UPDATE integrations_file_jobs
               SET status = $1, error_details = $2, updated_at = NOW()
               WHERE id = $3 AND tenant_id = $4
               RETURNING id, tenant_id, file_ref, parser_type, status,
                         error_details, idempotency_key, created_at, updated_at"#,
        )
        .bind(&req.new_status)
        .bind(&req.error_details)
        .bind(req.job_id)
        .bind(&req.tenant_id)
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ───────────────────────────────────────────────────
        let event_id = Uuid::new_v4();
        let correlation_id = Uuid::new_v4().to_string();

        let envelope = build_file_job_status_changed_envelope(
            event_id,
            req.tenant_id.clone(),
            correlation_id,
            None,
            FileJobStatusChangedPayload {
                job_id: updated.id,
                tenant_id: req.tenant_id.clone(),
                previous_status,
                new_status: req.new_status.clone(),
                error_details: req.error_details.clone(),
                changed_at: updated.updated_at,
            },
        );

        enqueue_event_tx(
            &mut tx,
            event_id,
            EVENT_TYPE_FILE_JOB_STATUS_CHANGED,
            "file_job",
            &updated.id.to_string(),
            &req.tenant_id,
            &envelope,
        )
        .await?;

        tx.commit().await?;
        Ok(updated)
    }
}
