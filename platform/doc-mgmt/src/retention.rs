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

use crate::handlers::{capitalize_actor_type, extract_idem_key, is_unique_violation, AppState};
use crate::models::*;

// ── Set Retention Policy (upsert) ────────────────────────────────────
//
// Guard: validate request
// Mutation: upsert retention_policies row
// Idempotent: ON CONFLICT UPDATE

pub async fn set_retention_policy(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Json(req): Json<SetRetentionPolicyRequest>,
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

    if req.doc_type.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "doc_type is required"})),
        );
    }
    if req.retention_days <= 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "retention_days must be positive"})),
        );
    }

    let now = Utc::now();
    let policy_id = Uuid::new_v4();

    let row = sqlx::query_as::<_, RetentionPolicy>(
        "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $6)
         ON CONFLICT (tenant_id, doc_type) DO UPDATE
           SET retention_days = EXCLUDED.retention_days, updated_at = EXCLUDED.updated_at
         RETURNING id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at",
    )
    .bind(policy_id)
    .bind(tenant_id)
    .bind(&req.doc_type)
    .bind(req.retention_days)
    .bind(actor_id)
    .bind(now)
    .fetch_one(&state.db)
    .await;

    match row {
        Ok(policy) => (StatusCode::OK, Json(serde_json::json!({"policy": policy}))),
        Err(e) => {
            tracing::error!(error = %e, "upsert retention policy failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            )
        }
    }
}

// ── Get Retention Policy ─────────────────────────────────────────────

pub async fn get_retention_policy(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(doc_type): Path<String>,
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

    let policy = sqlx::query_as::<_, RetentionPolicy>(
        "SELECT id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at
         FROM retention_policies WHERE tenant_id = $1 AND doc_type = $2",
    )
    .bind(tenant_id)
    .bind(&doc_type)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    match policy {
        Some(p) => (StatusCode::OK, Json(serde_json::json!({"policy": p}))),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "no retention policy for this doc_type"})),
        ),
    }
}

// ── Apply Legal Hold ─────────────────────────────────────────────────
//
// Guard: document must exist, belong to tenant
// Mutation: insert legal_holds row (idempotent — duplicate active reason is no-op)
// Outbox: enqueue legal_hold.applied event

pub async fn apply_hold(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Path(doc_id): Path<Uuid>,
    Json(req): Json<ApplyHoldRequest>,
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

    if req.reason.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "reason is required"})),
        );
    }

    // ── Idempotency check ────────────────────────────────────────────
    let idem_key = extract_idem_key(&headers);
    if let Some(ref key) = idem_key {
        if let Ok(Some(cached)) =
            crate::handlers::check_idempotency(&state.db, &tenant_id.to_string(), key).await
        {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Guard: document must exist and belong to tenant ──────────────
    let doc_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM documents WHERE id = $1 AND tenant_id = $2)",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    if !doc_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "document not found"})),
        );
    }

    // ── Check for existing active hold with same reason (idempotent) ─
    let existing: Option<LegalHold> = sqlx::query_as::<_, LegalHold>(
        "SELECT id, document_id, tenant_id, reason, held_by, held_at, released_by, released_at
         FROM legal_holds
         WHERE document_id = $1 AND tenant_id = $2 AND reason = $3 AND released_at IS NULL",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&req.reason)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    if let Some(hold) = existing {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"hold": hold, "already_active": true})),
        );
    }

    // ── Mutation + Outbox (atomic) ───────────────────────────────────
    let hold_id = Uuid::new_v4();
    let now = Utc::now();

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

    let insert_result = sqlx::query(
        "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(hold_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&req.reason)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await;

    if let Err(e) = insert_result {
        let _ = tx.rollback().await;
        if is_unique_violation(&e) {
            // Race condition: another transaction inserted the same hold
            return (
                StatusCode::OK,
                Json(serde_json::json!({"hold_id": hold_id, "already_active": true})),
            );
        }
        tracing::error!(error = %e, "insert legal hold failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // Outbox
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "legal_hold.applied".to_string(),
        LegalHoldAppliedPayload {
            document_id: doc_id,
            hold_id,
            reason: req.reason.clone(),
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

    let subject = nats_subject("doc_mgmt", "legal_hold.applied");

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("legal_hold.applied")
            .bind(&subject)
            .bind(&event_payload)
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
        "hold": {
            "id": hold_id,
            "document_id": doc_id,
            "tenant_id": tenant_id,
            "reason": req.reason,
            "held_by": actor_id,
            "held_at": now,
        }
    });

    if let Some(ref key) = idem_key {
        let _ = crate::handlers::store_idempotency(
            &mut tx,
            &tenant_id.to_string(),
            key,
            &response_body,
            201,
        )
        .await;
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    (StatusCode::CREATED, Json(response_body))
}

// ── Release Legal Hold ───────────────────────────────────────────────
//
// Guard: document + active hold must exist
// Mutation: set released_by + released_at on the hold row
// Outbox: enqueue legal_hold.released event

pub async fn release_hold(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Path(doc_id): Path<Uuid>,
    Json(req): Json<ReleaseHoldRequest>,
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

    if req.reason.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "reason is required"})),
        );
    }

    // ── Idempotency check ────────────────────────────────────────────
    let idem_key = extract_idem_key(&headers);
    if let Some(ref key) = idem_key {
        if let Ok(Some(cached)) =
            crate::handlers::check_idempotency(&state.db, &tenant_id.to_string(), key).await
        {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Guard: find active hold with this reason ─────────────────────
    let hold: Option<LegalHold> = sqlx::query_as::<_, LegalHold>(
        "SELECT id, document_id, tenant_id, reason, held_by, held_at, released_by, released_at
         FROM legal_holds
         WHERE document_id = $1 AND tenant_id = $2 AND reason = $3 AND released_at IS NULL",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&req.reason)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let hold = match hold {
        Some(h) => h,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "no active hold with this reason"})),
            )
        }
    };

    // ── Mutation + Outbox (atomic) ───────────────────────────────────
    let now = Utc::now();

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

    let result = sqlx::query(
        "UPDATE legal_holds SET released_by = $1, released_at = $2
         WHERE id = $3 AND released_at IS NULL",
    )
    .bind(actor_id)
    .bind(now)
    .bind(hold.id)
    .execute(&mut *tx)
    .await;

    match result {
        Err(e) => {
            let _ = tx.rollback().await;
            tracing::error!(error = %e, "release hold failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
        Ok(r) if r.rows_affected() == 0 => {
            let _ = tx.rollback().await;
            // Already released (concurrent)
            return (
                StatusCode::OK,
                Json(serde_json::json!({"hold_id": hold.id, "already_released": true})),
            );
        }
        Ok(_) => {}
    }

    // Outbox
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "legal_hold.released".to_string(),
        LegalHoldReleasedPayload {
            document_id: doc_id,
            hold_id: hold.id,
            reason: req.reason.clone(),
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

    let subject = nats_subject("doc_mgmt", "legal_hold.released");

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("legal_hold.released")
            .bind(&subject)
            .bind(&event_payload)
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
        "hold_id": hold.id,
        "released_by": actor_id,
        "released_at": now,
    });

    if let Some(ref key) = idem_key {
        let _ = crate::handlers::store_idempotency(
            &mut tx,
            &tenant_id.to_string(),
            key,
            &response_body,
            200,
        )
        .await;
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    (StatusCode::OK, Json(response_body))
}

// ── List Holds ───────────────────────────────────────────────────────

pub async fn list_holds(
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

    let holds: Vec<LegalHold> = sqlx::query_as::<_, LegalHold>(
        "SELECT id, document_id, tenant_id, reason, held_by, held_at, released_by, released_at
         FROM legal_holds
         WHERE document_id = $1 AND tenant_id = $2
         ORDER BY held_at DESC",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    (StatusCode::OK, Json(serde_json::json!({"holds": holds})))
}

// ── Dispose Document ─────────────────────────────────────────────────
//
// Guard: document must be released/superseded, no active holds, retention met
// Mutation: status → disposed
// Outbox: document.disposed event

pub async fn dispose_document(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
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
    let actor_id = claims.user_id;

    // ── Idempotency check ────────────────────────────────────────────
    let idem_key = extract_idem_key(&headers);
    if let Some(ref key) = idem_key {
        if let Ok(Some(cached)) =
            crate::handlers::check_idempotency(&state.db, &tenant_id.to_string(), key).await
        {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Guard: fetch document ────────────────────────────────────────
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

    // Must be released or superseded (not draft, not already disposed)
    if doc.status != "released" && doc.status != "superseded" {
        let _ = tx.rollback().await;
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("document must be in 'released' or 'superseded' state to dispose (current: {})", doc.status)
            })),
        );
    }

    // ── Guard: no active legal holds (app-level check + DB trigger backup) ──
    let active_holds: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds
         WHERE document_id = $1 AND tenant_id = $2 AND released_at IS NULL",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(0);

    if active_holds > 0 {
        let _ = tx.rollback().await;
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "cannot dispose document — active legal hold(s) exist",
                "active_holds": active_holds,
            })),
        );
    }

    // ── Guard: retention period must have elapsed ────────────────────
    let policy: Option<RetentionPolicy> = sqlx::query_as::<_, RetentionPolicy>(
        "SELECT id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at
         FROM retention_policies WHERE tenant_id = $1 AND doc_type = $2",
    )
    .bind(tenant_id)
    .bind(&doc.doc_type)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);

    if let Some(ref p) = policy {
        // Use updated_at as the "released/superseded at" timestamp
        let eligible_after = doc.updated_at + chrono::Duration::days(p.retention_days as i64);
        let now = Utc::now();
        if now < eligible_after {
            let _ = tx.rollback().await;
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "retention period has not elapsed",
                    "eligible_after": eligible_after,
                    "retention_days": p.retention_days,
                })),
            );
        }
    }
    // No policy → no retention requirement (disposal allowed if no holds)

    // ── Mutation: set status to disposed ──────────────────────────────
    let now = Utc::now();

    // The DB trigger trg_block_dispose_with_hold provides a safety net
    let result = sqlx::query(
        "UPDATE documents SET status = 'disposed', updated_at = $1
         WHERE id = $2 AND tenant_id = $3 AND status IN ('released', 'superseded')",
    )
    .bind(now)
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await;

    match result {
        Err(e) => {
            let _ = tx.rollback().await;
            let msg = e.to_string();
            if msg.contains("active legal hold") {
                return (
                    StatusCode::CONFLICT,
                    Json(
                        serde_json::json!({"error": "cannot dispose — active legal hold (DB enforced)"}),
                    ),
                );
            }
            tracing::error!(error = %e, "dispose document failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
        Ok(r) if r.rows_affected() == 0 => {
            let _ = tx.rollback().await;
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "document status changed concurrently"})),
            );
        }
        Ok(_) => {}
    }

    // ── Outbox ───────────────────────────────────────────────────────
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.disposed".to_string(),
        DocumentDisposedPayload {
            document_id: doc_id,
            doc_number: doc.doc_number.clone(),
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

    let subject = nats_subject("doc_mgmt", "document.disposed");

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("document.disposed")
            .bind(&subject)
            .bind(&event_payload)
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
        "document_id": doc_id,
        "status": "disposed",
        "disposed_at": now,
    });

    if let Some(ref key) = idem_key {
        let _ = crate::handlers::store_idempotency(
            &mut tx,
            &tenant_id.to_string(),
            key,
            &response_body,
            200,
        )
        .await;
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    (StatusCode::OK, Json(response_body))
}
