use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use event_bus::outbox::validate_and_serialize_envelope;
use platform_contracts::{event_naming::nats_subject, mutation_classes, EventEnvelope};
use uuid::Uuid;

use crate::handlers::{
    capitalize_actor_type, check_idempotency, extract_idem_key, is_unique_violation,
    store_idempotency, AppState,
};
use crate::models::*;

pub async fn create_distribution(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Path(doc_id): Path<Uuid>,
    Json(req): Json<CreateDistributionRequest>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;
    let actor_id = claims.user_id;

    if req.recipient_ref.trim().is_empty()
        || req.channel.trim().is_empty()
        || req.template_key.trim().is_empty()
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "recipient_ref, channel, and template_key are required"}),
            ),
        );
    }

    let idem_key = match extract_idem_key(&headers) {
        Some(k) if !k.trim().is_empty() => k,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "idempotency-key header is required"})),
            )
        }
    };

    if let Ok(Some(cached)) = check_idempotency(&state.db, &tenant_id.to_string(), &idem_key).await
    {
        return (
            StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
            Json(cached.response_body),
        );
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "begin transaction failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    let doc: Option<Document> = sqlx::query_as::<_, Document>(
        "SELECT id, tenant_id, doc_number, title, doc_type, status, superseded_by, created_by, created_at, updated_at
         FROM documents WHERE id = $1 AND tenant_id = $2 FOR UPDATE",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);

    let doc = match doc {
        Some(d) => d,
        None => {
            let _ = tx.rollback().await;
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "document not found"})),
            );
        }
    };

    if doc.status != "released" {
        let _ = tx.rollback().await;
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "document must be released before distribution"})),
        );
    }

    let revision_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM revisions WHERE document_id = $1 AND tenant_id = $2 ORDER BY revision_number DESC LIMIT 1",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);

    let distribution_id = Uuid::new_v4();
    let now = Utc::now();
    let payload_json = req
        .payload_json
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));

    let insert = sqlx::query(
        "INSERT INTO document_distributions
         (id, tenant_id, document_id, revision_id, recipient_ref, channel, template_key, payload_json,
          status, requested_by, requested_at, idempotency_key, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', $9, $10, $11, $10, $10)",
    )
    .bind(distribution_id)
    .bind(tenant_id)
    .bind(doc_id)
    .bind(revision_id)
    .bind(&req.recipient_ref)
    .bind(&req.channel)
    .bind(&req.template_key)
    .bind(&payload_json)
    .bind(actor_id)
    .bind(now)
    .bind(&idem_key)
    .execute(&mut *tx)
    .await;

    if let Err(e) = insert {
        let _ = tx.rollback().await;
        if is_unique_violation(&e) {
            if let Ok(Some(cached)) =
                check_idempotency(&state.db, &tenant_id.to_string(), &idem_key).await
            {
                return (
                    StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                    Json(cached.response_body),
                );
            }
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "duplicate distribution idempotency key"})),
            );
        }
        tracing::error!(error = %e, "insert distribution failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let log_key = format!("{}:requested", idem_key);
    if let Err(e) = sqlx::query(
        "INSERT INTO document_distribution_status_log
         (distribution_id, tenant_id, previous_status, new_status, idempotency_key, payload_json, changed_by, changed_at)
         VALUES ($1, $2, NULL, 'pending', $3, $4, $5, $6)",
    )
    .bind(distribution_id)
    .bind(tenant_id)
    .bind(log_key)
    .bind(serde_json::json!({"source": "doc_mgmt.distribute"}))
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "insert distribution status log failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.distribution.requested".to_string(),
        DocumentDistributionRequestedPayload {
            distribution_id,
            document_id: doc_id,
            revision_id,
            doc_number: doc.doc_number.clone(),
            recipient_ref: req.recipient_ref.clone(),
            channel: req.channel.clone(),
            template_key: req.template_key.clone(),
            payload_json: payload_json.clone(),
        },
    )
    .with_mutation_class(Some(mutation_classes::SIDE_EFFECT.to_string()))
    .with_actor(actor_id, capitalize_actor_type(claims.actor_type));

    let event_payload = match validate_and_serialize_envelope(&envelope) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.rollback().await;
            tracing::error!(error = %e, "envelope validation failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("document.distribution.requested")
            .bind(nats_subject("doc_mgmt", "document.distribution.requested"))
            .bind(event_payload)
            .execute(&mut *tx)
            .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "outbox insert failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let response_body = serde_json::json!({
        "distribution": {
            "id": distribution_id,
            "document_id": doc_id,
            "revision_id": revision_id,
            "recipient_ref": req.recipient_ref,
            "channel": req.channel,
            "template_key": req.template_key,
            "status": "pending",
            "requested_at": now,
            "idempotency_key": idem_key,
        }
    });

    let _ = store_idempotency(
        &mut tx,
        &tenant_id.to_string(),
        &idem_key,
        &response_body,
        201,
    )
    .await;

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    (StatusCode::CREATED, Json(response_body))
}

pub async fn list_distributions(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(doc_id): Path<Uuid>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;

    let rows = sqlx::query_as::<_, DocumentDistribution>(
        "SELECT id, tenant_id, document_id, revision_id, recipient_ref, channel, template_key,
                payload_json, status, provider_message_id, requested_by, requested_at, sent_at,
                delivered_at, failed_at, failure_reason, idempotency_key, created_at, updated_at
         FROM document_distributions
         WHERE tenant_id = $1 AND document_id = $2
         ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .bind(doc_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::json!({"distributions": rows})),
    )
}

pub async fn update_distribution_status(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Path(distribution_id): Path<Uuid>,
    Json(req): Json<DistributionStatusUpdateRequest>,
) -> impl IntoResponse {
    let claims = match claims {
        Some(axum::Extension(c)) => c,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "unauthorized"})),
            )
        }
    };

    let tenant_id = claims.tenant_id;
    let actor_id = claims.user_id;

    let new_status = req.status.trim().to_lowercase();
    if !matches!(
        new_status.as_str(),
        "sent" | "delivered" | "failed" | "ignored"
    ) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "invalid status"})),
        );
    }

    let idem_key = extract_idem_key(&headers).or_else(|| {
        req.notification_event_id
            .map(|id| format!("notif-event:{id}"))
    });

    let idem_key = match idem_key {
        Some(k) if !k.trim().is_empty() => k,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "idempotency-key header or notification_event_id is required"}),
                ),
            )
        }
    };

    if let Ok(Some(cached)) = check_idempotency(&state.db, &tenant_id.to_string(), &idem_key).await
    {
        return (
            StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
            Json(cached.response_body),
        );
    }

    let mut tx = match state.db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "begin transaction failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    let dist: Option<DocumentDistribution> = sqlx::query_as::<_, DocumentDistribution>(
        "SELECT id, tenant_id, document_id, revision_id, recipient_ref, channel, template_key,
                payload_json, status, provider_message_id, requested_by, requested_at, sent_at,
                delivered_at, failed_at, failure_reason, idempotency_key, created_at, updated_at
         FROM document_distributions
         WHERE id = $1 AND tenant_id = $2
         FOR UPDATE",
    )
    .bind(distribution_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);

    let dist = match dist {
        Some(d) => d,
        None => {
            let _ = tx.rollback().await;
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "distribution not found"})),
            );
        }
    };

    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT new_status FROM document_distribution_status_log WHERE tenant_id = $1 AND idempotency_key = $2",
    )
    .bind(tenant_id)
    .bind(&idem_key)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None)
    {
        let response_body = serde_json::json!({
            "distribution_id": distribution_id,
            "status": existing,
            "cached": true,
        });
        let _ = tx.rollback().await;
        return (StatusCode::OK, Json(response_body));
    }

    if dist.status == "delivered" || dist.status == "ignored" {
        let _ = tx.rollback().await;
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "distribution is final and cannot transition"})),
        );
    }

    let now = Utc::now();
    let delivered_at = req.delivered_at.unwrap_or(now);

    let (sent_at, delivered_at_col, failed_at, failure_reason) = match new_status.as_str() {
        "sent" => (
            Some(now),
            dist.delivered_at,
            dist.failed_at,
            dist.failure_reason.clone(),
        ),
        "delivered" => (dist.sent_at.or(Some(now)), Some(delivered_at), None, None),
        "failed" => (
            dist.sent_at,
            dist.delivered_at,
            Some(now),
            req.failure_reason.clone(),
        ),
        "ignored" => (
            dist.sent_at,
            dist.delivered_at,
            dist.failed_at,
            req.failure_reason.clone(),
        ),
        _ => (
            dist.sent_at,
            dist.delivered_at,
            dist.failed_at,
            dist.failure_reason.clone(),
        ),
    };

    if let Err(e) = sqlx::query(
        "UPDATE document_distributions
         SET status = $1,
             provider_message_id = COALESCE($2, provider_message_id),
             sent_at = $3,
             delivered_at = $4,
             failed_at = $5,
             failure_reason = $6,
             updated_at = $7
         WHERE id = $8 AND tenant_id = $9",
    )
    .bind(&new_status)
    .bind(&req.provider_message_id)
    .bind(sent_at)
    .bind(delivered_at_col)
    .bind(failed_at)
    .bind(&failure_reason)
    .bind(now)
    .bind(distribution_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "update distribution status failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    if let Err(e) = sqlx::query(
        "INSERT INTO document_distribution_status_log
         (distribution_id, tenant_id, previous_status, new_status, idempotency_key, notification_event_id,
          payload_json, changed_by, changed_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(distribution_id)
    .bind(tenant_id)
    .bind(&dist.status)
    .bind(&new_status)
    .bind(&idem_key)
    .bind(req.notification_event_id)
    .bind(serde_json::json!({
        "provider_message_id": req.provider_message_id,
        "failure_reason": req.failure_reason,
    }))
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        if is_unique_violation(&e) {
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "distribution_id": distribution_id,
                    "status": new_status,
                    "cached": true
                })),
            );
        }
        tracing::error!(error = %e, "insert status log failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.distribution.status.updated".to_string(),
        DocumentDistributionStatusUpdatedPayload {
            distribution_id,
            document_id: dist.document_id,
            status: new_status.clone(),
            provider_message_id: req.provider_message_id.clone(),
            failure_reason: req.failure_reason.clone(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
    .with_actor(actor_id, capitalize_actor_type(claims.actor_type));

    let event_payload = match validate_and_serialize_envelope(&envelope) {
        Ok(p) => p,
        Err(e) => {
            let _ = tx.rollback().await;
            tracing::error!(error = %e, "envelope validation failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("document.distribution.status.updated")
            .bind(nats_subject(
                "doc_mgmt",
                "document.distribution.status.updated",
            ))
            .bind(event_payload)
            .execute(&mut *tx)
            .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "outbox insert failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    let response_body = serde_json::json!({
        "distribution_id": distribution_id,
        "status": new_status,
        "provider_message_id": req.provider_message_id,
    });

    let _ = store_idempotency(
        &mut tx,
        &tenant_id.to_string(),
        &idem_key,
        &response_body,
        200,
    )
    .await;

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    (StatusCode::OK, Json(response_body))
}
