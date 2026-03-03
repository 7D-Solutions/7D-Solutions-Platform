use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use event_bus::outbox::validate_and_serialize_envelope;
use platform_contracts::{event_naming::nats_subject, mutation_classes, EventEnvelope};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::*;

pub struct AppState {
    pub db: PgPool,
}

pub fn extract_idem_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

// ── Create Document ──────────────────────────────────────────────────
//
// Guard: validate request + check tenant isolation
// Mutation: insert document + initial revision (atomic)
// Outbox: enqueue document.created event

pub async fn create_document(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Json(req): Json<CreateDocumentRequest>,
) -> impl IntoResponse {
    // ── Guard ────────────────────────────────────────────────────────
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

    if req.doc_number.is_empty() || req.title.is_empty() || req.doc_type.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "doc_number, title, and doc_type are required"})),
        );
    }

    // ── Idempotency check ────────────────────────────────────────────
    let idem_key = extract_idem_key(&headers);
    if let Some(ref key) = idem_key {
        if let Ok(Some(cached)) = check_idempotency(&state.db, &tenant_id.to_string(), key).await {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Mutation (atomic: doc + revision + outbox) ────────────────────
    let doc_id = Uuid::new_v4();
    let rev_id = Uuid::new_v4();
    let now = Utc::now();
    let body = req.body.clone().unwrap_or(serde_json::json!({}));

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.created".to_string(),
        DocumentCreatedPayload {
            document_id: doc_id,
            doc_number: req.doc_number.clone(),
            title: req.title.clone(),
            doc_type: req.doc_type.clone(),
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
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

    let subject = nats_subject("doc_mgmt", "document.created");

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

    // Insert document
    let insert_doc = sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, $7, $7)",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&req.doc_number)
    .bind(&req.title)
    .bind(&req.doc_type)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await;

    if let Err(e) = insert_doc {
        let _ = tx.rollback().await;
        if is_unique_violation(&e) {
            return (
                StatusCode::CONFLICT,
                Json(
                    serde_json::json!({"error": "document with this doc_number already exists for tenant"}),
                ),
            );
        }
        tracing::error!(error = %e, "insert document failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // Insert initial revision
    if let Err(e) = sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, created_by, created_at)
         VALUES ($1, $2, $3, 1, $4, 'Initial revision', $5, $6)",
    )
    .bind(rev_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(&body)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "insert revision failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // Outbox insert
    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("document.created")
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

    // Build response
    let response_body = serde_json::json!({
        "document": {
            "id": doc_id,
            "tenant_id": tenant_id,
            "doc_number": req.doc_number,
            "title": req.title,
            "doc_type": req.doc_type,
            "status": "draft",
            "created_by": actor_id,
            "created_at": now,
            "updated_at": now,
        },
        "latest_revision": {
            "id": rev_id,
            "document_id": doc_id,
            "tenant_id": tenant_id,
            "revision_number": 1,
            "body": body,
            "change_summary": "Initial revision",
            "created_by": actor_id,
            "created_at": now,
        }
    });

    // Store idempotency key
    if let Some(ref key) = idem_key {
        let _ = store_idempotency(&mut tx, &tenant_id.to_string(), key, &response_body, 201).await;
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

// ── Release Document ─────────────────────────────────────────────────
//
// Guard: document must exist, belong to tenant, be in draft state
// Mutation: update status to released
// Outbox: enqueue document.released event

pub async fn release_document(
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
        if let Ok(Some(cached)) = check_idempotency(&state.db, &tenant_id.to_string(), key).await {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Guard: fetch document, verify tenant and status ──────────────
    let doc: Option<Document> = sqlx::query_as::<_, Document>(
        "SELECT id, tenant_id, doc_number, title, doc_type, status, superseded_by, created_by, created_at, updated_at
         FROM documents WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let doc = match doc {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "document not found"})),
            )
        }
    };

    if doc.status != "draft" {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "document is not in draft state"})),
        );
    }

    // Get latest revision number for the event payload
    let rev_number: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(revision_number) FROM revisions WHERE document_id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(None);
    let rev_number = rev_number.unwrap_or(1);

    // ── Mutation + Outbox (atomic) ───────────────────────────────────
    let now = Utc::now();

    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.released".to_string(),
        DocumentReleasedPayload {
            document_id: doc_id,
            doc_number: doc.doc_number.clone(),
            revision_number: rev_number,
        },
    )
    .with_mutation_class(Some(mutation_classes::LIFECYCLE.to_string()))
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

    let subject = nats_subject("doc_mgmt", "document.released");

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
        "UPDATE documents SET status = 'released', updated_at = $1 WHERE id = $2 AND tenant_id = $3 AND status = 'draft'",
    )
    .bind(now)
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await;

    match result {
        Err(e) => {
            let _ = tx.rollback().await;
            tracing::error!(error = %e, "update document status failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "internal error"})),
            );
        }
        Ok(r) if r.rows_affected() == 0 => {
            let _ = tx.rollback().await;
            return (
                StatusCode::CONFLICT,
                Json(
                    serde_json::json!({"error": "document is not in draft state (concurrent modification)"}),
                ),
            );
        }
        Ok(_) => {}
    }

    // Mark all revisions for this document as released (DB-enforced immutability)
    if let Err(e) = sqlx::query(
        "UPDATE revisions SET status = 'released' WHERE document_id = $1 AND tenant_id = $2 AND status = 'draft'",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "mark revisions released failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("document.released")
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
        "status": "released",
        "revision_number": rev_number,
        "released_at": now,
    });

    if let Some(ref key) = idem_key {
        let _ = store_idempotency(&mut tx, &tenant_id.to_string(), key, &response_body, 200).await;
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

// ── Supersede Document ───────────────────────────────────────────────
//
// Guard: document must exist, belong to tenant, be in 'released' state
// Mutation: mark old doc as 'superseded', create new doc (draft) with linkage
// Outbox: enqueue document.superseded event

pub async fn supersede_document(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    headers: HeaderMap,
    Path(doc_id): Path<Uuid>,
    Json(req): Json<SupersedeRequest>,
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

    if req.new_doc_number.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "new_doc_number is required"})),
        );
    }

    // ── Idempotency check ────────────────────────────────────────────
    let idem_key = extract_idem_key(&headers);
    if let Some(ref key) = idem_key {
        if let Ok(Some(cached)) = check_idempotency(&state.db, &tenant_id.to_string(), key).await {
            return (
                StatusCode::from_u16(cached.status_code as u16).unwrap_or(StatusCode::OK),
                Json(cached.response_body),
            );
        }
    }

    // ── Guard: fetch document, verify tenant and released status ──────
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
            Json(
                serde_json::json!({"error": format!("document must be in 'released' state to supersede (current: {})", doc.status)}),
            ),
        );
    }

    if doc.superseded_by.is_some() {
        let _ = tx.rollback().await;
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "document has already been superseded"})),
        );
    }

    // ── Mutation: create new doc + initial revision, mark old as superseded ──
    let new_doc_id = Uuid::new_v4();
    let new_rev_id = Uuid::new_v4();
    let now = Utc::now();
    let new_title = req.new_title.unwrap_or_else(|| doc.title.clone());
    let change_summary = req
        .change_summary
        .unwrap_or_else(|| format!("Supersedes {}", doc.doc_number));

    // Copy body from latest released revision of old document
    let latest_body: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT body FROM revisions WHERE document_id = $1 AND tenant_id = $2
         ORDER BY revision_number DESC LIMIT 1",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&mut *tx)
    .await
    .unwrap_or(None);
    let body = latest_body.unwrap_or(serde_json::json!({}));

    // Insert new document (draft)
    if let Err(e) = sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, 'draft', $6, $7, $7)",
    )
    .bind(new_doc_id)
    .bind(tenant_id)
    .bind(&req.new_doc_number)
    .bind(&new_title)
    .bind(&doc.doc_type)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        if is_unique_violation(&e) {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "a document with this doc_number already exists for tenant"})),
            );
        }
        tracing::error!(error = %e, "insert new document failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // Insert initial revision for new document
    if let Err(e) = sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 1, $4, $5, 'draft', $6, $7)",
    )
    .bind(new_rev_id)
    .bind(new_doc_id)
    .bind(tenant_id)
    .bind(&body)
    .bind(&change_summary)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "insert new revision failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // Mark old document as superseded
    if let Err(e) = sqlx::query(
        "UPDATE documents SET status = 'superseded', superseded_by = $1, updated_at = $2
         WHERE id = $3 AND tenant_id = $4",
    )
    .bind(new_doc_id)
    .bind(now)
    .bind(doc_id)
    .bind(tenant_id)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "mark old document superseded failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // ── Outbox event ─────────────────────────────────────────────────
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "document.superseded".to_string(),
        DocumentSupersededPayload {
            old_document_id: doc_id,
            new_document_id: new_doc_id,
            new_doc_number: req.new_doc_number.clone(),
            old_doc_number: doc.doc_number.clone(),
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

    let subject = nats_subject("doc_mgmt", "document.superseded");

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("document.superseded")
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
        "old_document_id": doc_id,
        "old_doc_number": doc.doc_number,
        "old_status": "superseded",
        "new_document": {
            "id": new_doc_id,
            "doc_number": req.new_doc_number,
            "title": new_title,
            "status": "draft",
        },
        "new_revision": {
            "id": new_rev_id,
            "revision_number": 1,
        },
    });

    if let Some(ref key) = idem_key {
        let _ = store_idempotency(&mut tx, &tenant_id.to_string(), key, &response_body, 201).await;
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

// ── Get Document ─────────────────────────────────────────────────────

pub async fn get_document(
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

    let doc: Option<Document> = sqlx::query_as::<_, Document>(
        "SELECT id, tenant_id, doc_number, title, doc_type, status, superseded_by, created_by, created_at, updated_at
         FROM documents WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    let doc = match doc {
        Some(d) => d,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "document not found"})),
            )
        }
    };

    let latest_rev: Option<Revision> = sqlx::query_as::<_, Revision>(
        "SELECT id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at
         FROM revisions WHERE document_id = $1 AND tenant_id = $2
         ORDER BY revision_number DESC LIMIT 1",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "document": doc,
            "latest_revision": latest_rev,
        })),
    )
}

// ── List Documents ───────────────────────────────────────────────────

pub async fn list_documents(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
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

    let docs: Vec<Document> = sqlx::query_as::<_, Document>(
        "SELECT id, tenant_id, doc_number, title, doc_type, status, superseded_by, created_by, created_at, updated_at
         FROM documents WHERE tenant_id = $1
         ORDER BY created_at DESC",
    )
    .bind(tenant_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    (StatusCode::OK, Json(serde_json::json!({"documents": docs})))
}

// ── Create Revision ──────────────────────────────────────────────────

pub async fn create_revision(
    State(state): State<std::sync::Arc<AppState>>,
    claims: Option<axum::Extension<security::VerifiedClaims>>,
    Path(doc_id): Path<Uuid>,
    Json(req): Json<CreateRevisionRequest>,
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

    // Guard: document must be in draft and belong to tenant
    let doc: Option<Document> = sqlx::query_as::<_, Document>(
        "SELECT id, tenant_id, doc_number, title, doc_type, status, superseded_by, created_by, created_at, updated_at
         FROM documents WHERE id = $1 AND tenant_id = $2",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .fetch_optional(&state.db)
    .await
    .unwrap_or(None);

    match doc {
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "document not found"})),
            )
        }
        Some(ref d) if d.status != "draft" => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({"error": "cannot add revision to a released document"})),
            )
        }
        _ => {}
    }

    // Mutation + outbox (atomic)
    let rev_id = Uuid::new_v4();
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

    // Get next revision number
    let max_rev: Option<i32> =
        sqlx::query_scalar("SELECT MAX(revision_number) FROM revisions WHERE document_id = $1")
            .bind(doc_id)
            .fetch_one(&mut *tx)
            .await
            .unwrap_or(None);
    let next_rev = max_rev.unwrap_or(0) + 1;

    if let Err(e) = sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, created_by, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(rev_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(next_rev)
    .bind(&req.body)
    .bind(&req.change_summary)
    .bind(actor_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    {
        let _ = tx.rollback().await;
        tracing::error!(error = %e, "insert revision failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    // Update document timestamp
    let _ = sqlx::query("UPDATE documents SET updated_at = $1 WHERE id = $2")
        .bind(now)
        .bind(doc_id)
        .execute(&mut *tx)
        .await;

    // Outbox
    let envelope = EventEnvelope::new(
        tenant_id.to_string(),
        "doc_mgmt".to_string(),
        "revision.created".to_string(),
        RevisionCreatedPayload {
            document_id: doc_id,
            revision_id: rev_id,
            revision_number: next_rev,
        },
    )
    .with_mutation_class(Some(mutation_classes::DATA_MUTATION.to_string()))
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

    let event_subject = nats_subject("doc_mgmt", "revision.created");

    if let Err(e) =
        sqlx::query("INSERT INTO doc_outbox (event_type, subject, payload) VALUES ($1, $2, $3)")
            .bind("revision.created")
            .bind(&event_subject)
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

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "commit failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "internal error"})),
        );
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "revision": {
                "id": rev_id,
                "document_id": doc_id,
                "tenant_id": tenant_id,
                "revision_number": next_rev,
                "body": req.body,
                "change_summary": req.change_summary,
                "created_by": actor_id,
                "created_at": now,
            }
        })),
    )
}

// ── Idempotency helpers ──────────────────────────────────────────────

#[derive(sqlx::FromRow)]
pub struct CachedResponse {
    pub response_body: serde_json::Value,
    pub status_code: i32,
}

pub async fn check_idempotency(
    pool: &PgPool,
    app_id: &str,
    key: &str,
) -> Result<Option<CachedResponse>, sqlx::Error> {
    sqlx::query_as::<_, CachedResponse>(
        "SELECT response_body, status_code FROM doc_idempotency_keys
         WHERE app_id = $1 AND idempotency_key = $2 AND expires_at > now()",
    )
    .bind(app_id)
    .bind(key)
    .fetch_optional(pool)
    .await
}

pub async fn store_idempotency(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    app_id: &str,
    key: &str,
    response: &serde_json::Value,
    status_code: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO doc_idempotency_keys (app_id, idempotency_key, response_body, status_code, expires_at)
         VALUES ($1, $2, $3, $4, now() + interval '24 hours')
         ON CONFLICT (app_id, idempotency_key) DO NOTHING",
    )
    .bind(app_id)
    .bind(key)
    .bind(response)
    .bind(status_code)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// EventEnvelope validation expects "User"/"Service"/"System" (capitalized).
pub fn capitalize_actor_type(at: security::claims::ActorType) -> String {
    match at {
        security::claims::ActorType::User => "User".to_string(),
        security::claims::ActorType::Service => "Service".to_string(),
        security::claims::ActorType::System => "System".to_string(),
    }
}

pub fn is_unique_violation(e: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(ref db_err) = e {
        return db_err.code().as_deref() == Some("23505");
    }
    false
}
