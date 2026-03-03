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

use crate::handlers::{capitalize_actor_type, check_idempotency, extract_idem_key, AppState};
use crate::models::*;
use crate::render::{apply_template, compute_hash};

// ── Create Template ─────────────────────────────────────────────────
//
// Guard: validate request + tenant isolation
// Mutation: insert template row (atomic with outbox)
// Outbox: enqueue template.created event

pub async fn create_template(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateTemplateRequest>,
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

    // ── Guard ────────────────────────────────────────────────────────
    if req.name.is_empty() || req.doc_type.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "name and doc_type are required"})),
        );
    }

    // ── Idempotency check ────────────────────────────────────────────
    let idem_key = extract_idem_key(&headers);
    if let Some(ref key) = idem_key {
        if let Ok(Some(cached)) =
            check_idempotency(&state.db, &tenant_id.to_string(), key).await
        {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Determine next version for this template name ────────────────
    let max_version: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(version) FROM doc_templates WHERE tenant_id = $1 AND name = $2",
    )
    .bind(tenant_id)
    .bind(&req.name)
    .fetch_one(&state.db)
    .await
    .unwrap_or(None);
    let next_version = max_version.unwrap_or(0) + 1;

    // ── Mutation (atomic: template + outbox) ─────────────────────────
    let template_id = Uuid::new_v4();
    let now = Utc::now();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "template.created".to_string(),
        TemplateCreatedPayload {
            template_id,
            name: req.name.clone(),
            doc_type: req.doc_type.clone(),
            version: next_version,
        },
    )
    .with_mutation_class(Some(mutation_classes::DATA_MUTATION.to_string()))
    .with_actor(actor_id, capitalize_actor_type(claims.actor_type));

    let event_payload = match validate_and_serialize_envelope(&envelope) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "envelope validation failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    let subject = nats_subject("doc_mgmt", "template.created");

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
        "INSERT INTO doc_templates (id, tenant_id, name, doc_type, body_template, version, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
    )
    .bind(template_id)
    .bind(tenant_id)
    .bind(&req.name)
    .bind(&req.doc_type)
    .bind(&req.body_template)
    .bind(next_version)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await;

    if let Err(e) = insert_result {
        let _ = tx.rollback().await;
        if crate::handlers::is_unique_violation(&e) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "template version already exists"})),
            );
        }
        tracing::error!(error = %e, "insert template failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    if let Err(e) = sqlx::query(
        "INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)",
    )
    .bind("template.created")
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
        "template": {
            "id": template_id,
            "tenant_id": tenant_id,
            "name": req.name,
            "doc_type": req.doc_type,
            "body_template": req.body_template,
            "version": next_version,
            "created_by": actor_id,
            "created_at": now,
            "updated_at": now,
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

// ── Get Template ────────────────────────────────────────────────────

pub async fn get_template(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(template_id): Path<Uuid>,
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

    let template = sqlx::query_as::<_, DocTemplate>(
        "SELECT id, tenant_id, name, doc_type, body_template, version, created_by, created_at, updated_at
         FROM doc_templates WHERE id = $1 AND tenant_id = $2",
    )
    .bind(template_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    match template {
        Some(t) => (StatusCode::OK, Json(serde_json::json!({"template": t}))),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "template not found"})),
        ),
    }
}

// ── Render Template ─────────────────────────────────────────────────
//
// Guard: template must exist + belong to tenant
// Mutation: render output, compute hashes, insert artifact (atomic with outbox)
// Outbox: enqueue document.rendered event
// Idempotency: same idempotency_key returns cached artifact

pub async fn render_template(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Path(template_id): Path<Uuid>,
    Json(req): Json<RenderRequest>,
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
            check_idempotency(&state.db, &tenant_id.to_string(), key).await
        {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Guard: fetch template ────────────────────────────────────────
    let template = sqlx::query_as::<_, DocTemplate>(
        "SELECT id, tenant_id, name, doc_type, body_template, version, created_by, created_at, updated_at
         FROM doc_templates WHERE id = $1 AND tenant_id = $2",
    )
    .bind(template_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let template = match template {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "template not found"})),
            )
        }
    };

    // ── Render: apply input_data to body_template ────────────────────
    let output = apply_template(&template.body_template, &req.input_data);
    let input_hash = compute_hash(&req.input_data);
    let output_hash = compute_hash(&output);

    // ── Check for duplicate render via idempotency_key ────────────────
    if let Some(ref key) = idem_key {
        let existing: Option<RenderArtifact> = sqlx::query_as::<_, RenderArtifact>(
            "SELECT id, tenant_id, template_id, idempotency_key, input_hash, output_hash, output, rendered_by, rendered_at
             FROM render_artifacts WHERE tenant_id = $1 AND idempotency_key = $2",
        )
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(&state.db)
        .await
        .unwrap_or(None);

        if let Some(artifact) = existing {
            return (
                StatusCode::OK,
                Json(serde_json::json!({"artifact": artifact, "cached": true})),
            );
        }
    }

    // ── Mutation + Outbox (atomic) ───────────────────────────────────
    let artifact_id = Uuid::new_v4();
    let now = Utc::now();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.rendered".to_string(),
        DocumentRenderedPayload {
            artifact_id,
            template_id,
            output_hash: output_hash.clone(),
        },
    )
    .with_mutation_class(Some(mutation_classes::DATA_MUTATION.to_string()))
    .with_actor(actor_id, capitalize_actor_type(claims.actor_type));

    let event_payload = match validate_and_serialize_envelope(&envelope) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "envelope validation failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
    };

    let subject = nats_subject("doc_mgmt", "document.rendered");

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
        "INSERT INTO render_artifacts (id, tenant_id, template_id, idempotency_key, input_hash, output_hash, output, rendered_by, rendered_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .bind(template_id)
    .bind(idem_key.as_deref())
    .bind(&input_hash)
    .bind(&output_hash)
    .bind(&output)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await;

    if let Err(e) = insert_result {
        let _ = tx.rollback().await;
        if crate::handlers::is_unique_violation(&e) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "duplicate idempotency key"})),
            );
        }
        tracing::error!(error = %e, "insert artifact failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    if let Err(e) = sqlx::query(
        "INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)",
    )
    .bind("document.rendered")
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
        "artifact": {
            "id": artifact_id,
            "tenant_id": tenant_id,
            "template_id": template_id,
            "idempotency_key": idem_key,
            "input_hash": input_hash,
            "output_hash": output_hash,
            "output": output,
            "rendered_by": actor_id,
            "rendered_at": now,
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

// ── Get Artifact ────────────────────────────────────────────────────

pub async fn get_artifact(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(artifact_id): Path<Uuid>,
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

    let artifact = sqlx::query_as::<_, RenderArtifact>(
        "SELECT id, tenant_id, template_id, idempotency_key, input_hash, output_hash, output, rendered_by, rendered_at
         FROM render_artifacts WHERE id = $1 AND tenant_id = $2",
    )
    .bind(artifact_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    match artifact {
        Some(a) => (StatusCode::OK, Json(serde_json::json!({"artifact": a}))),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "artifact not found"})),
        ),
    }
}
