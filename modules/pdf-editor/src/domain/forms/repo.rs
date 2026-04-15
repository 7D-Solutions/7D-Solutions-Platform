//! Template and field repositories — database access layer.

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    validate_field_type, CreateFieldRequest, CreateTemplateRequest, FormError, FormField,
    FormTemplate, ListTemplatesQuery, ReorderFieldsRequest, UpdateFieldRequest,
    UpdateTemplateRequest,
};

pub struct TemplateRepo;

impl TemplateRepo {
    pub async fn create(
        pool: &PgPool,
        req: &CreateTemplateRequest,
    ) -> Result<FormTemplate, FormError> {
        if req.tenant_id.trim().is_empty() {
            return Err(FormError::Validation("tenant_id is required".into()));
        }
        if req.name.trim().is_empty() {
            return Err(FormError::Validation("name is required".into()));
        }
        if req.created_by.trim().is_empty() {
            return Err(FormError::Validation("created_by is required".into()));
        }

        sqlx::query_as::<_, FormTemplate>(
            r#"
            INSERT INTO form_templates (tenant_id, name, description, created_by)
            VALUES ($1, $2, $3, $4)
            RETURNING *
            "#,
        )
        .bind(req.tenant_id.trim())
        .bind(req.name.trim())
        .bind(req.description.as_deref())
        .bind(req.created_by.trim())
        .fetch_one(pool)
        .await
        .map_err(FormError::Database)
    }

    pub async fn find_by_id(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
    ) -> Result<Option<FormTemplate>, FormError> {
        sqlx::query_as::<_, FormTemplate>(
            "SELECT * FROM form_templates WHERE id = $1 AND tenant_id = $2",
        )
        .bind(id)
        .bind(tenant_id)
        .fetch_optional(pool)
        .await
        .map_err(FormError::Database)
    }

    pub async fn list(
        pool: &PgPool,
        q: &ListTemplatesQuery,
    ) -> Result<(Vec<FormTemplate>, i64), FormError> {
        if q.tenant_id.trim().is_empty() {
            return Err(FormError::Validation("tenant_id is required".into()));
        }
        let page_size = q.page_size.unwrap_or(50).clamp(1, 100);
        let page = q.page.unwrap_or(1).max(1);
        let offset = (page - 1) * page_size;

        let total: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM form_templates WHERE tenant_id = $1")
                .bind(q.tenant_id.trim())
                .fetch_one(pool)
                .await?;

        let items = sqlx::query_as::<_, FormTemplate>(
            r#"
            SELECT * FROM form_templates
            WHERE tenant_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(q.tenant_id.trim())
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok((items, total.0))
    }

    pub async fn update(
        pool: &PgPool,
        id: Uuid,
        tenant_id: &str,
        req: &UpdateTemplateRequest,
    ) -> Result<FormTemplate, FormError> {
        if let Some(ref name) = req.name {
            if name.trim().is_empty() {
                return Err(FormError::Validation("name must not be empty".into()));
            }
        }

        sqlx::query_as::<_, FormTemplate>(
            r#"
            UPDATE form_templates SET
                name        = COALESCE($3, name),
                description = COALESCE($4, description),
                updated_at  = NOW()
            WHERE id = $1 AND tenant_id = $2
            RETURNING *
            "#,
        )
        .bind(id)
        .bind(tenant_id)
        .bind(req.name.as_deref())
        .bind(req.description.as_deref())
        .fetch_optional(pool)
        .await?
        .ok_or(FormError::TemplateNotFound)
    }
}

pub struct FieldRepo;

impl FieldRepo {
    /// Create a field at the end of the display_order sequence.
    pub async fn create(
        pool: &PgPool,
        template_id: Uuid,
        tenant_id: &str,
        req: &CreateFieldRequest,
    ) -> Result<FormField, FormError> {
        if req.field_key.trim().is_empty() {
            return Err(FormError::Validation("field_key is required".into()));
        }
        if req.field_label.trim().is_empty() {
            return Err(FormError::Validation("field_label is required".into()));
        }
        validate_field_type(&req.field_type)?;

        // Verify template exists and belongs to this tenant
        let tmpl_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM form_templates WHERE id = $1 AND tenant_id = $2")
                .bind(template_id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;
        if tmpl_exists.is_none() {
            return Err(FormError::TemplateNotFound);
        }

        // Compute next display_order (max + 1, or 0 if none)
        let max_order: Option<i32> = sqlx::query_scalar(
            "SELECT MAX(display_order) FROM form_fields \
                 WHERE template_id = $1 \
                   AND template_id IN (SELECT id FROM form_templates WHERE tenant_id = $2)",
        )
        .bind(template_id)
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;
        let next_order = max_order.map(|m| m + 1).unwrap_or(0);

        sqlx::query_as::<_, FormField>(
            r#"
            INSERT INTO form_fields
                (template_id, field_key, field_label, field_type,
                 validation_rules, pdf_position, display_order)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING *
            "#,
        )
        .bind(template_id)
        .bind(req.field_key.trim())
        .bind(req.field_label.trim())
        .bind(&req.field_type)
        .bind(
            req.validation_rules
                .as_ref()
                .unwrap_or(&serde_json::json!({})),
        )
        .bind(req.pdf_position.as_ref().unwrap_or(&serde_json::json!({})))
        .bind(next_order)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            if let sqlx::Error::Database(ref dbe) = e {
                if dbe.code().as_deref() == Some("23505") {
                    return FormError::DuplicateFieldKey;
                }
            }
            FormError::Database(e)
        })
    }

    /// List fields for a template, ordered by display_order ASC.
    pub async fn list(
        pool: &PgPool,
        template_id: Uuid,
        tenant_id: &str,
    ) -> Result<Vec<FormField>, FormError> {
        // Verify template belongs to tenant
        let tmpl_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM form_templates WHERE id = $1 AND tenant_id = $2")
                .bind(template_id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;
        if tmpl_exists.is_none() {
            return Err(FormError::TemplateNotFound);
        }

        sqlx::query_as::<_, FormField>(
            r#"
            SELECT * FROM form_fields
            WHERE template_id = $1
              AND template_id IN (SELECT id FROM form_templates WHERE tenant_id = $2)
            ORDER BY display_order ASC
            "#,
        )
        .bind(template_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await
        .map_err(FormError::Database)
    }

    /// List fields for a template (no tenant check — caller must verify ownership).
    pub async fn list_by_template(
        pool: &PgPool,
        template_id: Uuid,
    ) -> Result<Vec<FormField>, sqlx::Error> {
        sqlx::query_as::<_, FormField>(
            r#"
            SELECT * FROM form_fields
            WHERE template_id = $1
            ORDER BY display_order ASC
            "#,
        )
        .bind(template_id)
        .fetch_all(pool)
        .await
    }

    /// Update a field. Does not change display_order (use reorder for that).
    pub async fn update(
        pool: &PgPool,
        field_id: Uuid,
        template_id: Uuid,
        tenant_id: &str,
        req: &UpdateFieldRequest,
    ) -> Result<FormField, FormError> {
        if let Some(ref ft) = req.field_type {
            validate_field_type(ft)?;
        }
        if let Some(ref label) = req.field_label {
            if label.trim().is_empty() {
                return Err(FormError::Validation(
                    "field_label must not be empty".into(),
                ));
            }
        }

        // Verify template belongs to tenant
        let tmpl_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM form_templates WHERE id = $1 AND tenant_id = $2")
                .bind(template_id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;
        if tmpl_exists.is_none() {
            return Err(FormError::TemplateNotFound);
        }

        sqlx::query_as::<_, FormField>(
            r#"
            UPDATE form_fields SET
                field_label      = COALESCE($3, field_label),
                field_type       = COALESCE($4, field_type),
                validation_rules = COALESCE($5, validation_rules),
                pdf_position     = COALESCE($6, pdf_position)
            WHERE id = $1 AND template_id = $2
            RETURNING *
            "#,
        )
        .bind(field_id)
        .bind(template_id)
        .bind(req.field_label.as_deref())
        .bind(req.field_type.as_deref())
        .bind(&req.validation_rules)
        .bind(&req.pdf_position)
        .fetch_optional(pool)
        .await?
        .ok_or(FormError::FieldNotFound)
    }

    /// Reorder fields for a template. The field_ids vec defines the new order.
    /// All field IDs must belong to the template and the list must be exhaustive.
    pub async fn reorder(
        pool: &PgPool,
        template_id: Uuid,
        tenant_id: &str,
        req: &ReorderFieldsRequest,
    ) -> Result<Vec<FormField>, FormError> {
        if req.field_ids.is_empty() {
            return Err(FormError::Validation("field_ids must not be empty".into()));
        }

        // Verify template belongs to tenant
        let tmpl_exists: Option<(Uuid,)> =
            sqlx::query_as("SELECT id FROM form_templates WHERE id = $1 AND tenant_id = $2")
                .bind(template_id)
                .bind(tenant_id)
                .fetch_optional(pool)
                .await?;
        if tmpl_exists.is_none() {
            return Err(FormError::TemplateNotFound);
        }

        // Get existing field IDs
        let existing: Vec<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM form_fields \
             WHERE template_id = $1 \
               AND template_id IN (SELECT id FROM form_templates WHERE tenant_id = $2) \
             ORDER BY display_order",
        )
        .bind(template_id)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;
        let existing_ids: Vec<Uuid> = existing.into_iter().map(|(id,)| id).collect();

        // Validate: same set of IDs
        let mut req_sorted = req.field_ids.clone();
        req_sorted.sort();
        let mut existing_sorted = existing_ids.clone();
        existing_sorted.sort();
        if req_sorted != existing_sorted {
            return Err(FormError::Validation(
                "field_ids must contain exactly all fields of the template".into(),
            ));
        }

        // Update display_order in a transaction
        let mut tx = pool.begin().await?;
        for (i, field_id) in req.field_ids.iter().enumerate() {
            sqlx::query(
                "UPDATE form_fields SET display_order = $1 WHERE id = $2 AND template_id = $3",
            )
            .bind(i as i32)
            .bind(field_id)
            .bind(template_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;

        // Return the reordered list
        Self::list(pool, template_id, tenant_id).await
    }
}
