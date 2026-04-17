use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::bom_queries;
use crate::domain::bom_service::BomError;
use crate::domain::guards::GuardError;
use crate::domain::models::*;
use crate::domain::outbox;
use crate::events::{self, BomEventType};

// ============================================================================
// Explode: run BOM explosion, apply scrap factors, subtract on-hand, persist
// ============================================================================

pub async fn explode(
    pool: &PgPool,
    tenant_id: &str,
    req: &MrpExplodeRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<MrpSnapshotWithLines, BomError> {
    if req.demand_quantity <= 0.0 {
        return Err(GuardError::Validation("demand_quantity must be positive".to_string()).into());
    }

    // Fetch the root assembly's part_id to seed the cascaded-quantity map
    let root_part_id: Uuid = sqlx::query_scalar(
        "SELECT part_id FROM bom_headers WHERE id = $1 AND tenant_id = $2",
    )
    .bind(req.bom_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound(format!("BOM {} not found", req.bom_id)))?;

    // Run the existing multi-level BOM explosion filtered by effectivity_date
    let explosion_query = ExplosionQuery {
        date: Some(req.effectivity_date),
        max_depth: None,
    };
    let explosion_rows =
        bom_queries::explode(pool, tenant_id, req.bom_id, &explosion_query).await?;

    // Build on-hand lookup (caller-supplied, deterministic)
    let on_hand_map: HashMap<Uuid, f64> = req.on_hand.iter().map(|e| (e.item_id, e.quantity)).collect();

    // Cascaded effective demand: tracks the scrap-adjusted quantity needed for each
    // assembly as we walk the tree top-down. Children use their parent's value.
    // Invariant: scrap must be applied BEFORE on-hand subtraction (bead spec §how_to_think).
    let mut parent_effective: HashMap<Uuid, f64> = HashMap::new();
    parent_effective.insert(root_part_id, req.demand_quantity);

    let mut staged_lines: Vec<MrpRequirementLine> = Vec::with_capacity(explosion_rows.len());
    let mut net_shortage_count: i64 = 0;

    for row in &explosion_rows {
        let parent_demand = parent_effective
            .get(&row.parent_part_id)
            .copied()
            .unwrap_or(req.demand_quantity);

        let gross_quantity = parent_demand * row.quantity;
        let scrap_adjusted_quantity = gross_quantity * (1.0 + row.scrap_factor);
        let on_hand_quantity = on_hand_map.get(&row.component_item_id).copied().unwrap_or(0.0);
        let net_quantity = (scrap_adjusted_quantity - on_hand_quantity).max(0.0);

        if net_quantity > 0.0 {
            net_shortage_count += 1;
        }

        // Propagate this component's scrap-adjusted demand to its children
        parent_effective.insert(row.component_item_id, scrap_adjusted_quantity);

        staged_lines.push(MrpRequirementLine {
            id: 0,
            snapshot_id: Uuid::nil(),
            level: row.level,
            parent_part_id: row.parent_part_id,
            component_item_id: row.component_item_id,
            gross_quantity,
            scrap_factor: row.scrap_factor,
            scrap_adjusted_quantity,
            on_hand_quantity,
            net_quantity,
            uom: row.uom.clone(),
            revision_id: row.revision_id,
            revision_label: row.revision_label.clone(),
        });
    }

    // Persist snapshot + lines + outbox event atomically
    let on_hand_json = serde_json::to_value(&req.on_hand)?;

    let mut tx = pool.begin().await?;

    let snapshot: MrpSnapshot = sqlx::query_as(
        r#"
        INSERT INTO mrp_snapshots
            (tenant_id, bom_id, demand_quantity, effectivity_date, on_hand_snapshot, created_by)
        VALUES ($1, $2, $3, $4, $5::JSONB, $6)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.bom_id)
    .bind(req.demand_quantity)
    .bind(req.effectivity_date)
    .bind(serde_json::to_string(&on_hand_json)?)
    .bind(&req.created_by)
    .fetch_one(&mut *tx)
    .await?;

    let mut inserted_lines: Vec<MrpRequirementLine> = Vec::with_capacity(staged_lines.len());
    for line in &staged_lines {
        let inserted: MrpRequirementLine = sqlx::query_as(
            r#"
            INSERT INTO mrp_requirement_lines
                (snapshot_id, level, parent_part_id, component_item_id,
                 gross_quantity, scrap_factor, scrap_adjusted_quantity,
                 on_hand_quantity, net_quantity, uom, revision_id, revision_label)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING *
            "#,
        )
        .bind(snapshot.id)
        .bind(line.level)
        .bind(line.parent_part_id)
        .bind(line.component_item_id)
        .bind(line.gross_quantity)
        .bind(line.scrap_factor)
        .bind(line.scrap_adjusted_quantity)
        .bind(line.on_hand_quantity)
        .bind(line.net_quantity)
        .bind(&line.uom)
        .bind(line.revision_id)
        .bind(&line.revision_label)
        .fetch_one(&mut *tx)
        .await?;
        inserted_lines.push(inserted);
    }

    let line_count = inserted_lines.len() as i64;
    let envelope = events::build_mrp_exploded_envelope(
        snapshot.id,
        req.bom_id,
        req.demand_quantity,
        line_count,
        net_shortage_count,
        tenant_id.to_string(),
        correlation_id.to_string(),
        causation_id.map(str::to_string),
    );
    outbox::enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::MrpExploded,
        "mrp_snapshot",
        &snapshot.id.to_string(),
        &envelope,
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;

    Ok(MrpSnapshotWithLines {
        snapshot,
        lines: inserted_lines,
    })
}

// ============================================================================
// Queries
// ============================================================================

pub async fn get_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    snapshot_id: Uuid,
) -> Result<MrpSnapshotWithLines, BomError> {
    let snapshot: MrpSnapshot = sqlx::query_as(
        "SELECT * FROM mrp_snapshots WHERE id = $1 AND tenant_id = $2",
    )
    .bind(snapshot_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound(format!("MRP snapshot {} not found", snapshot_id)))?;

    let lines: Vec<MrpRequirementLine> = sqlx::query_as(
        "SELECT * FROM mrp_requirement_lines WHERE snapshot_id = $1 ORDER BY level, id",
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?;

    Ok(MrpSnapshotWithLines { snapshot, lines })
}

pub async fn list_snapshots(
    pool: &PgPool,
    tenant_id: &str,
    query: &MrpSnapshotListQuery,
) -> Result<Vec<MrpSnapshot>, BomError> {
    let snapshots: Vec<MrpSnapshot> = if let Some(bom_id) = query.bom_id {
        sqlx::query_as(
            "SELECT * FROM mrp_snapshots WHERE tenant_id = $1 AND bom_id = $2 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .bind(bom_id)
        .fetch_all(pool)
        .await?
    } else {
        sqlx::query_as(
            "SELECT * FROM mrp_snapshots WHERE tenant_id = $1 ORDER BY created_at DESC",
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await?
    };
    Ok(snapshots)
}
