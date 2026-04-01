use platform_sdk::VerifiedClaims;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::eco_models::*;
use crate::domain::guards::{guard_non_empty, GuardError};
use crate::domain::numbering_client::NumberingClient;
use crate::domain::outbox::enqueue_event;
use crate::events::{self, BomEventType};

use crate::domain::bom_service::BomError;
use super::{get_eco, insert_audit};

pub async fn create_eco(
    pool: &PgPool,
    tenant_id: &str,
    req: &CreateEcoRequest,
    numbering: Option<&NumberingClient>,
    auth_header: Option<&str>,
    correlation_id: &str,
    causation_id: Option<&str>,
    claims: &VerifiedClaims,
) -> Result<Eco, BomError> {
    guard_non_empty(&req.title, "title")?;
    guard_non_empty(&req.created_by, "created_by")?;

    // Resolve eco_number: explicit value wins, otherwise auto-allocate.
    let eco_number = match &req.eco_number {
        Some(n) if !n.is_empty() => n.clone(),
        _ => {
            let nc = numbering.ok_or_else(|| {
                GuardError::Validation(
                    "eco_number required when numbering service is not configured".to_string(),
                )
            })?;
            nc.allocate_eco_number(tenant_id, correlation_id, auth_header, claims)
                .await?
        }
    };

    let mut tx = pool.begin().await?;

    let eco = sqlx::query_as::<_, Eco>(
        r#"
        INSERT INTO ecos (tenant_id, eco_number, title, description, created_by)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING *
        "#,
    )
    .bind(tenant_id)
    .bind(&eco_number)
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
            eco_number.clone(),
            req.title.clone(),
            correlation_id.to_string(),
            causation_id.map(String::from),
        ),
        correlation_id,
        causation_id,
    )
    .await?;

    tx.commit().await?;

    // Best-effort confirm after successful INSERT.
    if let Some(nc) = numbering {
        nc.confirm_eco_number(correlation_id, auth_header, claims).await;
    }

    Ok(eco)
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
        return Err(GuardError::Validation("Only draft ECOs can be submitted".to_string()).into());
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
        req.comment
            .as_deref()
            .map(|c| serde_json::json!({ "comment": c })),
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
        req.comment
            .as_deref()
            .map(|c| serde_json::json!({ "comment": c })),
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
        req.comment
            .as_deref()
            .map(|c| serde_json::json!({ "comment": c })),
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
