//! Lot merge service: Guard → Mutation → Outbox atomicity.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::helpers::{find_idempotency_key, find_lot, insert_genealogy_edge, upsert_lot_in_tx};
use super::{GenealogyError, GenealogyResult, LotMergeRequest};
use crate::events::{
    build_lot_merged_envelope, LotMergedPayload, MergeParentEdge, EVENT_TYPE_LOT_MERGED,
};

/// Process a lot merge atomically.
///
/// Returns `(GenealogyResult, is_replay)`.
pub async fn process_merge(
    pool: &PgPool,
    req: &LotMergeRequest,
) -> Result<(GenealogyResult, bool), GenealogyError> {
    // --- Guard: stateless validation ---
    validate_merge_request(req)?;

    // --- Idempotency check ---
    let request_hash = serde_json::to_string(req)?;
    if let Some(record) = find_idempotency_key(pool, &req.tenant_id, &req.idempotency_key).await? {
        if record.request_hash != request_hash {
            return Err(GenealogyError::ConflictingIdempotencyKey);
        }
        let result: GenealogyResult = serde_json::from_str(&record.response_body)?;
        return Ok((result, true));
    }

    // --- Guard: all parent lots must exist and not include child ---
    let mut parent_rows = Vec::with_capacity(req.parents.len());
    for parent in &req.parents {
        if parent.lot_code == req.child_lot_code {
            return Err(GenealogyError::Validation(format!(
                "parent lot_code '{}' cannot be the same as child",
                parent.lot_code
            )));
        }
        let row = find_lot(pool, &req.tenant_id, req.item_id, &parent.lot_code)
            .await?
            .ok_or_else(|| GenealogyError::LotNotFound(parent.lot_code.clone()))?;
        parent_rows.push((row, parent.quantity));
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

    // Step 1: Upsert child lot
    let child_lot_id =
        upsert_lot_in_tx(&mut tx, &req.tenant_id, req.item_id, &req.child_lot_code).await?;

    // Step 2: Insert genealogy edges for each parent
    let mut parent_edges = Vec::with_capacity(parent_rows.len());
    for (parent_row, qty) in &parent_rows {
        insert_genealogy_edge(
            &mut tx,
            &req.tenant_id,
            operation_id,
            parent_row.id,
            child_lot_id,
            "merge",
            *qty,
            occurred_at,
            req.actor_id,
            req.notes.as_deref(),
        )
        .await?;

        parent_edges.push(MergeParentEdge {
            parent_lot_id: parent_row.id,
            parent_lot_code: parent_row.lot_code.clone(),
            quantity: *qty,
        });
    }

    // Step 3: Build event and enqueue in outbox
    let payload = LotMergedPayload {
        operation_id,
        tenant_id: req.tenant_id.clone(),
        child_lot_id,
        child_lot_code: req.child_lot_code.clone(),
        item_id: req.item_id,
        parents: parent_edges,
        actor_id: req.actor_id,
        occurred_at,
    };

    let envelope = build_lot_merged_envelope(
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
    .bind(EVENT_TYPE_LOT_MERGED)
    .bind(child_lot_id.to_string())
    .bind(&req.tenant_id)
    .bind(&envelope_json)
    .bind(&correlation_id)
    .bind(&req.causation_id)
    .execute(&mut *tx)
    .await?;

    // Step 4: Build result and store idempotency key
    let result = GenealogyResult {
        operation_id,
        edge_count: req.parents.len(),
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

fn validate_merge_request(req: &LotMergeRequest) -> Result<(), GenealogyError> {
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
    if req.child_lot_code.trim().is_empty() {
        return Err(GenealogyError::Validation(
            "child_lot_code is required".to_string(),
        ));
    }
    if req.parents.is_empty() {
        return Err(GenealogyError::Validation(
            "at least one parent lot is required".to_string(),
        ));
    }
    for (i, parent) in req.parents.iter().enumerate() {
        if parent.lot_code.trim().is_empty() {
            return Err(GenealogyError::Validation(format!(
                "parents[{}].lot_code is required",
                i
            )));
        }
        if parent.quantity <= 0 {
            return Err(GenealogyError::Validation(format!(
                "parents[{}].quantity must be > 0",
                i
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_merge_req() -> LotMergeRequest {
        LotMergeRequest {
            tenant_id: "tenant-1".to_string(),
            item_id: Uuid::new_v4(),
            parents: vec![super::super::MergeParent {
                lot_code: "LOT-P1".to_string(),
                quantity: 10,
            }],
            child_lot_code: "LOT-MERGED".to_string(),
            actor_id: None,
            notes: None,
            idempotency_key: "merge-001".to_string(),
            correlation_id: None,
            causation_id: None,
        }
    }

    #[test]
    fn merge_rejects_empty_tenant() {
        let mut r = valid_merge_req();
        r.tenant_id = "".to_string();
        assert!(matches!(
            validate_merge_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn merge_rejects_empty_parents() {
        let mut r = valid_merge_req();
        r.parents = vec![];
        assert!(matches!(
            validate_merge_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn merge_rejects_zero_quantity() {
        let mut r = valid_merge_req();
        r.parents[0].quantity = 0;
        assert!(matches!(
            validate_merge_request(&r),
            Err(GenealogyError::Validation(_))
        ));
    }

    #[test]
    fn merge_accepts_valid_request() {
        assert!(validate_merge_request(&valid_merge_req()).is_ok());
    }
}
