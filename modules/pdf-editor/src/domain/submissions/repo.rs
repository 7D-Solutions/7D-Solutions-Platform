//! Submission repository — database access layer.

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::forms::FieldRepo;
use crate::event_bus::{create_pdf_editor_envelope, enqueue_event};

use super::validation::validate_submission;
use super::{
    AutosaveRequest, CreateSubmissionRequest, FormSubmission, FormSubmittedPayload,
    ListSubmissionsQuery, SubmissionError,
};

pub struct SubmissionRepo;

impl SubmissionRepo {
    /// Create a new draft submission.
    pub async fn create(
        pool: &PgPool,
        req: &CreateSubmissionRequest,
    ) -> Result<FormSubmission, SubmissionError> {
        if req.tenant_id.trim().is_empty() {
            return Err(SubmissionError::Validation("tenant_id is required".into()));
        }
        if req.submitted_by.trim().is_empty() {
            return Err(SubmissionError::Validation(
                "submitted_by is required".into(),
            ));
        }

        // Verify template exists and belongs to tenant
        let tmpl_exists: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM form_templates WHERE id = $1 AND tenant_id = $2",
        )
        .bind(req.template_id)
        .bind(req.tenant_id.trim())
        .fetch_optional(pool)
        .await?;
        if tmpl_exists.is_none() {
            return Err(SubmissionError::TemplateNotFound);
        }

        let field_data = req
            .field_data
            .as_ref()
            .unwrap_or(&serde_json::json!({}))
            .clone();

        sqlx::query_as::<_, FormSubmission>(
            r#"
            INSERT INTO form_submissions (tenant_id, template_id, submitted_by, field_data)
            VALUES ($1, $2, $3, $4)
            RETURNING *
            "#,
        )
        .bind(req.tenant_id.trim())
        .bind(req.template_id)
        .bind(req.submitted_by.trim())
        .bind(&field_data)
        .fetch_one(pool)
        .await
        .map_err(SubmissionError::Database)
    }

    /// Autosave field_data on a draft submission.
    pub async fn autosave(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
        req: &AutosaveRequest,
    ) -> Result<FormSubmission, SubmissionError> {
        let sub = sqlx::query_as::<_, FormSubmission>(
            r#"
            UPDATE form_submissions
            SET field_data = $3
            WHERE id = $1 AND tenant_id = $2 AND status = 'draft'
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(&req.field_data)
        .fetch_optional(pool)
        .await?;

        match sub {
            Some(s) => Ok(s),
            None => {
                // Distinguish not-found vs already-submitted
                let exists: Option<(String,)> = sqlx::query_as(
                    "SELECT status FROM form_submissions WHERE id = $1 AND tenant_id = $2",
                )
                .bind(id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;
                match exists {
                    Some((status,)) if status == "submitted" => {
                        Err(SubmissionError::AlreadySubmitted)
                    }
                    _ => Err(SubmissionError::NotFound),
                }
            }
        }
    }

    /// Validate and submit a draft submission. Emits pdf.form.submitted event.
    pub async fn submit(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<FormSubmission, SubmissionError> {
        // Fetch the submission
        let sub: FormSubmission = sqlx::query_as(
            "SELECT * FROM form_submissions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await?
        .ok_or(SubmissionError::NotFound)?;

        if sub.status == "submitted" {
            return Err(SubmissionError::AlreadySubmitted);
        }

        // Load template fields for validation
        let fields = FieldRepo::list_by_template(pool, sub.template_id).await?;

        // Validate field_data against field definitions
        if let Err(errors) = validate_submission(&fields, &sub.field_data) {
            return Err(SubmissionError::Validation(errors.join("; ")));
        }

        // Transition to submitted + enqueue event in one transaction
        let mut tx = pool.begin().await?;

        let submitted: FormSubmission = sqlx::query_as(
            r#"
            UPDATE form_submissions
            SET status = 'submitted', submitted_at = NOW()
            WHERE id = $1 AND tenant_id = $2 AND status = 'draft'
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SubmissionError::AlreadySubmitted)?;

        // Enqueue pdf.form.submitted event
        let payload = FormSubmittedPayload {
            tenant_id: submitted.tenant_id.clone(),
            submission_id: submitted.id,
            template_id: submitted.template_id,
            submitted_by: submitted.submitted_by.clone(),
        };
        let envelope = create_pdf_editor_envelope(
            Uuid::new_v4(),
            submitted.tenant_id.clone(),
            "pdf.form.submitted".to_string(),
            None,
            None,
            "DATA_MUTATION".to_string(),
            payload,
        );
        enqueue_event(&mut tx, "pdf.form.submitted", &envelope).await?;

        tx.commit().await?;
        Ok(submitted)
    }

    /// Find a submission by ID with tenant isolation.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<FormSubmission>, SubmissionError> {
        sqlx::query_as::<_, FormSubmission>(
            "SELECT * FROM form_submissions WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(SubmissionError::Database)
    }

    /// List submissions with optional template_id and status filters.
    pub async fn list(
        pool: &PgPool,
        q: &ListSubmissionsQuery,
    ) -> Result<Vec<FormSubmission>, SubmissionError> {
        if q.tenant_id.trim().is_empty() {
            return Err(SubmissionError::Validation("tenant_id is required".into()));
        }
        let limit = q.limit.unwrap_or(50).clamp(1, 100);
        let offset = q.offset.unwrap_or(0);

        sqlx::query_as::<_, FormSubmission>(
            r#"
            SELECT * FROM form_submissions
            WHERE tenant_id = $1
              AND ($2::uuid IS NULL OR template_id = $2)
              AND ($3::text IS NULL OR status = $3)
            ORDER BY created_at DESC
            LIMIT $4 OFFSET $5
            "#,
        )
        .bind(q.tenant_id.trim())
        .bind(q.template_id)
        .bind(q.status.as_deref())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(SubmissionError::Database)
    }
}
