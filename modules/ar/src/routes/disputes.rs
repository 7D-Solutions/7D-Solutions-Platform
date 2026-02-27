use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use security::VerifiedClaims;
use sqlx::PgPool;

use crate::models::{Dispute, ErrorResponse, ListDisputesQuery, SubmitDisputeEvidenceRequest};
use crate::tilled::dispute::SubmitEvidenceRequest;
use crate::tilled::TilledClient;

/// GET /api/ar/disputes - List disputes with optional filters
pub async fn list_disputes(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Query(query): Query<ListDisputesQuery>,
) -> Result<Json<Vec<Dispute>>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    let limit = query.limit.unwrap_or(100).min(500);
    let offset = query.offset.unwrap_or(0);

    // Build dynamic query based on filters
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
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "database_error",
                    "Failed to list disputes",
                )),
            )
        })?;

    Ok(Json(disputes))
}

/// GET /api/ar/disputes/{id} - Get a specific dispute
pub async fn get_dispute(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
) -> Result<Json<Dispute>, (StatusCode, Json<ErrorResponse>)> {
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch dispute",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Dispute {} not found", id),
            )),
        )
    })?;

    Ok(Json(dispute))
}

/// POST /api/ar/disputes/{id}/evidence - Submit evidence for a dispute
pub async fn submit_dispute_evidence(
    State(db): State<PgPool>,
    claims: Option<Extension<VerifiedClaims>>,
    Path(id): Path<i32>,
    Json(req): Json<SubmitDisputeEvidenceRequest>,
) -> Result<Json<Dispute>, (StatusCode, Json<ErrorResponse>)> {
    let app_id = super::tenant::extract_tenant(&claims)?;

    // Verify dispute exists and belongs to app
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
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "database_error",
                "Failed to fetch dispute",
            )),
        )
    })?
    .ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new(
                "not_found",
                format!("Dispute {} not found", id),
            )),
        )
    })?;

    // Check if evidence is still acceptable (before due date)
    if let Some(evidence_due_by) = dispute.evidence_due_by {
        if chrono::Utc::now().naive_utc() > evidence_due_by {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(
                    "validation_error",
                    "Evidence submission deadline has passed",
                )),
            ));
        }
    }

    // Only open/pending disputes accept evidence
    if dispute.status != "needs_response" && dispute.status != "open" {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "validation_error",
                format!(
                    "Cannot submit evidence for dispute with status '{}'",
                    dispute.status
                ),
            )),
        ));
    }

    let client = TilledClient::from_env(&app_id).map_err(|e| {
        tracing::error!("Failed to create Tilled client: {:?}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new(
                "provider_config_error",
                format!("Failed to initialize payment provider: {}", e),
            )),
        )
    })?;

    // Extract evidence fields from the generic JSON payload
    let evidence_text = req.evidence.get("evidence_text").and_then(|v| v.as_str()).map(String::from);
    let evidence_file = req.evidence.get("evidence_file").and_then(|v| v.as_str()).map(String::from);
    let customer_communication = req.evidence.get("customer_communication").and_then(|v| v.as_str()).map(String::from);
    let uncategorized_text = req.evidence.get("uncategorized_text").and_then(|v| v.as_str()).map(String::from);

    let tilled_req = SubmitEvidenceRequest {
        evidence_text,
        evidence_file,
        customer_communication,
        uncategorized_text,
    };

    match client
        .submit_dispute_evidence(&dispute.tilled_dispute_id, tilled_req)
        .await
    {
        Ok(_tilled_dispute) => {
            // Record evidence submission timestamp in local dispute
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
                tracing::error!("Failed to update dispute after evidence submission: {:?}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse::new(
                        "database_error",
                        "Failed to update dispute",
                    )),
                )
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
            Err((
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse::new(
                    "provider_error",
                    format!("Payment provider evidence submission failed: {}", e),
                )),
            ))
        }
    }
}
