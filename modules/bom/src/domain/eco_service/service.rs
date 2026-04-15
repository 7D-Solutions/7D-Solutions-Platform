use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::eco_models::*;
use crate::domain::guards::GuardError;
use crate::domain::outbox::enqueue_event;
use crate::events::{self, BomEventType};

use super::{get_eco, insert_audit};
use crate::domain::bom_service::BomError;

// ============================================================================
// ECO linkage
// ============================================================================

pub async fn link_bom_revision(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
    req: &LinkBomRevisionRequest,
) -> Result<EcoBomRevision, BomError> {
    let eco = get_eco(pool, tenant_id, eco_id).await?;
    if eco.status == "applied" || eco.status == "rejected" {
        return Err(
            GuardError::Validation("Cannot link revisions to a closed ECO".to_string()).into(),
        );
    }

    // Verify both revisions exist and belong to the same BOM
    let before =
        crate::domain::bom_service::get_revision(pool, tenant_id, req.before_revision_id).await?;
    let after =
        crate::domain::bom_service::get_revision(pool, tenant_id, req.after_revision_id).await?;

    if before.bom_id != req.bom_id || after.bom_id != req.bom_id {
        return Err(GuardError::Validation(
            "Revisions must belong to the specified BOM".to_string(),
        )
        .into());
    }

    if after.status != "draft" {
        return Err(
            GuardError::Validation("After-revision must be in draft status".to_string()).into(),
        );
    }

    let link = sqlx::query_as::<_, EcoBomRevision>(
        r#"
        INSERT INTO eco_bom_revisions (eco_id, tenant_id, bom_id, before_revision_id, after_revision_id)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(eco_id)
    .bind(tenant_id)
    .bind(req.bom_id)
    .bind(req.before_revision_id)
    .bind(req.after_revision_id)
    .fetch_one(pool)
    .await?;

    Ok(link)
}

pub async fn link_doc_revision(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
    req: &LinkDocRevisionRequest,
) -> Result<EcoDocRevision, BomError> {
    let eco = get_eco(pool, tenant_id, eco_id).await?;
    if eco.status == "applied" || eco.status == "rejected" {
        return Err(GuardError::Validation(
            "Cannot link doc revisions to a closed ECO".to_string(),
        )
        .into());
    }

    let link = sqlx::query_as::<_, EcoDocRevision>(
        r#"
        INSERT INTO eco_doc_revisions (eco_id, tenant_id, doc_id, doc_revision_id)
        VALUES ($1, $2, $3, $4)
        RETURNING *
        "#,
    )
    .bind(eco_id)
    .bind(tenant_id)
    .bind(req.doc_id)
    .bind(req.doc_revision_id)
    .fetch_one(pool)
    .await?;

    Ok(link)
}

// ============================================================================
// Apply ECO — drives BOM revision supersession
// ============================================================================

pub async fn apply_eco(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
    req: &ApplyEcoRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Eco, BomError> {
    let eco = get_eco(pool, tenant_id, eco_id).await?;
    if eco.status != "approved" {
        return Err(GuardError::Validation("Only approved ECOs can be applied".to_string()).into());
    }

    // Get linked BOM revisions
    let bom_links = list_bom_revision_links(pool, tenant_id, eco_id).await?;
    if bom_links.is_empty() {
        return Err(GuardError::Validation(
            "ECO must have at least one BOM revision link".to_string(),
        )
        .into());
    }

    let mut tx = pool.begin().await?;

    // For each linked BOM revision pair: supersede the old, release the new
    for link in &bom_links {
        // Supersede the before-revision
        sqlx::query(
            r#"
            UPDATE bom_revisions
            SET status = 'superseded', updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(link.before_revision_id)
        .bind(tenant_id)
        .execute(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            BomEventType::RevisionSuperseded,
            "bom_revision",
            &link.before_revision_id.to_string(),
            &events::build_revision_superseded_envelope(
                link.before_revision_id,
                link.bom_id,
                tenant_id.to_string(),
                eco_id,
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Release the after-revision with effectivity
        sqlx::query(
            r#"
            UPDATE bom_revisions
            SET status = 'effective',
                effective_from = $3,
                effective_to = $4,
                updated_at = NOW()
            WHERE id = $1 AND tenant_id = $2
            "#,
        )
        .bind(link.after_revision_id)
        .bind(tenant_id)
        .bind(req.effective_from)
        .bind(req.effective_to)
        .execute(&mut *tx)
        .await?;

        enqueue_event(
            &mut tx,
            tenant_id,
            BomEventType::RevisionReleased,
            "bom_revision",
            &link.after_revision_id.to_string(),
            &events::build_revision_released_envelope(
                link.after_revision_id,
                link.bom_id,
                tenant_id.to_string(),
                eco_id,
                req.effective_from,
                req.effective_to,
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;

        // Emit eco.applied event per BOM link
        enqueue_event(
            &mut tx,
            tenant_id,
            BomEventType::EcoApplied,
            "eco",
            &eco_id.to_string(),
            &events::build_eco_applied_envelope(
                eco_id,
                tenant_id.to_string(),
                eco.eco_number.clone(),
                link.bom_id,
                link.before_revision_id,
                link.after_revision_id,
                correlation_id.to_string(),
                causation_id.map(String::from),
            ),
            correlation_id,
            causation_id,
        )
        .await?;
    }

    // Mark ECO as applied
    let updated = sqlx::query_as::<_, Eco>(
        r#"
        UPDATE ecos
        SET status = 'applied', applied_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(eco_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await?;

    insert_audit(
        &mut tx,
        eco_id,
        tenant_id,
        "applied",
        &req.actor,
        Some(serde_json::json!({
            "effective_from": req.effective_from,
            "effective_to": req.effective_to,
            "bom_links": bom_links.len(),
        })),
    )
    .await?;

    tx.commit().await?;
    Ok(updated)
}

// ============================================================================
// Queries
// ============================================================================

pub async fn list_bom_revision_links(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
) -> Result<Vec<EcoBomRevision>, BomError> {
    let rows = sqlx::query_as::<_, EcoBomRevision>(
        "SELECT * FROM eco_bom_revisions WHERE eco_id = $1 AND tenant_id = $2 ORDER BY created_at",
    )
    .bind(eco_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_doc_revision_links(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
) -> Result<Vec<EcoDocRevision>, BomError> {
    let rows = sqlx::query_as::<_, EcoDocRevision>(
        "SELECT * FROM eco_doc_revisions WHERE eco_id = $1 AND tenant_id = $2 ORDER BY created_at",
    )
    .bind(eco_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn eco_history_for_part(
    pool: &PgPool,
    tenant_id: &str,
    part_id: Uuid,
) -> Result<Vec<Eco>, BomError> {
    let rows = sqlx::query_as::<_, Eco>(
        r#"
        SELECT DISTINCT e.*
        FROM ecos e
        JOIN eco_bom_revisions ebr ON ebr.eco_id = e.id AND ebr.tenant_id = e.tenant_id
        JOIN bom_headers h ON h.id = ebr.bom_id AND h.tenant_id = e.tenant_id
        WHERE e.tenant_id = $1 AND h.part_id = $2
        ORDER BY e.created_at
        "#,
    )
    .bind(tenant_id)
    .bind(part_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn get_audit_trail(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
) -> Result<Vec<EcoAuditEntry>, BomError> {
    let rows = sqlx::query_as::<_, EcoAuditEntry>(
        "SELECT * FROM eco_audit WHERE eco_id = $1 AND tenant_id = $2 ORDER BY created_at",
    )
    .bind(eco_id)
    .bind(tenant_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
