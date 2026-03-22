use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::guards::{
    guard_non_empty, guard_positive_quantity, guard_scrap_factor, GuardError,
};
use crate::domain::models::*;
use crate::events::{self, BomEventType};

pub use crate::domain::bom_queries::{explode, where_used};
use crate::domain::outbox::enqueue_event;

#[derive(Debug, Error)]
pub enum BomError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// BOM Header
// ---

pub async fn create_bom(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateBomRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<BomHeader, BomError> {
    guard_non_empty(tenant_id, "tenant_id")?;

    let mut tx = pool.begin().await?;

    let header = sqlx::query_as::<_, BomHeader>(
        r#"
        INSERT INTO bom_headers (tenant_id, part_id, description)
        VALUES ($1, $2, $3)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.part_id)
    .bind(&req.description)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::BomCreated,
        "bom_header",
        &header.id.to_string(),
        &events::build_bom_created_envelope(
            header.id,
            tenant_id.to_string(),
            req.part_id,
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(header)
}

pub async fn get_bom(
    pool: &PgPool,
    tenant_id: &str,
    bom_id: Uuid,
) -> Result<BomHeader, BomError> {
    sqlx::query_as::<_, BomHeader>(
        "SELECT * FROM bom_headers WHERE id = $1 AND tenant_id = $2",
    )
    .bind(bom_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound("BOM not found".to_string()).into())
}

pub async fn get_bom_by_part_id(
    pool: &PgPool,
    tenant_id: &str,
    part_id: Uuid,
) -> Result<BomHeader, BomError> {
    sqlx::query_as::<_, BomHeader>(
        "SELECT * FROM bom_headers WHERE part_id = $1 AND tenant_id = $2",
    )
    .bind(part_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound("No BOM found for this part".to_string()).into())
}

// BOM Revisions
// ---

pub async fn create_revision(
    pool: &PgPool,
    tenant_id: &str,
    bom_id: Uuid,
    req: &CreateRevisionRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<BomRevision, BomError> {
    guard_non_empty(&req.revision_label, "revision_label")?;

    // Guard: BOM must exist and belong to tenant
    let _header = get_bom(pool, tenant_id, bom_id).await?;

    let mut tx = pool.begin().await?;

    let revision = sqlx::query_as::<_, BomRevision>(
        r#"
        INSERT INTO bom_revisions (bom_id, tenant_id, revision_label)
        VALUES ($1, $2, $3)
        RETURNING *
        "#,
    )
    .bind(bom_id)
    .bind(tenant_id)
    .bind(&req.revision_label)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::RevisionCreated,
        "bom_revision",
        &revision.id.to_string(),
        &events::build_revision_created_envelope(
            revision.id,
            bom_id,
            tenant_id.to_string(),
            req.revision_label.clone(),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(revision)
}

pub async fn set_effectivity(
    pool: &PgPool,
    tenant_id: &str,
    revision_id: Uuid,
    req: &SetEffectivityRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<BomRevision, BomError> {
    let revision = get_revision(pool, tenant_id, revision_id).await?;

    if revision.status == "superseded" {
        return Err(GuardError::Validation(
            "Cannot set effectivity on a superseded revision".to_string(),
        )
        .into());
    }

    let mut tx = pool.begin().await?;

    // If setting to effective, supersede any other effective revisions for this BOM
    // whose date ranges would overlap (the DB exclusion index will also catch this,
    // but we give a clearer error by superseding proactively).
    if revision.status == "draft" {
        supersede_overlapping(&mut tx, tenant_id, revision.bom_id, req).await?;
    }

    let updated = sqlx::query_as::<_, BomRevision>(
        r#"
        UPDATE bom_revisions
        SET status = 'effective',
            effective_from = $1,
            effective_to = $2,
            updated_at = NOW()
        WHERE id = $3 AND tenant_id = $4
        RETURNING *
        "#,
    )
    .bind(req.effective_from)
    .bind(req.effective_to)
    .bind(revision_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::EffectivitySet,
        "bom_revision",
        &revision_id.to_string(),
        &events::build_effectivity_set_envelope(
            revision_id,
            revision.bom_id,
            tenant_id.to_string(),
            req.effective_from,
            req.effective_to,
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

async fn supersede_overlapping(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    bom_id: Uuid,
    req: &SetEffectivityRequest,
) -> Result<(), BomError> {
    // Find effective revisions that overlap the requested range
    sqlx::query(
        r#"
        UPDATE bom_revisions
        SET status = 'superseded', updated_at = NOW()
        WHERE bom_id = $1
          AND tenant_id = $2
          AND status = 'effective'
          AND tstzrange(effective_from, effective_to, '[)') &&
              tstzrange($3, $4, '[)')
        "#,
    )
    .bind(bom_id)
    .bind(tenant_id)
    .bind(req.effective_from)
    .bind(req.effective_to)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn get_revision(
    pool: &PgPool,
    tenant_id: &str,
    revision_id: Uuid,
) -> Result<BomRevision, BomError> {
    sqlx::query_as::<_, BomRevision>(
        "SELECT * FROM bom_revisions WHERE id = $1 AND tenant_id = $2",
    )
    .bind(revision_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound("Revision not found".to_string()).into())
}

pub async fn list_revisions(
    pool: &PgPool,
    tenant_id: &str,
    bom_id: Uuid,
) -> Result<Vec<BomRevision>, BomError> {
    let rows = sqlx::query_as::<_, BomRevision>(
        "SELECT * FROM bom_revisions WHERE bom_id = $1 AND tenant_id = $2 ORDER BY created_at",
    )
    .bind(bom_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

// BOM Lines
// ---

pub async fn add_line(
    pool: &PgPool,
    tenant_id: &str,
    revision_id: Uuid,
    req: &AddLineRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<BomLine, BomError> {
    guard_positive_quantity(req.quantity)?;
    guard_scrap_factor(req.scrap_factor)?;

    let revision = get_revision(pool, tenant_id, revision_id).await?;
    if revision.status != "draft" {
        return Err(
            GuardError::Validation("Can only add lines to draft revisions".to_string()).into(),
        );
    }

    let mut tx = pool.begin().await?;

    let line = sqlx::query_as::<_, BomLine>(
        r#"
        INSERT INTO bom_lines (revision_id, tenant_id, component_item_id, quantity, uom, scrap_factor, find_number)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, revision_id, tenant_id, component_item_id,
                  quantity::FLOAT8 AS quantity, uom,
                  scrap_factor::FLOAT8 AS scrap_factor,
                  find_number, created_at, updated_at
        "#,
    )
    .bind(revision_id)
    .bind(tenant_id)
    .bind(req.component_item_id)
    .bind(req.quantity)
    .bind(&req.uom)
    .bind(req.scrap_factor.unwrap_or(0.0))
    .bind(req.find_number)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::LineAdded,
        "bom_line",
        &line.id.to_string(),
        &events::build_line_added_envelope(
            line.id,
            revision_id,
            tenant_id.to_string(),
            req.component_item_id,
            req.quantity,
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(line)
}

pub async fn update_line(
    pool: &PgPool,
    tenant_id: &str,
    line_id: Uuid,
    req: &UpdateLineRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<BomLine, BomError> {
    if let Some(qty) = req.quantity {
        guard_positive_quantity(qty)?;
    }
    guard_scrap_factor(req.scrap_factor)?;

    let existing = fetch_line(pool, line_id, tenant_id).await?;

    let revision = get_revision(pool, tenant_id, existing.revision_id).await?;
    if revision.status != "draft" {
        return Err(
            GuardError::Validation("Can only update lines on draft revisions".to_string()).into(),
        );
    }

    let quantity = req.quantity.unwrap_or(existing.quantity);
    let scrap_factor = req.scrap_factor.unwrap_or(existing.scrap_factor.unwrap_or(0.0));
    let find_number = req.find_number.or(existing.find_number);

    let mut tx = pool.begin().await?;

    let line = sqlx::query_as::<_, BomLine>(
        r#"
        UPDATE bom_lines
        SET quantity = $1, uom = COALESCE($2, uom), scrap_factor = $3,
            find_number = $4, updated_at = NOW()
        WHERE id = $5 AND tenant_id = $6
        RETURNING id, revision_id, tenant_id, component_item_id,
                  quantity::FLOAT8 AS quantity, uom,
                  scrap_factor::FLOAT8 AS scrap_factor,
                  find_number, created_at, updated_at
        "#,
    )
    .bind(quantity)
    .bind(&req.uom)
    .bind(scrap_factor)
    .bind(find_number)
    .bind(line_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::LineUpdated,
        "bom_line",
        &line_id.to_string(),
        &events::build_line_updated_envelope(
            line_id,
            existing.revision_id,
            tenant_id.to_string(),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(line)
}

pub async fn remove_line(
    pool: &PgPool,
    tenant_id: &str,
    line_id: Uuid,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), BomError> {
    let existing = fetch_line(pool, line_id, tenant_id).await?;

    let revision = get_revision(pool, tenant_id, existing.revision_id).await?;
    if revision.status != "draft" {
        return Err(
            GuardError::Validation("Can only remove lines from draft revisions".to_string())
                .into(),
        );
    }

    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM bom_lines WHERE id = $1 AND tenant_id = $2")
        .bind(line_id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::LineRemoved,
        "bom_line",
        &line_id.to_string(),
        &events::build_line_removed_envelope(
            line_id,
            existing.revision_id,
            tenant_id.to_string(),
            existing.component_item_id,
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn list_lines(
    pool: &PgPool,
    tenant_id: &str,
    revision_id: Uuid,
) -> Result<Vec<BomLine>, BomError> {
    let rows = sqlx::query_as::<_, BomLine>(
        r#"
        SELECT id, revision_id, tenant_id, component_item_id,
               quantity::FLOAT8 AS quantity, uom,
               scrap_factor::FLOAT8 AS scrap_factor,
               find_number, created_at, updated_at
        FROM bom_lines
        WHERE revision_id = $1 AND tenant_id = $2
        ORDER BY find_number, created_at
        "#,
    )
    .bind(revision_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

async fn fetch_line(
    pool: &PgPool,
    line_id: Uuid,
    tenant_id: &str,
) -> Result<BomLine, BomError> {
    sqlx::query_as::<_, BomLine>(
        r#"
        SELECT id, revision_id, tenant_id, component_item_id,
               quantity::FLOAT8 AS quantity, uom,
               scrap_factor::FLOAT8 AS scrap_factor,
               find_number, created_at, updated_at
        FROM bom_lines
        WHERE id = $1 AND tenant_id = $2
        "#,
    )
    .bind(line_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound("BOM line not found".to_string()).into())
}
