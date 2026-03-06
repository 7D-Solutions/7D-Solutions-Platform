use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::eco_models::*;
use crate::domain::guards::{guard_non_empty, GuardError};
use crate::domain::outbox::enqueue_event;
use crate::events::{self, BomEventType};

use super::bom_service::BomError;

// ============================================================================
// ECO lifecycle
// ============================================================================

pub async fn create_eco(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateEcoRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Eco, BomError> {
    guard_non_empty(&req.eco_number, "eco_number")?;
    guard_non_empty(&req.title, "title")?;
    guard_non_empty(&req.created_by, "created_by")?;

    let mut tx = pool.begin().await?;

    let eco = sqlx::query_as::<_, Eco>(
        r#"
        INSERT INTO ecos (tenant_id, eco_number, title, description, created_by)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(&req.eco_number)
    .bind(&req.title)
    .bind(&req.description)
    .bind(&req.created_by)
    .fetch_one(&mut *tx)
    .await?;

    // Audit
    insert_audit(&mut tx, eco.id, tenant_id, "created", &req.created_by, None).await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::EcoCreated,
        "eco",
        &eco.id.to_string(),
        &events::build_eco_created_envelope(
            eco.id,
            tenant_id.to_string(),
            req.eco_number.clone(),
            req.title.clone(),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;
    Ok(eco)
}

pub async fn get_eco(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
) -> Result<Eco, BomError> {
    sqlx::query_as::<_, Eco>(
        "SELECT * FROM ecos WHERE id = $1 AND tenant_id = $2",
    )
    .bind(eco_id)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| GuardError::NotFound("ECO not found".to_string()).into())
}

pub async fn submit_eco(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
    req: &EcoActionRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Eco, BomError> {
    let eco = get_eco(pool, tenant_id, eco_id).await?;
    if eco.status != "draft" {
        return Err(
            GuardError::Validation("Only draft ECOs can be submitted".to_string()).into(),
        );
    }

    let mut tx = pool.begin().await?;

    let updated = sqlx::query_as::<_, Eco>(
        r#"
        UPDATE ecos SET status = 'submitted', updated_at = NOW()
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
        "submitted",
        &req.actor,
        req.comment.as_deref().map(|c| serde_json::json!({ "comment": c })),
    )
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::EcoSubmitted,
        "eco",
        &eco_id.to_string(),
        &events::build_eco_status_changed_envelope(
            BomEventType::EcoSubmitted,
            eco_id,
            tenant_id.to_string(),
            eco.eco_number.clone(),
            "submitted".to_string(),
            req.actor.clone(),
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

pub async fn approve_eco(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
    req: &EcoActionRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Eco, BomError> {
    let eco = get_eco(pool, tenant_id, eco_id).await?;
    if eco.status != "submitted" {
        return Err(
            GuardError::Validation("Only submitted ECOs can be approved".to_string()).into(),
        );
    }

    let mut tx = pool.begin().await?;

    let updated = sqlx::query_as::<_, Eco>(
        r#"
        UPDATE ecos
        SET status = 'approved', approved_by = $3, approved_at = NOW(), updated_at = NOW()
        WHERE id = $1 AND tenant_id = $2
        RETURNING *
        "#,
    )
    .bind(eco_id)
    .bind(tenant_id)
    .bind(&req.actor)
    .fetch_one(&mut *tx)
    .await?;

    insert_audit(
        &mut tx,
        eco_id,
        tenant_id,
        "approved",
        &req.actor,
        req.comment.as_deref().map(|c| serde_json::json!({ "comment": c })),
    )
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::EcoApproved,
        "eco",
        &eco_id.to_string(),
        &events::build_eco_status_changed_envelope(
            BomEventType::EcoApproved,
            eco_id,
            tenant_id.to_string(),
            eco.eco_number.clone(),
            "approved".to_string(),
            req.actor.clone(),
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

pub async fn reject_eco(
    pool: &PgPool,
    tenant_id: &str,
    eco_id: Uuid,
    req: &EcoActionRequest,
    correlation_id: &str,
    causation_id: Option<&str>,
) -> Result<Eco, BomError> {
    let eco = get_eco(pool, tenant_id, eco_id).await?;
    if eco.status != "submitted" {
        return Err(
            GuardError::Validation("Only submitted ECOs can be rejected".to_string()).into(),
        );
    }

    let mut tx = pool.begin().await?;

    let updated = sqlx::query_as::<_, Eco>(
        r#"
        UPDATE ecos SET status = 'rejected', updated_at = NOW()
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
        "rejected",
        &req.actor,
        req.comment.as_deref().map(|c| serde_json::json!({ "comment": c })),
    )
    .await?;

    enqueue_event(
        &mut tx,
        tenant_id,
        BomEventType::EcoRejected,
        "eco",
        &eco_id.to_string(),
        &events::build_eco_status_changed_envelope(
            BomEventType::EcoRejected,
            eco_id,
            tenant_id.to_string(),
            eco.eco_number.clone(),
            "rejected".to_string(),
            req.actor.clone(),
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
    let before = super::bom_service::get_revision(pool, tenant_id, req.before_revision_id).await?;
    let after = super::bom_service::get_revision(pool, tenant_id, req.after_revision_id).await?;

    if before.bom_id != req.bom_id || after.bom_id != req.bom_id {
        return Err(
            GuardError::Validation("Revisions must belong to the specified BOM".to_string())
                .into(),
        );
    }

    if after.status != "draft" {
        return Err(
            GuardError::Validation(
                "After-revision must be in draft status".to_string(),
            )
            .into(),
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
        return Err(
            GuardError::Validation("Cannot link doc revisions to a closed ECO".to_string())
                .into(),
        );
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
        return Err(
            GuardError::Validation("Only approved ECOs can be applied".to_string()).into(),
        );
    }

    // Get linked BOM revisions
    let bom_links = list_bom_revision_links(pool, tenant_id, eco_id).await?;
    if bom_links.is_empty() {
        return Err(
            GuardError::Validation("ECO must have at least one BOM revision link".to_string())
                .into(),
        );
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

// ============================================================================
// Helpers
// ============================================================================

async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    eco_id: Uuid,
    tenant_id: &str,
    action: &str,
    actor: &str,
    detail: Option<serde_json::Value>,
) -> Result<(), BomError> {
    sqlx::query(
        r#"
        INSERT INTO eco_audit (eco_id, tenant_id, action, actor, detail)
        VALUES ($1, $2, $3, $4, $5)
        "#,
    )
    .bind(eco_id)
    .bind(tenant_id)
    .bind(action)
    .bind(actor)
    .bind(detail)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
