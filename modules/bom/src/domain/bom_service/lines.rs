use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::guards::{guard_positive_quantity, guard_scrap_factor, GuardError};
use crate::domain::inventory_client::InventoryClient;
use crate::domain::models::*;
use crate::domain::outbox::enqueue_event;
use crate::events::{self, BomEventType};
use platform_sdk::VerifiedClaims;

use super::headers::get_revision;
use super::BomError;

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
    let scrap_factor = req
        .scrap_factor
        .unwrap_or(existing.scrap_factor.unwrap_or(0.0));
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
        return Err(GuardError::Validation(
            "Can only remove lines from draft revisions".to_string(),
        )
        .into());
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

/// List BOM lines with embedded inventory item details.
///
/// For each line, fetches the corresponding item from the inventory service.
/// Unresolvable `component_item_id` values produce `item: null` — never an error.
pub async fn list_lines_enriched(
    pool: &PgPool,
    tenant_id: &str,
    revision_id: Uuid,
    inventory: &InventoryClient,
    claims: &VerifiedClaims,
) -> Result<Vec<BomLineEnriched>, BomError> {
    let lines = list_lines(pool, tenant_id, revision_id).await?;

    let mut enriched = Vec::with_capacity(lines.len());
    for line in lines {
        let item = inventory
            .fetch_item_details(claims, tenant_id, line.component_item_id)
            .await?;
        enriched.push(BomLineEnriched {
            id: line.id,
            revision_id: line.revision_id,
            tenant_id: line.tenant_id.clone(),
            component_item_id: line.component_item_id,
            quantity: line.quantity,
            uom: line.uom.clone(),
            scrap_factor: line.scrap_factor,
            find_number: line.find_number,
            created_at: line.created_at,
            updated_at: line.updated_at,
            item,
        });
    }
    Ok(enriched)
}

async fn fetch_line(pool: &PgPool, line_id: Uuid, tenant_id: &str) -> Result<BomLine, BomError> {
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
