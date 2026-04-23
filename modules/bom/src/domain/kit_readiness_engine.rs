use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::bom_queries;
use crate::domain::bom_service::BomError;
use crate::domain::guards::GuardError;
use crate::domain::inventory_client::InventoryClient;
use crate::domain::models::{
    ExplosionQuery, KitReadinessCheckRequest, KitReadinessLine, KitReadinessResult,
    KitReadinessSnapshot,
};
use crate::domain::outbox;
use crate::events::{self, BomEventType};
use platform_sdk::VerifiedClaims;

pub async fn check(
    pool: &PgPool,
    tenant_id: &str,
    req: &KitReadinessCheckRequest,
    inventory: &InventoryClient,
    claims: Option<&VerifiedClaims>,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<KitReadinessResult, BomError> {
    if req.required_quantity <= 0.0 {
        return Err(
            GuardError::Validation("required_quantity must be positive".to_string()).into(),
        );
    }

    // Fetch the root assembly's part_id to seed demand propagation
    let root_part_id: Uuid =
        sqlx::query_scalar("SELECT part_id FROM bom_headers WHERE id = $1 AND tenant_id = $2")
            .bind(req.bom_id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| GuardError::NotFound(format!("BOM {} not found", req.bom_id)))?;

    let explosion_rows = bom_queries::explode(
        pool,
        tenant_id,
        req.bom_id,
        &ExplosionQuery {
            date: Some(req.check_date),
            max_depth: None,
        },
    )
    .await?;

    // Propagate demand top-down (same algorithm as MRP engine)
    let mut parent_effective: HashMap<Uuid, f64> = HashMap::new();
    parent_effective.insert(root_part_id, req.required_quantity);

    struct ExplodedLine {
        component_item_id: Uuid,
        gross_qty: f64,
    }

    let mut exploded: Vec<ExplodedLine> = Vec::with_capacity(explosion_rows.len());
    for row in &explosion_rows {
        let parent_demand = parent_effective
            .get(&row.parent_part_id)
            .copied()
            .unwrap_or(req.required_quantity);
        let gross_qty = parent_demand * row.quantity;
        let scrap_adj = gross_qty * (1.0 + row.scrap_factor);
        parent_effective.insert(row.component_item_id, scrap_adj);
        exploded.push(ExplodedLine {
            component_item_id: row.component_item_id,
            gross_qty: scrap_adj,
        });
    }

    // Fetch availability for each unique component (cached per item)
    let mut avail_cache: HashMap<Uuid, crate::domain::inventory_client::AvailabilityInfo> =
        HashMap::new();
    for line in &exploded {
        if !avail_cache.contains_key(&line.component_item_id) {
            let info = inventory
                .fetch_availability(claims, tenant_id, line.component_item_id)
                .await?;
            avail_cache.insert(line.component_item_id, info);
        }
    }

    // Build per-component readiness lines
    let mut kit_lines: Vec<KitReadinessLine> = Vec::with_capacity(exploded.len());
    let mut ready_count = 0usize;

    for line in &exploded {
        let avail = avail_cache
            .get(&line.component_item_id)
            .expect("cache populated above");

        let status = if avail.available_qty >= line.gross_qty as i64 {
            ready_count += 1;
            "ready"
        } else if avail.quarantine_qty > 0 {
            "quarantined"
        } else if avail.expired_qty > 0 {
            "expired"
        } else {
            "short"
        };

        kit_lines.push(KitReadinessLine {
            component_item_id: line.component_item_id,
            required_qty: line.gross_qty,
            on_hand_qty: avail.on_hand_qty,
            expired_qty: avail.expired_qty,
            available_qty: avail.available_qty,
            status: status.to_string(),
        });
    }

    let overall_status = if kit_lines.is_empty() || ready_count == kit_lines.len() {
        "ready"
    } else if ready_count == 0 {
        "not_ready"
    } else {
        "partial"
    };

    // Collect issue summary for shortfall lines
    let issue_summary: Vec<serde_json::Value> = kit_lines
        .iter()
        .filter(|l| l.status != "ready")
        .map(|l| {
            serde_json::json!({
                "component_item_id": l.component_item_id,
                "status": l.status,
                "required_qty": l.required_qty,
                "available_qty": l.available_qty,
            })
        })
        .collect();

    // Persist snapshot + lines + outbox event atomically
    let mut tx = pool.begin().await?;

    let snapshot: KitReadinessSnapshot = sqlx::query_as(
        r#"
        INSERT INTO kit_readiness_snapshots
            (tenant_id, bom_id, required_quantity, check_date, overall_status,
             issue_summary, created_by)
        VALUES ($1, $2, $3, $4, $5, $6::JSONB, $7)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(req.bom_id)
    .bind(req.required_quantity)
    .bind(req.check_date)
    .bind(overall_status)
    .bind(serde_json::to_value(&issue_summary)?)
    .bind(&req.created_by)
    .fetch_one(&mut *tx)
    .await?;

    for line in &kit_lines {
        sqlx::query(
            r#"
            INSERT INTO kit_readiness_lines
                (snapshot_id, component_item_id, required_qty,
                 on_hand_qty, expired_qty, available_qty, status)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(snapshot.id)
        .bind(line.component_item_id)
        .bind(line.required_qty)
        .bind(line.on_hand_qty)
        .bind(line.expired_qty)
        .bind(line.available_qty)
        .bind(&line.status)
        .execute(&mut *tx)
        .await?;
    }

    let envelope = events::build_kit_readiness_checked_envelope(
        snapshot.id,
        req.bom_id,
        overall_status.to_string(),
        tenant_id.to_string(),
        correlation_id.to_string(),
        causation_id.map(str::to_string),
    );
    outbox::enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::KitReadinessChecked,
        "kit_readiness_snapshot",
        &snapshot.id.to_string(),
        &envelope,
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;

    Ok(KitReadinessResult {
        snapshot,
        lines: kit_lines,
    })
}

pub async fn get_snapshot(
    pool: &PgPool,
    tenant_id: &str,
    snapshot_id: Uuid,
) -> Result<KitReadinessResult, BomError> {
    let snapshot: KitReadinessSnapshot =
        sqlx::query_as("SELECT * FROM kit_readiness_snapshots WHERE id = $1 AND tenant_id = $2")
            .bind(snapshot_id)
            .bind(tenant_id)
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| {
                GuardError::NotFound(format!("Kit readiness snapshot {} not found", snapshot_id))
            })?;

    // Lines are stored in kit_readiness_lines but KitReadinessLine doesn't derive FromRow
    // (it omits the id and snapshot_id PK). Query the fields explicitly.
    let rows: Vec<(Uuid, f64, i64, i64, i64, String)> = sqlx::query_as(
        r#"SELECT component_item_id, required_qty, on_hand_qty, expired_qty, available_qty, status
           FROM kit_readiness_lines WHERE snapshot_id = $1 ORDER BY id"#,
    )
    .bind(snapshot_id)
    .fetch_all(pool)
    .await?;

    let lines = rows
        .into_iter()
        .map(
            |(component_item_id, required_qty, on_hand_qty, expired_qty, available_qty, status)| {
                KitReadinessLine {
                    component_item_id,
                    required_qty,
                    on_hand_qty,
                    expired_qty,
                    available_qty,
                    status,
                }
            },
        )
        .collect();

    Ok(KitReadinessResult { snapshot, lines })
}
