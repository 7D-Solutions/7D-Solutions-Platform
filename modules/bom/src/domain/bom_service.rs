use chrono::Utc;
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

use crate::domain::guards::{
    guard_non_empty, guard_positive_quantity, guard_scrap_factor, GuardError,
};
use crate::domain::models::*;
use crate::events::{self, BomEventType};

const DEFAULT_MAX_DEPTH: i32 = 20;

#[derive(Debug, Error)]
pub enum BomError {
    #[error("Guard failed: {0}")]
    Guard(#[from] GuardError),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
}

// ============================================================================
// BOM Header
// ============================================================================

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

// ============================================================================
// BOM Revisions
// ============================================================================

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

// ============================================================================
// BOM Lines
// ============================================================================

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

// ============================================================================
// Explosion (multi-level BOM flattening with depth guard + cycle detection)
// ============================================================================

pub async fn explode(
    pool: &PgPool,
    tenant_id: &str,
    bom_id: Uuid,
    query: &ExplosionQuery,
) -> Result<Vec<ExplosionRow>, BomError> {
    let max_depth = query.max_depth.unwrap_or(DEFAULT_MAX_DEPTH);
    if max_depth < 1 || max_depth > 100 {
        return Err(GuardError::Validation(
            "max_depth must be between 1 and 100".to_string(),
        )
        .into());
    }

    let date = query.date.unwrap_or_else(Utc::now);

    // Recursive CTE with cycle detection via path array
    let rows = sqlx::query_as::<_, ExplosionDbRow>(
        r#"
        WITH RECURSIVE bom_tree AS (
            -- Anchor: top-level BOM lines
            SELECT
                1 AS level,
                h.part_id AS parent_part_id,
                l.component_item_id,
                l.quantity::FLOAT8 AS quantity,
                l.uom,
                COALESCE(l.scrap_factor::FLOAT8, 0) AS scrap_factor,
                r.id AS revision_id,
                r.revision_label,
                ARRAY[h.part_id, l.component_item_id] AS path
            FROM bom_headers h
            JOIN bom_revisions r ON r.bom_id = h.id AND r.tenant_id = $1
            JOIN bom_lines l ON l.revision_id = r.id AND l.tenant_id = $1
            WHERE h.id = $2
              AND h.tenant_id = $1
              AND r.status = 'effective'
              AND r.effective_from <= $3
              AND (r.effective_to IS NULL OR r.effective_to > $3)

            UNION ALL

            -- Recursive: expand each component that itself has a BOM
            SELECT
                bt.level + 1,
                h2.part_id,
                l2.component_item_id,
                l2.quantity::FLOAT8,
                l2.uom,
                COALESCE(l2.scrap_factor::FLOAT8, 0),
                r2.id,
                r2.revision_label,
                bt.path || l2.component_item_id
            FROM bom_tree bt
            JOIN bom_headers h2 ON h2.part_id = bt.component_item_id AND h2.tenant_id = $1
            JOIN bom_revisions r2 ON r2.bom_id = h2.id AND r2.tenant_id = $1
            JOIN bom_lines l2 ON l2.revision_id = r2.id AND l2.tenant_id = $1
            WHERE r2.status = 'effective'
              AND r2.effective_from <= $3
              AND (r2.effective_to IS NULL OR r2.effective_to > $3)
              AND bt.level < $4
              AND NOT (l2.component_item_id = ANY(bt.path))
        )
        SELECT level, parent_part_id, component_item_id, quantity, uom,
               scrap_factor, revision_id, revision_label
        FROM bom_tree
        ORDER BY level, parent_part_id
        "#,
    )
    .bind(tenant_id)
    .bind(bom_id)
    .bind(date)
    .bind(max_depth)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct ExplosionDbRow {
    level: i32,
    parent_part_id: Uuid,
    component_item_id: Uuid,
    quantity: f64,
    uom: Option<String>,
    scrap_factor: f64,
    revision_id: Uuid,
    revision_label: String,
}

impl From<ExplosionDbRow> for ExplosionRow {
    fn from(r: ExplosionDbRow) -> Self {
        ExplosionRow {
            level: r.level,
            parent_part_id: r.parent_part_id,
            component_item_id: r.component_item_id,
            quantity: r.quantity,
            uom: r.uom,
            scrap_factor: r.scrap_factor,
            revision_id: r.revision_id,
            revision_label: r.revision_label,
        }
    }
}

// ============================================================================
// Where-Used (reverse lookup)
// ============================================================================

pub async fn where_used(
    pool: &PgPool,
    tenant_id: &str,
    item_id: Uuid,
    query: &WhereUsedQuery,
) -> Result<Vec<WhereUsedRow>, BomError> {
    let date = query.date.unwrap_or_else(Utc::now);

    let rows = sqlx::query_as::<_, WhereUsedDbRow>(
        r#"
        SELECT h.id AS bom_id, h.part_id, r.id AS revision_id,
               r.revision_label, l.quantity::FLOAT8 AS quantity, l.uom
        FROM bom_lines l
        JOIN bom_revisions r ON r.id = l.revision_id AND r.tenant_id = $1
        JOIN bom_headers h ON h.id = r.bom_id AND h.tenant_id = $1
        WHERE l.component_item_id = $2
          AND l.tenant_id = $1
          AND r.status = 'effective'
          AND r.effective_from <= $3
          AND (r.effective_to IS NULL OR r.effective_to > $3)
        ORDER BY h.part_id
        "#,
    )
    .bind(tenant_id)
    .bind(item_id)
    .bind(date)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

#[derive(sqlx::FromRow)]
struct WhereUsedDbRow {
    bom_id: Uuid,
    part_id: Uuid,
    revision_id: Uuid,
    revision_label: String,
    quantity: f64,
    uom: Option<String>,
}

impl From<WhereUsedDbRow> for WhereUsedRow {
    fn from(r: WhereUsedDbRow) -> Self {
        WhereUsedRow {
            bom_id: r.bom_id,
            part_id: r.part_id,
            revision_id: r.revision_id,
            revision_label: r.revision_label,
            quantity: r.quantity,
            uom: r.uom,
        }
    }
}

// ============================================================================
// Outbox helper
// ============================================================================

async fn enqueue_event<T: serde::Serialize>(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    tenant_id: &str,
    event_type: BomEventType,
    aggregate_type: &str,
    aggregate_id: &str,
    envelope: &event_bus::EventEnvelope<T>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<(), BomError> {
    let envelope_json = serde_json::to_string(envelope)?;

    sqlx::query(
        r#"
        INSERT INTO bom_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES
            ($1, $2, $3, $4, $5, $6::JSONB, $7, $8, '1.0.0')
        "#,
    )
    .bind(envelope.event_id)
    .bind(event_type.as_str())
    .bind(aggregate_type)
    .bind(aggregate_id)
    .bind(tenant_id)
    .bind(&envelope_json)
    .bind(correlation_id)
    .bind(causation_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}
