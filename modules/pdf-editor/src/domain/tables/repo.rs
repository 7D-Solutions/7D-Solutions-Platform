//! Table render repository — database access layer.
//!
//! All mutations follow Guard → Mutation → Outbox:
//! 1. Guard: validate_render_request() checks table structure and PDF
//! 2. Mutation: INSERT render request, execute render, UPDATE with output
//! 3. Outbox: enqueue pdf.table.rendered event in same transaction

use sqlx::PgPool;
use uuid::Uuid;

use crate::event_bus::{create_pdf_editor_envelope, enqueue_event};

use super::{
    render_table, validate_render_request, RenderTableRequest, TableError,
    TableRenderRequest, TableRenderedPayload,
};

pub struct TableRenderRepo;

impl TableRenderRepo {
    /// Render a table onto a PDF template. Idempotent via (tenant_id, idempotency_key).
    ///
    /// Flow: Guard → Mutation → Outbox (all in one transaction).
    pub async fn render(
        pool: &PgPool,
        req: &RenderTableRequest,
    ) -> Result<TableRenderRequest, TableError> {
        // ── Guard ──────────────────────────────────────────────
        validate_render_request(req)?;

        // ── Check idempotency (return existing if duplicate key) ──
        let existing: Option<TableRenderRequest> = sqlx::query_as(
            "SELECT * FROM table_render_requests WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(req.tenant_id.trim())
        .bind(req.idempotency_key.trim())
        .fetch_optional(pool)
        .await?;

        if let Some(record) = existing {
            return Ok(record);
        }

        // ── Render the table ─────────────────────────────────
        let table_def_json =
            serde_json::to_value(&req.table_definition).map_err(|e| {
                TableError::Render(format!("failed to serialize table definition: {}", e))
            })?;

        let (pdf_output, status, error_message) =
            match render_table(&req.pdf_template, &req.table_definition) {
                Ok(bytes) => (Some(bytes), "rendered", None),
                Err(e) => (None, "failed", Some(e.to_string())),
            };

        // ── Mutation + Outbox (single transaction) ─────────────
        let mut tx = pool.begin().await?;

        let record: TableRenderRequest = sqlx::query_as(
            r#"
            INSERT INTO table_render_requests
                (tenant_id, idempotency_key, table_definition, pdf_output,
                 status, error_message, rendered_at)
            VALUES ($1, $2, $3, $4, $5, $6,
                    CASE WHEN $5 = 'rendered' THEN NOW() ELSE NULL END)
            RETURNING *
            "#,
        )
        .bind(req.tenant_id.trim())
        .bind(req.idempotency_key.trim())
        .bind(&table_def_json)
        .bind(pdf_output.as_deref())
        .bind(status)
        .bind(error_message.as_deref())
        .fetch_one(&mut *tx)
        .await?;

        // ── Outbox ─────────────────────────────────────────────
        let payload = TableRenderedPayload {
            tenant_id: record.tenant_id.clone(),
            render_request_id: record.id,
            status: record.status.clone(),
        };
        let envelope = create_pdf_editor_envelope(
            Uuid::new_v4(),
            record.tenant_id.clone(),
            "pdf.table.rendered".to_string(),
            None,
            None,
            "DATA_MUTATION".to_string(),
            payload,
        );
        enqueue_event(&mut tx, "pdf.table.rendered", &envelope).await?;

        tx.commit().await?;
        Ok(record)
    }

    /// Find a render request by ID with tenant isolation.
    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<TableRenderRequest>, TableError> {
        sqlx::query_as::<_, TableRenderRequest>(
            "SELECT * FROM table_render_requests WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(TableError::Database)
    }

    /// List render requests for a tenant.
    pub async fn list(
        pool: &PgPool,
        tenant_id: &str,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<Vec<TableRenderRequest>, TableError> {
        if tenant_id.trim().is_empty() {
            return Err(TableError::Validation("tenant_id is required".into()));
        }
        let limit = limit.unwrap_or(50).clamp(1, 100);
        let offset = offset.unwrap_or(0);

        sqlx::query_as::<_, TableRenderRequest>(
            r#"
            SELECT * FROM table_render_requests
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(tenant_id.trim())
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(TableError::Database)
    }
}
