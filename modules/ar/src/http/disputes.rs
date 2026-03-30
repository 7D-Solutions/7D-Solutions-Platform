use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{ApiError, Dispute, ListDisputesQuery, SubmitDisputeEvidenceRequest};
use crate::tilled::dispute::{EvidenceFile, SubmitEvidenceRequest};
use crate::tilled::TilledClient;

/// GET /api/ar/disputes - List disputes with optional filters
pub async fn list_disputes(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListDisputesQuery>,
) -> Result<Json<Vec<Dispute>>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    let mut sql = String::from(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE app_id = $1
        "#,
    );

    let mut bind_index = 2;
    if query.charge_id.is_some() {
        sql.push_str(&format!(" AND charge_id = ${}", bind_index));
        bind_index += 1;
    }
    if query.status.is_some() {
        sql.push_str(&format!(" AND status = ${}", bind_index));
        bind_index += 1;
    }

    sql.push_str(&format!(
        " ORDER BY created_at DESC LIMIT ${} OFFSET ${}",
        bind_index,
        bind_index + 1
    ));

    let mut query_builder = sqlx::query_as::<_, Dispute>(&sql).bind(&app_id);

    if let Some(charge_id) = query.charge_id {
        query_builder = query_builder.bind(charge_id);
    }
    if let Some(status) = query.status {
        query_builder = query_builder.bind(status);
    }

    let disputes = query_builder
        .bind(limit)
        .bind(offset)
        .fetch_all(&db)
        .await
        .map_err(|e| {
            tracing::error!("Database error listing disputes: {:?}", e);
            ApiError::internal("Failed to list disputes")
        })?;

    Ok(Json(disputes))
}

/// GET /api/ar/disputes/{id} - Get a specific dispute
pub async fn get_dispute(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Dispute>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let dispute = sqlx::query_as::<_, Dispute>(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching dispute: {:?}", e);
        ApiError::internal("Failed to fetch dispute")
    })?
    .ok_or_else(|| ApiError::not_found(format!("Dispute {} not found", id)))?;

    Ok(Json(dispute))
}

/// POST /api/ar/disputes/{id}/evidence - Submit evidence for a dispute
pub async fn submit_dispute_evidence(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<SubmitDisputeEvidenceRequest>,
) -> Result<Json<Dispute>, ApiError> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let dispute = sqlx::query_as::<_, Dispute>(
        r#"
        SELECT
            id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
            status, amount_cents, currency, reason, reason_code,
            evidence_due_by, opened_at, closed_at, created_at, updated_at
        FROM ar_disputes
        WHERE id = $1 AND app_id = $2
        "#,
    )
    .bind(id)
    .bind(&app_id)
    .fetch_optional(&db)
    .await
    .map_err(|e| {
        tracing::error!("Database error fetching dispute: {:?}", e);
        ApiError::internal("Failed to fetch dispute")
    })?
    .ok_or_else(|| ApiError::not_found(format!("Dispute {} not found", id)))?;

    if let Some(evidence_due_by) = dispute.evidence_due_by {
        if chrono::Utc::now().naive_utc() > evidence_due_by {
            return Err(ApiError::bad_request(
                "Evidence submission deadline has passed",
            ));
        }
    }

    if dispute.status != "needs_response" && dispute.status != "open" {
        return Err(ApiError::bad_request(format!(
            "Cannot submit evidence for dispute with status '{}'",
            dispute.status
        )));
    }

    let client = TilledClient::from_env(&app_id).map_err(|e| {
        tracing::error!("Failed to create Tilled client: {:?}", e);
        ApiError::internal(format!(
            "Failed to initialize payment provider: {}",
            e
        ))
    })?;

    let description = req
        .evidence
        .get("description")
        .and_then(|v| v.as_str())
        .or_else(|| {
            req.evidence
                .get("evidence_description")
                .and_then(|v| v.as_str())
        })
        .or_else(|| req.evidence.get("evidence_text").and_then(|v| v.as_str()))
        .map(String::from);
    let files = req.evidence.get("files").and_then(|v| {
        v.as_array().map(|arr| {
            arr.iter()
                .filter_map(|entry| {
                    if let Some(file_id) = entry.as_str() {
                        Some(EvidenceFile {
                            file_id: file_id.to_string(),
                            evidence_type: "uncategorized".to_string(),
                        })
                    } else {
                        let file_id = entry.get("file_id").and_then(|v| v.as_str())?;
                        let evidence_type = entry
                            .get("type")
                            .and_then(|v| v.as_str())
                            .or_else(|| entry.get("evidence_type").and_then(|v| v.as_str()))
                            .unwrap_or("uncategorized");
                        Some(EvidenceFile {
                            file_id: file_id.to_string(),
                            evidence_type: evidence_type.to_string(),
                        })
                    }
                })
                .collect::<Vec<_>>()
        })
    });

    let tilled_req = SubmitEvidenceRequest { description, files };

    match client
        .submit_dispute_evidence(&dispute.tilled_dispute_id, tilled_req)
        .await
    {
        Ok(_tilled_dispute) => {
            let updated = sqlx::query_as::<_, Dispute>(
                r#"
                UPDATE ar_disputes
                SET status = 'under_review', updated_at = NOW()
                WHERE id = $1
                RETURNING
                    id, app_id, tilled_dispute_id, tilled_charge_id, charge_id,
                    status, amount_cents, currency, reason, reason_code,
                    evidence_due_by, opened_at, closed_at, created_at, updated_at
                "#,
            )
            .bind(id)
            .fetch_one(&db)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Failed to update dispute after evidence submission: {:?}",
                    e
                );
                ApiError::internal("Failed to update dispute")
            })?;

            tracing::info!(
                "Submitted evidence for dispute {} (Tilled ID: {})",
                id,
                dispute.tilled_dispute_id
            );

            Ok(Json(updated))
        }
        Err(e) => {
            tracing::error!(
                "Tilled evidence submission failed for dispute {}: {:?}",
                id,
                e
            );
            Err(ApiError::new(
                502,
                "provider_error",
                format!("Payment provider evidence submission failed: {}", e),
            ))
        }
    }
}
