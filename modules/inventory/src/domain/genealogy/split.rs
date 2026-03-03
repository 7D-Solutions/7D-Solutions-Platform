//! Lot split service: Guard → Mutation → Outbox atomicity.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::helpers::{
    find_idempotency_key, find_lot, insert_genealogy_edge, lot_on_hand, upsert_lot_in_tx,
};
use super::{GenealogyError, GenealogyResult, LotSplitRequest};
use crate::events::{
    build_lot_split_envelope, LotSplitPayload, SplitChildEdge, EVENT_TYPE_LOT_SPLIT,
};

/// Process a lot split atomically.
///
/// Returns `(GenealogyResult, is_replay)`.
pub async fn process_split(
    pool: &PgPool,
    req: &LotSplitRequest,
) -> Result<(GenealogyResult, bool), GenealogyError> {
    // --- Guard: stateless validation ---
    validate_split_request(req)?;

    // --- Idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(GenealogyError::ConflictingIdempotencyKey);
        }
        let result: GenealogyResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: parent lot must exist ---
    let parent = find_lot(pool, &req.tenant_id, req.item_id, &req.parent_lot_code)
        .await?
        .ok_or_else(|| GenealogyError::LotNotFound(req.parent_lot_code.clone()))?;

    // --- Guard: quantity conservation ---
    let parent_qty = lot_on_hand(pool, &req.tenant_id, parent.id).await?;
    let children_sum: i64 = req.children.iter().map(|c| c.quantity).sum();
    if children_sum != parent_qty {
        return Err(GenealogyError::QuantityConservation {
            children_sum,
            parent_qty,
        });
    }

    // --- Guard: child lot codes must not include parent ---
    for child in &req.children {
        if child.lot_code == req.parent_lot_code {
            return Err(GenealogyError::Validation(format!(
                "child lot_code '{}' cannot be the same as parent",
                child.lot_code
            )));
        }
    }

    // --- Atomic transaction ---
    let operation_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let occurred_at = Utc::now();
    let correlation_id = req
        .correlation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut tx = pool.begin().await?;

    // Step 1: Upsert child lots and insert genealogy edges
    let mut child_edges = Vec::with_capacity(req.children.len());
    for child in &req.children {
        let child_lot_id =
            upsert_lot_in_tx(&mut tx, &req.tenant_id, req.item_id, &child.lot_code).await?;

        insert_genealogy_edge(
            &mut tx,
            &req.tenant_id,
            operation_id,
            parent.id,
            child_lot_id,
            "split",
            child.quantity,
            occurred_at,
            req.actor_id,
            req.notes.as_deref(),
        )
        .await?;

        child_edges.push(SplitChildEdge {
            child_lot_id,
            child_lot_code: child.lot_code.clone(),
            quantity: child.quantity,
        });
    }

    // Step 2: Build event and enqueue in outbox
    let payload = LotSplitPayload {
        operation_id,
        tenant_id: req.tenant_id.clone(),
        parent_lot_id: parent.id,
        parent_lot_code: parent.lot_code.clone(),
        item_id: req.item_id,
        children: child_edges,
        actor_id: req.actor_id,
        occurred_at,
    };

    let envelope = build_lot_split_envelope(
        event_id,
        req.tenant_id.clone(),
        correlation_id.clone(),
        req.causation_id.clone(),
        payload,
    );
    let envelope_json = serde_json::to_string(&envelope)?;

    sqlx::query(
        r#"
        INSERT INTO inv_outbox
            (event_id, event_type, aggregate_type, aggregate_id, tenant_id,
             payload, correlation_id, causation_id, schema_version)
        VALUES
            ($1, $2, 'inventory_lot', $3, $4, $5::JSONB, $6, $7, '1.0.0')
        "#,
    )
    .bind(event_id)
    .bind(EVENT_TYPE_LOT_SPLIT)
    .bind(parent.id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 3: Build result and store idempotency key
    let result = GenealogyResult {
        operation_id,
        edge_count: req.children.len(),
        event_id,
    };

    let response_json = serde_json::to_string(&result)?;
    let expires_at = occurred_at + Duration::days(7);

    sqlx::query(
        r#"
        INSERT INTO inv_idempotency_keys
            (tenant_id, idempotency_key, request_hash, response_body, status_code, expires_at)
        VALUES
            ($1, $2, $3, $4::JSONB, 201, $5)
        "#,
    )
    .bind(&req.tenant_id)
    .bind(&req.idempotency_key)
    .bind(&request_hash)
    .bind(&response_json)
    .bind(expires_at)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok((result, false))
}

fn validate_split_request(req: &LotSplitRequest) -> Result<(), GenealogyError> {
    if req.tenant_id.trim().is_empty() {
        return Err(GenealogyError::Validation(
            "tenant_id is required".to_string(),
        ));
    }
    if req.idempotency_key.trim().is_empty() {
        return Err(GenealogyError::Validation(
            "idempotency_key is required".to_string(),
        ));
    }
    if req.parent_lot_code.trim().is_empty() {
        return Err(GenealogyError::Validation(
            "parent_lot_code is required".to_string(),
        ));
    }
    if req.children.is_empty() {
        return Err(GenealogyError::Validation(
            "at least one child lot is required".to_string(),
        ));
    }
    for (i, child) in req.children.iter().enumerate() {
        if child.lot_code.trim().is_empty() {
            return Err(GenealogyError::Validation(format!(
                "children[{}].lot_code is required",
                i
            )));
        }
        if child.quantity <= 0 {
            return Err(GenealogyError::Validation(format!(
                "children[{}].quantity must be > 0",
                i
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_split_req() -> LotSplitRequest {
        LotSplitRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            parent_lot_code: "LOT-PARENT".to_string(),
            children: vec![super::super::SplitChild {
                lot_code: "LOT-CHILD-1".to_string(),
                quantity: 5,
            }],
            actor_id: None,
            notes: None,
            idempotency_key: "split-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn split_rejects_empty_tenant() {
        let mut r = valid_split_req();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_split_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn split_rejects_empty_idempotency_key() {
        let mut r = valid_split_req();
        r.idempotency_key = "  ".to_string();
        assert!(matches!(
            validate_split_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn split_rejects_empty_children() {
        let mut r = valid_split_req();
        r.children = vec![];
        assert!(matches!(
            validate_split_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn split_rejects_zero_quantity() {
        let mut r = valid_split_req();
        r.children[0].quantity = 0;
        assert!(matches!(
            validate_split_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn split_accepts_valid_request() {
        assert!(validate_split_request(&valid_split_req()).is_ok());
    }
}
