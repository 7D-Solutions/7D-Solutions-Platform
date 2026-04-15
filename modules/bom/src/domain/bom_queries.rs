use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::guards::GuardError;
use crate::domain::models::*;

use super::bom_service::BomError;

const DEFAULT_MAX_DEPTH: i32 = 20;

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
    if !(1..=100).contains(&max_depth) {
        return Err(
            GuardError::Validation("max_depth must be between 1 and 100".to_string()).into(),
        );
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
